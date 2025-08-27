use std::{collections::VecDeque, time::Duration};

use anyhow::Result;
use tokio::{
    sync::{mpsc, oneshot},
    time::interval,
};
use tracing::{error, info, warn};

use super::order_book::{OrderBook, ProcessResult};
use crate::{
    data::binance::models::{DepthSnapshot, DepthUpdate},
    streaming::{DatabaseWriter, MessageProducer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotReason {
    Initial,
    BufferedUpdates,
}

impl SnapshotReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::BufferedUpdates => "buffered_updates",
        }
    }
}

#[derive(Debug)]
pub struct SnapshotRequest {
    pub symbol: String,
    pub limit: Option<i32>,
    pub response_tx: oneshot::Sender<Result<DepthSnapshot>>,
    pub reason: SnapshotReason,
}

impl SnapshotRequest {
    #[must_use]
    pub fn new(
        symbol: String,
        limit: Option<i32>,
        reason: SnapshotReason,
    ) -> (Self, oneshot::Receiver<Result<DepthSnapshot>>) {
        let (response_tx, response_rx) = oneshot::channel();
        (
            Self {
                symbol,
                limit,
                response_tx,
                reason,
            },
            response_rx,
        )
    }
}

pub type SnapshotRequestSender = mpsc::UnboundedSender<SnapshotRequest>;

pub struct OrderBookProcessor {
    symbol: String,
    limit: Option<i32>,
    state: OrderBook,

    // Input channels
    book_diff_update: mpsc::UnboundedReceiver<DepthUpdate>,
    snapshot_request_tx: SnapshotRequestSender,

    // Output producers
    signals_producer: Option<MessageProducer>,
    state_producer: Option<MessageProducer>,
    database_writer: Option<DatabaseWriter>,

    // State tracking
    updates_since_last_signals: u64,
    updates_since_last_state: u64,
    last_signals_id: u64,
    last_state_publish: std::time::Instant,

    publish_interval: Option<Duration>,
}

impl OrderBookProcessor {
    pub async fn new(
        symbol: String,
        limit: Option<i32>,
        mut book_diff_update: mpsc::UnboundedReceiver<DepthUpdate>,
        snapshot_request_tx: SnapshotRequestSender,
        signals_producer: Option<MessageProducer>,
        state_producer: Option<MessageProducer>,
        database_writer: Option<DatabaseWriter>,
        publish_interval: Option<Duration>,
    ) -> Result<Self> {
        info!(symbol = %symbol, "Initializing order book");

        // Step 1: Start buffering updates (this is already happening via the receiver)

        // Step 2: Get initial snapshot
        let depth_snapshot = Self::request_snapshot(
            &symbol,
            limit,
            &snapshot_request_tx,
            SnapshotReason::Initial,
        )
        .await?;
        let mut state = OrderBook::from_snapshot(depth_snapshot);

        // Step 3: Process buffered updates
        let mut buffer = Vec::new();

        // Collect all buffered updates (non-blocking)
        book_diff_update.recv_many(&mut buffer, usize::MAX).await;
        let buffer: VecDeque<_> = buffer.into_iter().collect();

        // while let Ok(update) = book_diff_update.try_recv() {
        //     buffer.push_back(update);
        // }

        if !buffer.is_empty() {
            info!(symbol = %symbol, buffer_size = buffer.len(), "Processing buffered updates");

            // Process buffer with initialization rules
            match state.process_update_buffer(buffer) {
                ProcessResult::Updated => {
                    info!(symbol = %symbol, "Successfully processed buffered updates");
                }
                ProcessResult::NeedsSnapshot => {
                    info!(symbol = %symbol, "Buffer processing requires new snapshot");
                    let new_snapshot = Self::request_snapshot(
                        &symbol,
                        limit,
                        &snapshot_request_tx,
                        SnapshotReason::BufferedUpdates,
                    )
                    .await?;
                    state.apply_snapshot(new_snapshot);
                }
                ProcessResult::Stale => {
                    info!(symbol = %symbol, "No relevant updates in buffer");
                }
            }
        }

        Ok(Self {
            symbol,
            limit,
            state,
            book_diff_update,
            snapshot_request_tx,
            signals_producer,
            state_producer,
            database_writer,
            updates_since_last_signals: 0,
            updates_since_last_state: 0,
            last_signals_id: 0,
            last_state_publish: std::time::Instant::now(),
            publish_interval,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        info!(symbol = %self.symbol, "Starting  order book");

        let timer_duration = self.publish_interval.unwrap_or(Duration::MAX); // effectively disables interval
        let mut publish_timer = interval(timer_duration);

        loop {
            tokio::select! {
                update = self.book_diff_update.recv() => {
                    if let Some(update) = update {
                        match self.state.process_update(&update)? {
                            ProcessResult::Updated => {
                                info!(symbol = %self.symbol, update_id = update.final_update_id, "Processed update");
                                // Publish signals and state
                                if let Some(ref signals_producer) = self.signals_producer {
                                    let summary = self.state.market_data_summary();
                                    if let Err(e) = signals_producer.send_json(&summary).await {
                                        error!("Failed to publish orderbook signals: {}", e);
                                    }
                                    if let Some(ref db_writer) = self.database_writer
                                        && let Err(e) = db_writer
                                            .write_orderbook_summary(&summary, &self.symbol)
                                            .await
                                    {
                                        error!("Failed to write orderbook summary to database: {}", e);
                                    }
                                }
                                // Persist to database if configured
                                if let Some(ref db_writer) = self.database_writer
                                    && let Err(e) = db_writer.write_depth_update(&update).await
                                {
                                    error!("Failed to write depth update to database: {}", e);
                                }

                                // Publish state snapshot if needed
                                if let Some(ref state_producer) = self.state_producer && self.publish_interval.is_none() {
                                    let state = self.state.state_snapshot(self.limit);
                                    if let Err(e) = state_producer.send_json(&state).await {
                                        error!("Failed to publish orderbook state: {}", e);
                                    }

                                    // Also persist state snapshot to database for depth chart visualization
                                    if let Some(ref db_writer) = self.database_writer
                                        && let Err(e) =
                                            db_writer.write_orderbook_state(&state, &self.symbol).await
                                    {
                                        error!("Failed to write orderbook state to database: {}", e);
                                    }
                                }

                            }
                            ProcessResult::NeedsSnapshot => {
                                info!("Requesting new snapshot for {}", self.symbol);
                                self.handle_snapshot_request().await?;
                            }
                            ProcessResult::Stale => (),
                        }
                    } else {
                        info!("Book diff update channel closed for {}", self.symbol);
                        break;
                    }

                }
                _ = publish_timer.tick(), if self.publish_interval.is_some() => {
                    // Publish state snapshot at intervals
                    if let Some(ref state_producer) = self.state_producer {
                        let state = self.state.state_snapshot(self.limit);
                        if let Err(e) = state_producer.send_json(&state).await {
                            error!("Failed to publish orderbook state: {}", e);
                        }

                        // Also persist state snapshot to database for depth chart visualization
                        if let Some(ref db_writer) = self.database_writer
                            && let Err(e) =
                                db_writer.write_orderbook_state(&state, &self.symbol).await
                        {
                            error!("Failed to write orderbook state to database: {}", e);
                        }
                    }
                }

            }
        }
        info!(symbol = %self.symbol, "OrderBookProcessor completed");
        Ok(())
    }

    // fn should_publish_state(&self) -> bool {
    //     // Publish state less frequently or based on significant changes
    //     self.updates_since_last_state > 10
    //         || self.significant_book_change()
    //         || self.last_state_publish.elapsed() > std::time::Duration::from_millis(500)
    // }

    // fn significant_book_change(&self) -> bool {
    //     // TODO: Implement logic to detect significant changes
    //     // For now, just use simple threshold
    //     self.updates_since_last_state > 5
    // }

    async fn handle_snapshot_request(&mut self) -> Result<()> {
        match self.get_snapshot(SnapshotReason::BufferedUpdates).await {
            Ok(new_snapshot) => {
                info!(
                    symbol = %self.symbol,
                    new_update_id = new_snapshot.last_update_id,
                    "Applying new snapshot"
                );

                // Persist snapshot to database if configured
                if let Some(ref db_writer) = self.database_writer
                    && let Err(e) = db_writer
                        .write_depth_snapshot(
                            &new_snapshot,
                            &self.symbol,
                            SnapshotReason::BufferedUpdates.as_str(),
                        )
                        .await
                {
                    error!("Failed to write snapshot to database: {}", e);
                }

                self.state.apply_snapshot(new_snapshot);

                // Process any buffered updates
                self.process_buffered_updates_after_snapshot().await?;
            }
            Err(e) => {
                error!("Failed to get snapshot: {}", e);
            }
        }
        Ok(())
    }

    async fn process_buffered_updates_after_snapshot(&mut self) -> Result<()> {
        let mut buffer = Vec::new();
        while let Ok(buffered_update) = self.book_diff_update.try_recv() {
            buffer.push(buffered_update);
        }

        if !buffer.is_empty() {
            info!(
                symbol = %self.symbol,
                buffer_size = buffer.len(),
                "Processing {} buffered updates after snapshot",
                buffer.len()
            );

            let buffer_deque: VecDeque<_> = buffer.into_iter().collect();
            match self.state.process_update_buffer(buffer_deque) {
                ProcessResult::Updated => {
                    info!(symbol = %self.symbol, "Buffered updates processed successfully after snapshot");
                }
                ProcessResult::NeedsSnapshot => {
                    warn!(symbol = %self.symbol, "Buffered updates after snapshot require another snapshot");
                    let new_snapshot = Self::request_snapshot(
                        &self.symbol,
                        self.limit,
                        &self.snapshot_request_tx,
                        SnapshotReason::BufferedUpdates,
                    )
                    .await?;
                    self.state.apply_snapshot(new_snapshot);
                }
                ProcessResult::Stale => {
                    info!(symbol = %self.symbol, "No relevant buffered updates after snapshot");
                }
            }
        }
        Ok(())
    }

    async fn get_snapshot(&self, reason: SnapshotReason) -> Result<DepthSnapshot> {
        Self::request_snapshot(&self.symbol, self.limit, &self.snapshot_request_tx, reason).await
    }

    async fn request_snapshot(
        symbol: &str,
        limit: Option<i32>,
        sender: &SnapshotRequestSender,
        reason: SnapshotReason,
    ) -> Result<DepthSnapshot> {
        let (request, response_rx) = SnapshotRequest::new(symbol.to_string(), limit, reason);
        sender
            .send(request)
            .map_err(|e| anyhow::anyhow!("Failed to send snapshot request: {}", e))?;
        response_rx.await?
    }
}

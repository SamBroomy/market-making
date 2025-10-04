use std::{collections::VecDeque, time::Duration};

use anyhow::Result;
use domain::models::{
    market_data::{DepthSnapshot, DepthUpdate},
    order_book::{OrderBook, ProcessResult, SnapshotReason},
};
use infrastructure::{messaging::StreamProducer, persist::DataWriter};
use tokio::{
    sync::{mpsc, oneshot},
    time::interval,
};
use tracing::{debug, error, info, warn};

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
    signals_producer: Option<StreamProducer>,
    state_producer: Option<StreamProducer>,
    database_writer: Option<DataWriter>,

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
        signals_producer: Option<StreamProducer>,
        state_producer: Option<StreamProducer>,
        database_writer: Option<DataWriter>,
        publish_interval: Option<Duration>,
    ) -> Result<Self> {
        info!(
            symbol = %symbol,
            limit = ?limit,
            "Initializing order book processor"
        );

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

        if !buffer.is_empty() {
            info!(
                symbol = %symbol,
                buffer_size = buffer.len(),
                "Processing buffered updates"
            );

            // Process buffer with initialization rules
            match state.process_update_buffer(buffer) {
                ProcessResult::Updated(_) => {
                    info!(
                        symbol = %symbol,
                        "Successfully processed buffered updates"
                    );
                }
                ProcessResult::NeedsSnapshot(reason) => {
                    info!(
                        symbol = %symbol,
                        reason = %reason.as_str(),
                        "Buffer processing requires new snapshot"
                    );
                    let new_snapshot =
                        Self::request_snapshot(&symbol, limit, &snapshot_request_tx, reason)
                            .await?;
                    state.apply_snapshot(new_snapshot);
                }
                ProcessResult::Stale => {
                    info!(
                        symbol = %symbol,
                        "No relevant updates in buffer"
                    );
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
        info!(
            symbol = %self.symbol,
            "Starting order book processor"
        );

        let timer_duration = self.publish_interval.unwrap_or(Duration::MAX);
        let mut publish_timer = interval(timer_duration);

        loop {
            tokio::select! {
                update = self.book_diff_update.recv() => {
                    if let Some(update) = update {
                        match self.state.process_update(&update)? {
                            ProcessResult::Updated(summary) => {
                                debug!(
                                    symbol = %self.symbol,
                                    update_id = update.final_update_id,
                                    "Processed update"
                                );
                                // Publish signals and state
                                if let Some(ref signals_producer) = self.signals_producer {
                                    if let Err(e) = signals_producer.send(&summary).await {
                                        error!(
                                            symbol = %self.symbol,
                                            error = %e,
                                            "Failed to publish orderbook signals"
                                        );
                                    }
                                    if let Some(ref db_writer) = self.database_writer
                                        && let Err(e) = db_writer
                                            .write_orderbook_summary(&summary, &self.symbol)
                                            .await
                                    {
                                        error!(
                                            symbol = %self.symbol,
                                            error = %e,
                                            "Failed to write orderbook summary to database"
                                        );
                                    }
                                }
                                // Persist to database if configured
                                if let Some(ref db_writer) = self.database_writer
                                    && let Err(e) = db_writer.write_depth_update(&update).await
                                {
                                    error!(
                                        symbol = %self.symbol,
                                        error = %e,
                                        "Failed to write depth update to database"
                                    );
                                }
                            }
                            ProcessResult::NeedsSnapshot(reason) => {
                                info!(
                                    symbol = %self.symbol,
                                    reason = %reason.as_str(),
                                    "Requesting new snapshot"
                                );
                                self.handle_snapshot_request(reason).await?;
                            }
                            ProcessResult::Stale => (),
                        }
                    } else {
                        info!(
                            symbol = %self.symbol,
                            "Book diff update channel closed"
                        );
                        break;
                    }

                }
                _ = publish_timer.tick(), if self.publish_interval.is_some() => {
                    // Publish state snapshot at intervals
                    if let Some(ref state_producer) = self.state_producer {
                        let state = self.state.state_snapshot(self.limit);
                        if let Err(e) = state_producer.send(&state).await {
                            error!(
                                symbol = %self.symbol,
                                error = %e,
                                "Failed to publish orderbook state"
                            );
                        }

                        // Also persist state snapshot to database for depth chart visualization
                        if let Some(ref db_writer) = self.database_writer
                            && let Err(e) =
                                db_writer.write_orderbook_state(&state, &self.symbol).await
                        {
                            error!(
                                symbol = %self.symbol,
                                error = %e,
                                "Failed to write orderbook state to database"
                            );
                        }
                    }
                }

            }
        }
        info!(
            symbol = %self.symbol,
            "Order book processor completed"
        );
        Ok(())
    }

    async fn handle_snapshot_request(&mut self, reason: SnapshotReason) -> Result<()> {
        match self.get_snapshot(reason).await {
            Ok(new_snapshot) => {
                info!(
                    symbol = %self.symbol,
                    update_id = new_snapshot.last_update_id,
                    reason = %reason.as_str(),
                    "Applying new snapshot"
                );

                // Persist snapshot to database if configured
                if let Some(ref db_writer) = self.database_writer
                    && let Err(e) = db_writer
                        .write_depth_snapshot(&new_snapshot, &self.symbol, reason.as_str())
                        .await
                {
                    error!(
                        symbol = %self.symbol,
                        error = %e,
                        "Failed to write snapshot to database"
                    );
                }

                self.state.apply_snapshot(new_snapshot);

                // Publish orderbook state when we get a new snapshot
                if let Some(ref state_producer) = self.state_producer {
                    let state = self.state.state_snapshot(self.limit);
                    if let Err(e) = state_producer.send(&state).await {
                        error!(
                            symbol = %self.symbol,
                            error = %e,
                            "Failed to publish orderbook state after snapshot"
                        );
                    }

                    // Also persist state snapshot to database for depth chart visualization
                    if let Some(ref db_writer) = self.database_writer
                        && let Err(e) = db_writer.write_orderbook_state(&state, &self.symbol).await
                    {
                        error!(
                            symbol = %self.symbol,
                            error = %e,
                            "Failed to write orderbook state to database"
                        );
                    }
                }

                // Process any buffered updates
                self.process_buffered_updates_after_snapshot().await?;
            }
            Err(e) => {
                error!(
                    symbol = %self.symbol,
                    error = %e,
                    reason = %reason.as_str(),
                    "Failed to get snapshot"
                );
            }
        }
        Ok(())
    }

    async fn process_buffered_updates_after_snapshot(&mut self) -> Result<()> {
        let mut buffer = VecDeque::with_capacity(self.book_diff_update.len());
        while let Ok(buffered_update) = self.book_diff_update.try_recv() {
            buffer.push_back(buffered_update);
        }

        if !buffer.is_empty() {
            info!(
                symbol = %self.symbol,
                buffer_size = buffer.len(),
                "Processing buffered updates after snapshot"
            );

            match self.state.process_update_buffer(buffer) {
                ProcessResult::Updated(_) => {
                    info!(
                        symbol = %self.symbol,
                        "Buffered updates processed successfully after snapshot"
                    );
                }
                ProcessResult::NeedsSnapshot(reason) => {
                    warn!(
                        symbol = %self.symbol,
                        reason = %reason.as_str(),
                        "Buffered updates after snapshot require another snapshot"
                    );
                    let new_snapshot = self.get_snapshot(reason).await?;
                    self.state.apply_snapshot(new_snapshot);
                }
                ProcessResult::Stale => {
                    info!(
                        symbol = %self.symbol,
                        "No relevant buffered updates after snapshot"
                    );
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
            .map_err(|e| anyhow::anyhow!("Failed to send snapshot request: {e}"))?;
        response_rx.await?
    }
}

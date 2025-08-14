use std::collections::VecDeque;

use anyhow::Result;
use tokio::sync::{mpsc, mpsc::UnboundedReceiver as Receiver, oneshot};
use tracing::{error, info, warn};

use super::book_state::OrderBookState;
use crate::{
    book::book_state::ProcessResult,
    data::binance::models::{DepthSnapshot, DepthUpdate},
};

pub trait SnapshotProvider: Send + Sync {
    async fn get_snapshot(&self, symbol: &str, limit: Option<u32>) -> Result<DepthSnapshot>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotReason {
    Initial,
    BufferedUpdates,
}

impl SnapshotReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::BufferedUpdates => "buffered_updates",
        }
    }
}

// Request/Response types for snapshot channel
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
        assert!(
            limit.is_none_or(|l| l > 0),
            "Limit must be None or greater than 0"
        );
        assert!(
            limit.is_none_or(|l| l <= 5000),
            "Limit must be None or less than or equal to 5000"
        );
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

    pub fn send(self, sender: &SnapshotRequestSender) -> Result<()> {
        sender
            .send(self)
            .map_err(|e| anyhow::anyhow!("Failed to send snapshot request: {}", e))
    }
}

pub type SnapshotRequestSender = mpsc::UnboundedSender<SnapshotRequest>;
pub type SnapshotRequestReceiver = mpsc::UnboundedReceiver<SnapshotRequest>;

pub struct OrderBook {
    symbol: String,
    limit: Option<i32>,
    state: OrderBookState,
    book_diff_update: Receiver<DepthUpdate>,
    snapshot_request_tx: SnapshotRequestSender,
}

impl OrderBook {
    pub async fn new(
        symbol: String,
        limit: Option<i32>,
        mut book_diff_update: Receiver<DepthUpdate>,
        snapshot_request_tx: SnapshotRequestSender,
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
        let mut state = OrderBookState::from_snapshot(depth_snapshot);

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
        })
    }

    pub async fn run(mut self) -> Result<()> {
        info!(symbol = %self.symbol, "Starting order book for {}", self.symbol);
        while let Some(update) = self.book_diff_update.recv().await {
            match self.state.process_update(&update)? {
                ProcessResult::Updated => {
                    info!(symbol = %self.symbol, update_id = update.final_update_id,
                                "Processed update");
                }
                ProcessResult::NeedsSnapshot => {
                    info!("Requesting new snapshot for {}", self.symbol);

                    match self.get_snapshot(SnapshotReason::BufferedUpdates).await {
                        Ok(new_snapshot) => {
                            info!(
                                symbol = %self.symbol,
                                new_update_id = new_snapshot.last_update_id,
                                "Applying new snapshot"
                            );
                            self.state.apply_snapshot(new_snapshot);
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
                                let buffer_state = self.state.process_update_buffer(buffer_deque);
                                // Do we need to handle the result of processing the buffer?
                                match buffer_state {
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
                        }
                        Err(e) => {
                            error!("Failed to get snapshot: {}", e);
                        }
                    }
                }
                ProcessResult::Stale => (),
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
        request.send(sender)?;
        response_rx.await?
    }
}

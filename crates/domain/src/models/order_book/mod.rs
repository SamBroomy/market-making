pub mod half_book;
pub mod last_snapshot;

use std::collections::VecDeque;

use anyhow::Result;
use bincode::{Decode, Encode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, trace};

use crate::models::{
    indicators::MarketDataCalculator,
    market_data::{DepthSnapshot, DepthUpdate, MarketDataSummary, PriceLevels},
    order_book::{
        half_book::{AskBook, BidBook},
        last_snapshot::LastSnapshot,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct StateSnapshot {
    #[bincode(with_serde)]
    pub bids: PriceLevels,
    #[bincode(with_serde)]
    pub asks: PriceLevels,
    pub last_update_id: u64,
    #[bincode(with_serde)]
    pub last_update_time: DateTime<Utc>,
    pub depth_limit: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotReason {
    /// Initial snapshot when order book is first created
    Initial,
    /// Sequence gap detected in buffered updates during initialization
    BufferedUpdates,
    /// Sequence gap detected in live updates
    SequenceGap,
    /// Price has moved significantly from last snapshot bounds
    PriceMovement,
    /// Generic resync request
    Resync,
}
impl SnapshotReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::BufferedUpdates => "buffered_updates",
            Self::SequenceGap => "sequence_gap",
            Self::PriceMovement => "price_movement",
            Self::Resync => "resync",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProcessResult {
    Updated(MarketDataSummary),
    NeedsSnapshot(SnapshotReason),
    Stale,
}

#[derive(Debug, Clone)]
pub struct OrderBook {
    bids: BidBook,
    asks: AskBook,
    last_snapshot: LastSnapshot,
    last_update_id: u64,
    last_update_time: DateTime<Utc>,
}

impl OrderBook {
    #[must_use]
    pub fn from_snapshot(snapshot: DepthSnapshot) -> Self {
        let bid_depth = BidBook::from_snapshot(&snapshot.bids);
        let ask_depth = AskBook::from_snapshot(&snapshot.asks);
        let last_update_time = Utc::now();
        let last_update_id = snapshot.last_update_id;
        let last_snapshot = LastSnapshot::new(snapshot);

        Self {
            bids: bid_depth,
            asks: ask_depth,
            last_snapshot,
            last_update_id,
            last_update_time,
        }
    }

    /// Apply a new snapshot and reset our bounds
    pub fn apply_snapshot(&mut self, snapshot: DepthSnapshot) {
        debug!(
            last_update_id = snapshot.last_update_id,
            bid_levels = snapshot.bids.len(),
            ask_levels = snapshot.asks.len(),
            "Applying fresh order book snapshot"
        );
        self.bids.clear();
        self.asks.clear();

        self.bids.apply_snapshot_offers(&snapshot.bids);
        self.asks.apply_snapshot_offers(&snapshot.asks);
        self.last_update_time = Utc::now();
        self.last_update_id = snapshot.last_update_id;
        self.last_snapshot = LastSnapshot::new(snapshot);
    }

    /// Process live updates - more lenient about gaps
    pub fn process_update(&mut self, update: &DepthUpdate) -> Result<ProcessResult> {
        debug!(
            first_update_id = update.first_update_id,
            final_update_id = update.final_update_id,
            last_book_update_id = self.last_update_id,
            "Processing live order book update"
        );

        // Step 1: If the event u (final update ID) <= local update ID, ignore (stale)
        if update.final_update_id <= self.last_update_id {
            trace!(
                final_update_id = update.final_update_id,
                last_update_id = self.last_update_id,
                "Update is stale, ignoring"
            );
            return Ok(ProcessResult::Stale);
        }

        // Step 2: Check for sequence gaps - be lenient in live processing
        if update.first_update_id > self.last_update_id + 1 {
            let gap = update.first_update_id - self.last_update_id;

            debug!(
                first_update_id = update.first_update_id,
                last_update_id = self.last_update_id,
                gap = gap,
                "Sequence gap detected in live processing"
            );

            // Only error on very large gaps that indicate we're truly out of sync
            if gap > 50000 {
                return Err(anyhow::anyhow!(
                    "Large update sequence gap detected. ID {} is much greater than last ID {} (gap: {})",
                    update.first_update_id,
                    self.last_update_id,
                    gap
                ));
            }

            // For medium gaps, trigger snapshot
            if gap > 10000 {
                info!(
                    gap = gap,
                    "SEQUENCE GAP: Medium sequence gap detected, requesting new snapshot"
                );
                return Ok(ProcessResult::NeedsSnapshot(SnapshotReason::SequenceGap));
            }

            trace!(gap = gap, "Small sequence gap, continuing with update");
        }

        self.apply_update_changes(update);
        let current_best_bid = self.bids.best_price();
        let current_best_ask = self.asks.best_price();

        // Check if we've moved too far from our snapshot bounds
        if self
            .last_snapshot
            .needs_retrigger(current_best_bid, current_best_ask)
        {
            info!(
                best_bid = %current_best_bid,
                best_ask = %current_best_ask,
                "PRICE MOVEMENT: Price-based retrigger detected, requesting new snapshot"
            );
            return Ok(ProcessResult::NeedsSnapshot(SnapshotReason::PriceMovement));
        }

        Ok(ProcessResult::Updated(self.market_data_summary()))
    }

    /// Process buffered updates during initialization - more strict about sequence
    pub fn process_update_buffer(&mut self, buffer: VecDeque<DepthUpdate>) -> ProcessResult {
        let buffer_size = buffer.len();
        debug!(buffer_size, "Processing order book update buffer");

        let mut any_updates_applied = false;
        let mut discarded_count = 0;

        for update in buffer {
            // During initialization, follow Binance docs exactly:
            // "discard any event where u <= lastUpdateId of the snapshot"
            if update.final_update_id <= self.last_update_id {
                discarded_count += 1;
                continue;
            }

            // "The first buffered event should have U <= lastUpdateId + 1"
            // Be strict during initialization
            if update.first_update_id > self.last_update_id + 1 {
                debug!(
                    first_update_id = update.first_update_id,
                    last_update_id = self.last_update_id,
                    gap = update.first_update_id - self.last_update_id,
                    "Sequence gap in buffer - this indicates we need a new snapshot"
                );
                return ProcessResult::NeedsSnapshot(SnapshotReason::BufferedUpdates);
            }

            // Apply the update
            self.apply_update_changes(&update);
            any_updates_applied = true;
        }

        debug!(
            applied = any_updates_applied,
            discarded = discarded_count,
            final_update_id = self.last_update_id,
            "Buffer processing complete"
        );

        if any_updates_applied {
            // Check if we need a snapshot due to price movement
            let current_best_bid = self.bids.best_price();
            let current_best_ask = self.asks.best_price();

            if self
                .last_snapshot
                .needs_retrigger(current_best_bid, current_best_ask)
            {
                ProcessResult::NeedsSnapshot(SnapshotReason::PriceMovement)
            } else {
                ProcessResult::Updated(self.market_data_summary())
            }
        } else {
            ProcessResult::Stale
        }
    }

    fn apply_update_changes(&mut self, update: &DepthUpdate) {
        self.bids.apply_changes(&update.bids);
        self.asks.apply_changes(&update.asks);
        debug!(
            update_id = update.final_update_id,
            "Applied order book update changes"
        );
        self.last_update_id = update.final_update_id;
        self.last_update_time = update.event_time;
    }

    #[must_use]
    fn market_data_summary(&self) -> MarketDataSummary {
        MarketDataCalculator::new(
            &self.bids,
            &self.asks,
            self.last_update_id,
            self.last_update_time,
        )
        .market_data_summary()
    }

    #[must_use]
    pub fn state_snapshot(&self, limit: Option<i32>) -> StateSnapshot {
        let depth_limit = limit.map_or(500, |l| l.min(1000));

        let bids: PriceLevels = self
            .bids
            .iter()
            .take(depth_limit as usize)
            .map(|(k, v)| (*k, *v))
            .collect();
        let asks: PriceLevels = self
            .asks
            .iter()
            .take(depth_limit as usize)
            .map(|(k, v)| (*k, *v))
            .collect();

        StateSnapshot {
            bids,
            asks,
            last_update_id: self.last_update_id,
            last_update_time: self.last_update_time,
            depth_limit,
        }
    }
}

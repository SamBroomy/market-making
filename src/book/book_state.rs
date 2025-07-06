use std::collections::VecDeque;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{debug, info, trace};

use super::Price;
use crate::{
    book::{
        Volume,
        half_book::{AskBook, BidBook},
    },
    data::binance::models::{DepthSnapshot, DepthUpdate},
};

#[derive(Debug)]
pub enum ProcessResult {
    Updated,
    NeedsSnapshot,
    Stale,
}

/// Represents the last snapshot of the order book, including the bounds for bids and asks.
#[derive(Debug, Clone, Default)]
struct LastSnapshot {
    snapshot: DepthSnapshot,
    snapshot_bid_lower_bound: Price,
    snapshot_bid_retrigger_price: Price,
    snapshot_ask_upper_bound: Price,
    snapshot_ask_retrigger_price: Price,
    snapshot_time: DateTime<Utc>,
}
impl LastSnapshot {
    pub fn new(mut snapshot: DepthSnapshot) -> Self {
        snapshot.bids.sort_by(|a, b| b.price.cmp(&a.price)); // High to low
        snapshot.asks.sort_by(|a, b| a.price.cmp(&b.price)); // Low to high

        // Get the bounds of our 1000-level snapshot
        let best_bid = snapshot
            .bids
            .first()
            .expect("Bids should not be empty")
            .price;
        let worst_bid = snapshot
            .bids
            .last()
            .expect("Bids should not be empty")
            .price;
        let best_ask = snapshot
            .asks
            .first()
            .expect("Asks should not be empty")
            .price;
        let worst_ask = snapshot
            .asks
            .last()
            .expect("Asks should not be empty")
            .price;

        // Calculate retrigger points at 50% of the range
        let bid_range = best_bid - worst_bid;
        let ask_range = worst_ask - best_ask;

        // Retrigger when bids fall below this level (halfway to worst bid)
        let snapshot_bid_retrigger_price = best_bid - (bid_range / dec!(2));

        // Retrigger when asks rise above this level (halfway to worst ask)
        let snapshot_ask_retrigger_price = best_ask + (ask_range / dec!(2));

        Self {
            snapshot,
            snapshot_bid_lower_bound: worst_bid,
            snapshot_bid_retrigger_price,
            snapshot_ask_upper_bound: worst_ask,
            snapshot_ask_retrigger_price,
            snapshot_time: Utc::now(),
        }
    }

    fn bids_cause_retrigger(&self, current_best_bid: Price) -> bool {
        if current_best_bid <= self.snapshot_bid_retrigger_price {
            info!(
                current_best_bid = %current_best_bid,
                snapshot_bid_retrigger_price = %self.snapshot_bid_retrigger_price,
                "Bid retrigger condition met"
            );
            return true;
        }
        false
    }

    fn asks_cause_retrigger(&self, current_best_ask: Price) -> bool {
        if current_best_ask >= self.snapshot_ask_retrigger_price {
            info!(
                current_best_ask = %current_best_ask,
                snapshot_ask_retrigger_price = %self.snapshot_ask_retrigger_price,
                "Ask retrigger condition met"
            );
            return true;
        }
        false
    }

    fn calculate_bid_deviation(&self, current_best_bid: Price) -> Decimal {
        // Measure how far we are toward the retrigger boundary
        if current_best_bid < self.snapshot_bid_lower_bound {
            return dec!(1.0); // 100% deviation if below lower bound
        }
        if current_best_bid >= self.snapshot.bids[0].price {
            return dec!(0.0); // No deviation if at/above best bid
        }
        // Calculate position between best bid and lower bound
        let total_range = self.snapshot.bids[0].price - self.snapshot_bid_lower_bound;
        let remaining_distance = current_best_bid - self.snapshot_bid_lower_bound;
        // Return 1.0 - (remaining distance / total range)
        // This gives 0% at best bid, 100% at lower bound
        dec!(1.0)
            - remaining_distance
                .checked_div(total_range)
                .unwrap_or(dec!(1.0))
    }

    fn calculate_ask_deviation(&self, current_best_ask: Price) -> Decimal {
        if current_best_ask > self.snapshot_ask_upper_bound {
            return dec!(1.0); // 100% deviation if above upper bound
        }
        if current_best_ask <= self.snapshot.asks[0].price {
            return dec!(0.0); // No deviation if at/below best ask
        }
        // Calculate position between best ask and upper bound
        let total_range = self.snapshot_ask_upper_bound - self.snapshot.asks[0].price;
        let current_distance = current_best_ask - self.snapshot.asks[0].price;
        current_distance
            .checked_div(total_range)
            .unwrap_or(dec!(1.0))
    }

    // Check if we need a new snapshot
    fn needs_retrigger(&self, current_best_bid: Price, current_best_ask: Price) -> bool {
        // self.asks_cause_retrigger(current_best_ask) || self.bids_cause_retrigger(current_best_bid)
        // Calculate how far we've moved from the center of our s
        let bid_deviation = self.calculate_bid_deviation(current_best_bid);
        let ask_deviation = self.calculate_ask_deviation(current_best_ask);

        // Adaptive thresholds based on how long since last snapshot
        let time_since_snapshot = Utc::now() - self.snapshot_time;
        let threshold = self.calculate_adaptive_threshold(time_since_snapshot);

        let needs_retrigger = bid_deviation > threshold || ask_deviation > threshold;
        if needs_retrigger {
            info!(
                bid_deviation = %bid_deviation,
                ask_deviation = %ask_deviation,
                threshold = %threshold,
                time_since_snapshot = ?time_since_snapshot,
                "Adaptive retrigger condition met"
            );
        }

        needs_retrigger
    }

    fn calculate_adaptive_threshold(
        &self,
        time_elapsed: chrono::Duration,
    ) -> rust_decimal::Decimal {
        let base_threshold = dec!(0.5);
        let time_factor = Decimal::from(time_elapsed.num_seconds()) / dec!(1800); // 30 min
        let time_multiplier = dec!(1.0) + time_factor;
        let threshold = (base_threshold * time_multiplier).min(dec!(0.85));
        if time_elapsed.num_minutes() % 10 == 0 {
            info!(
                snapshot_age_minutes = time_elapsed.num_minutes(),
                threshold_percent = %threshold,
                time_factor = %time_factor,
                "Adaptive threshold status"
            );
        }

        threshold
    }
}

#[derive(Debug, Clone)]
pub struct OrderBookState {
    bid_depth: BidBook,
    ask_depth: AskBook,
    last_snapshot: LastSnapshot,
    last_update_id: u64,
    last_update_time: DateTime<Utc>,
}

impl OrderBookState {
    #[must_use]
    pub fn from_snapshot(snapshot: DepthSnapshot) -> Self {
        let bid_depth = BidBook::from_snapshot(&snapshot.bids);
        let ask_depth = AskBook::from_snapshot(&snapshot.asks);
        let last_update_time = Utc::now();
        let last_update_id = snapshot.last_update_id;
        let last_snapshot = LastSnapshot::new(snapshot);

        Self {
            bid_depth,
            ask_depth,
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
        self.bid_depth.clear();
        self.ask_depth.clear();

        self.bid_depth.apply_snapshot_offers(&snapshot.bids);
        self.ask_depth.apply_snapshot_offers(&snapshot.asks);
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
                return Ok(ProcessResult::NeedsSnapshot);
            }

            trace!(gap = gap, "Small sequence gap, continuing with update");
        }

        self.apply_update_changes(update);
        let current_best_bid = self.bid_depth.best_price();
        let current_best_ask = self.ask_depth.best_price();

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
            return Ok(ProcessResult::NeedsSnapshot);
        }

        Ok(ProcessResult::Updated)
    }

    /// Process buffered updates during initialization - more strict about sequence
    pub fn process_update_buffer(
        &mut self,
        buffer: VecDeque<DepthUpdate>,
    ) -> Result<ProcessResult> {
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
                return Ok(ProcessResult::NeedsSnapshot);
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
            let current_best_bid = self.bid_depth.best_price();
            let current_best_ask = self.ask_depth.best_price();

            if self
                .last_snapshot
                .needs_retrigger(current_best_bid, current_best_ask)
            {
                return Ok(ProcessResult::NeedsSnapshot);
            }

            Ok(ProcessResult::Updated)
        } else {
            Ok(ProcessResult::Stale)
        }
    }

    fn apply_update_changes(&mut self, update: &DepthUpdate) {
        self.bid_depth.apply_changes(&update.bids);
        self.ask_depth.apply_changes(&update.asks);
        debug!(
            update_id = update.final_update_id,
            "Applied order book update changes"
        );
        self.last_update_id = update.final_update_id;
        self.last_update_time = update.event_time;
    }

    fn calculate_vamp(&self, volume_cutoff_dolars: Volume) -> Option<Price> {
        let bid_vwap = self.bid_depth.volume_weighted_price(volume_cutoff_dolars)?;
        let ask_vwap = self.ask_depth.volume_weighted_price(volume_cutoff_dolars)?;
        Some((bid_vwap + ask_vwap) / dec!(2))
    }
}

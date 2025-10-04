use chrono::{DateTime, Duration, Utc};
use rust_decimal::{Decimal, dec};
use tracing::info;

use crate::models::market_data::{DepthSnapshot, Price};
/// Represents the last snapshot of the order book, including the bounds for bids and asks.
#[derive(Debug, Clone, Default)]
pub struct LastSnapshot {
    snapshot: DepthSnapshot,
    snapshot_bid_lower_bound: Price,
    snapshot_bid_retrigger_price: Price,
    snapshot_ask_upper_bound: Price,
    snapshot_ask_retrigger_price: Price,
    snapshot_time: DateTime<Utc>,
}
impl LastSnapshot {
    #[must_use]
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
    pub fn needs_retrigger(&self, current_best_bid: Price, current_best_ask: Price) -> bool {
        // self.asks_cause_retrigger(current_best_ask) || self.bids_cause_retrigger(current_best_bid)
        // Calculate how far we've moved from the center of our s
        let bid_deviation = self.calculate_bid_deviation(current_best_bid);
        let ask_deviation = self.calculate_ask_deviation(current_best_ask);

        // Adaptive thresholds based on how long since last snapshot
        let time_since_snapshot = Utc::now() - self.snapshot_time;
        let threshold = Self::calculate_adaptive_threshold(time_since_snapshot);

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

    fn calculate_adaptive_threshold(time_elapsed: Duration) -> Decimal {
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

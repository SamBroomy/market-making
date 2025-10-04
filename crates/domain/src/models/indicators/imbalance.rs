use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, dec};

use crate::models::{
    market_data::MarketDataSummary,
    order_book::half_book::{AskBook, BidBook},
};

/// Market data calculator that computes all metrics in one pass
pub struct MarketDataCalculator<'a> {
    bids: &'a BidBook,
    asks: &'a AskBook,
    last_update_id: u64,
    last_update_time: DateTime<Utc>,

    // Cached L1 values
    best_bid: Decimal,
    best_ask: Decimal,
    bid_volume_l1: Decimal,
    ask_volume_l1: Decimal,
    bid_volume_l5: Decimal,
    ask_volume_l5: Decimal,

    // Cached calculations
    mid_price: Decimal,
    spread: Decimal,
    spread_bps: Decimal,
}

impl<'a> MarketDataCalculator<'a> {
    #[must_use]
    pub fn new(
        bids: &'a BidBook,
        asks: &'a AskBook,
        last_update_id: u64,
        last_update_time: DateTime<Utc>,
    ) -> Self {
        // Calculate L1 values once
        let best_bid = bids.best_price();
        let best_ask = asks.best_price();
        let bid_volume_l1 = bids.best_quote();
        let ask_volume_l1 = asks.best_quote();
        let (bid_volume_l5, ask_volume_l5) = {
            let bid_volume_l5: Decimal = bids.iter().take(5).map(|(_, &size)| size).sum();
            let ask_volume_l5: Decimal = asks.iter().take(5).map(|(_, &size)| size).sum();
            (bid_volume_l5, ask_volume_l5)
        };

        // Calculate derived values once
        let mid_price = (best_ask + best_bid) / dec!(2);
        let spread = best_ask - best_bid;
        let spread_bps = if mid_price > dec!(0) {
            (spread / mid_price) * dec!(10000)
        } else {
            dec!(0)
        };

        Self {
            bids,
            asks,
            last_update_id,
            last_update_time,
            best_bid,
            best_ask,
            bid_volume_l1,
            ask_volume_l1,
            bid_volume_l5,
            ask_volume_l5,
            mid_price,
            spread,
            spread_bps,
        }
    }

    /// Calculate quote imbalance for N levels (cached for L1)
    fn quote_imbalance(bid_volume: Decimal, ask_volume: Decimal) -> Decimal {
        if bid_volume + ask_volume == dec!(0) {
            return dec!(0);
        }
        (bid_volume - ask_volume) / (bid_volume + ask_volume)
    }

    /// Stoikov's micro-price using cached values
    fn calculate_micro_price(&self, quote_imbalance_l1: Decimal) -> Decimal {
        let adjustment_factor = dec!(0.5);
        let imbalance_adjustment =
            (quote_imbalance_l1 - dec!(0.5)) * self.spread * adjustment_factor;
        self.mid_price + imbalance_adjustment
    }

    /// Weighted mid using cached values
    fn calculate_weighted_mid(&self, quote_imbalance_l1: Decimal) -> Decimal {
        (quote_imbalance_l1 * self.best_ask) + ((dec!(1) - quote_imbalance_l1) * self.best_bid)
    }

    /// Convert imbalance from [-1,1] to [0,1] range
    #[inline]
    fn normalize_imbalance(imbalance: Decimal) -> Decimal {
        (imbalance + dec!(1)) / dec!(2)
    }

    /// Generate complete market data summary with minimal recalculation
    #[must_use]
    pub fn market_data_summary(self) -> MarketDataSummary {
        // Calculate imbalances once
        let quote_imbalance_l1_raw = Self::quote_imbalance(self.bid_volume_l1, self.ask_volume_l1);
        let quote_imbalance_l5_raw = Self::quote_imbalance(self.bid_volume_l5, self.ask_volume_l5);

        // Normalize once
        let quote_imbalance_l1 = Self::normalize_imbalance(quote_imbalance_l1_raw);
        let quote_imbalance_l5 = Self::normalize_imbalance(quote_imbalance_l5_raw);

        // Use L1 imbalance for derived metrics (reuse calculation)
        let weighted_mid = self.calculate_weighted_mid(quote_imbalance_l1);
        let micro_price = self.calculate_micro_price(quote_imbalance_l1);

        MarketDataSummary {
            event_time: self.last_update_time,
            spread_bps: self.spread_bps,
            mid_price: self.mid_price,
            bid_volume_l1: self.bid_volume_l1,
            ask_volume_l1: self.ask_volume_l1,
            quote_imbalance_l1,
            bid_volume_l5: self.bid_volume_l5,
            ask_volume_l5: self.ask_volume_l5,
            quote_imbalance_l5,
            weighted_mid,
            micro_price,
            update_id: self.last_update_id,
        }
    }
}

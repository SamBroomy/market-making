use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub type Price = Decimal;
pub type Size = Decimal;
pub type Volume = Decimal;

pub type PriceLevels = BTreeMap<Price, Size>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketDataSummary {
    pub event_time: DateTime<Utc>,
    pub spread_bps: Decimal,
    pub mid_price: Price,
    // L1 raw quantities
    pub bid_volume_l1: Volume,
    pub ask_volume_l1: Volume,
    pub quote_imbalance_l1: Decimal, // [0,1] normalized for trading algorithms
    // L5 raw quantities
    pub bid_volume_l5: Volume,
    pub ask_volume_l5: Volume,
    pub quote_imbalance_l5: Decimal, // [0,1] normalized for trading algorithms

    pub weighted_mid: Decimal,
    pub micro_price: Decimal, // Stoikov's micro-price
    pub update_id: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeSummary {
    pub event_time: DateTime<Utc>,
    pub buy_volume: Decimal,
    pub sell_volume: Decimal,
    pub trade_count: i32,
    pub trade_intensity: Decimal, // trades/second
    pub imbalance: Decimal,       // Exponentially weighted
    pub volatility: Decimal,      // Simple variance
}

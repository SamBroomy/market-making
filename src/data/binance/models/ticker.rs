use bincode::{Decode, Encode};
use chrono::{DateTime, Utc, serde::ts_milliseconds};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
/// Latest book data for a symbol
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BookTickerEvent {
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b", with = "rust_decimal::serde::str")]
    pub best_bid_price: Decimal,
    #[serde(rename = "B", with = "rust_decimal::serde::str")]
    pub best_bid_qty: Decimal,
    #[serde(rename = "a", with = "rust_decimal::serde::str")]
    pub best_ask_price: Decimal,
    #[serde(rename = "A", with = "rust_decimal::serde::str")]
    pub best_ask_qty: Decimal,
}

/// Mini Ticker for 24hr stats
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MiniTickerData {
    #[serde(rename = "E")]
    pub event_time: u64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "c", with = "rust_decimal::serde::str")]
    pub close_price: Decimal,
    #[serde(rename = "o", with = "rust_decimal::serde::str")]
    pub open_price: Decimal,
    #[serde(rename = "h", with = "rust_decimal::serde::str")]
    pub high_price: Decimal,
    #[serde(rename = "l", with = "rust_decimal::serde::str")]
    pub low_price: Decimal,
    #[serde(rename = "v", with = "rust_decimal::serde::str")]
    pub volume: Decimal,
    #[serde(rename = "q", with = "rust_decimal::serde::str")]
    pub quote_volume: Decimal,
}

/// Full Ticker (24hr stats with more details)
#[derive(Debug, Deserialize, Serialize, Clone, Decode, Encode)]
pub struct TickerData {
    #[serde(rename = "E", with = "ts_milliseconds")]
    #[bincode(with_serde)]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub price_change: Decimal,
    #[serde(rename = "P", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub price_change_percent: Decimal,
    #[serde(rename = "w", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub weighted_avg_price: Decimal,
    #[serde(rename = "x", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub first_trade_price: Decimal,
    #[serde(rename = "c", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub last_price: Decimal,
    #[serde(rename = "Q", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub last_quantity: Decimal,
    #[serde(rename = "b", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub best_bid_price: Decimal,
    #[serde(rename = "B", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub best_bid_quantity: Decimal,
    #[serde(rename = "a", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub best_ask_price: Decimal,
    #[serde(rename = "A", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub best_ask_quantity: Decimal,
    #[serde(rename = "o", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub open_price: Decimal,
    #[serde(rename = "h", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub high_price: Decimal,
    #[serde(rename = "l", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub low_price: Decimal,
    #[serde(rename = "v", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub volume: Decimal,
    #[serde(rename = "q", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub quote_volume: Decimal,
    #[serde(rename = "O", with = "ts_milliseconds")]
    #[bincode(with_serde)]
    pub open_time: DateTime<Utc>,
    #[serde(rename = "C", with = "ts_milliseconds")]
    #[bincode(with_serde)]
    pub close_time: DateTime<Utc>,
    #[serde(rename = "F")]
    pub first_trade_id: u64,
    #[serde(rename = "L")]
    pub last_trade_id: u64,
    #[serde(rename = "n")]
    pub trade_count: u64,
}

/// Rolling Window Statistics (1h, 4h, 1d)
#[derive(Debug, Deserialize, Serialize, Clone, Decode, Encode)]
pub struct WindowTickerData {
    #[serde(rename = "e")]
    pub event_type: String, // "1hTicker", "4hTicker", etc.
    #[serde(rename = "E", with = "ts_milliseconds")]
    #[bincode(with_serde)]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub price_change: Decimal,
    #[serde(rename = "P", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub price_change_percent: Decimal,
    #[serde(rename = "o", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub open_price: Decimal,
    #[serde(rename = "h", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub high_price: Decimal,
    #[serde(rename = "l", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub low_price: Decimal,
    #[serde(rename = "c", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub close_price: Decimal,
    #[serde(rename = "w", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub weighted_avg_price: Decimal,
    #[serde(rename = "v", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub volume: Decimal,
    #[serde(rename = "q", with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub quote_volume: Decimal,
    #[serde(rename = "O", with = "ts_milliseconds")]
    #[bincode(with_serde)]
    pub open_time: DateTime<Utc>,
    #[serde(rename = "C", with = "ts_milliseconds")]
    #[bincode(with_serde)]
    pub close_time: DateTime<Utc>,
    #[serde(rename = "F")]
    pub first_trade_id: u64,
    #[serde(rename = "L")]
    pub last_trade_id: u64,
    #[serde(rename = "n")]
    pub trade_count: u64,
}

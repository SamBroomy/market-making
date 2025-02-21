use chrono::{serde::ts_milliseconds, DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AveragePrice {
    // #[serde(rename = "e")]
    // event_type: String,
    #[serde(rename = "E", with = "ts_milliseconds")]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "i")]
    interval: String,
    #[serde(rename = "w", with = "rust_decimal::serde::str")]
    average_price: Decimal,
    #[serde(rename = "T", with = "ts_milliseconds")]
    last_trade_time: DateTime<Utc>,
}

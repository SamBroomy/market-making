use chrono::{serde::ts_milliseconds, DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
#[derive(Debug, Deserialize)]
//#[serde(deny_unknown_fields)]
pub struct KlineEventData {
    // #[serde(rename = "e")]
    // event_type: String,
    #[serde(rename = "E", with = "ts_milliseconds")]
    event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "k")]
    kline: KlineData,
}

#[derive(Debug, Deserialize)]
//#[serde(deny_unknown_fields)]
pub struct KlineData {
    #[serde(rename = "t", with = "ts_milliseconds")]
    start_time: DateTime<Utc>,
    #[serde(rename = "T", with = "ts_milliseconds")]
    close_time: DateTime<Utc>,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "i")]
    interval: String,
    #[serde(rename = "f")]
    first_trade_id: u64,
    #[serde(rename = "L")]
    last_trade_id: u64,
    #[serde(rename = "o", with = "rust_decimal::serde::str")]
    open_price: Decimal,
    #[serde(rename = "c", with = "rust_decimal::serde::str")]
    close_price: Decimal,
    #[serde(rename = "h", with = "rust_decimal::serde::str")]
    high_price: Decimal,
    #[serde(rename = "l", with = "rust_decimal::serde::str")]
    low_price: Decimal,
    #[serde(rename = "v", with = "rust_decimal::serde::str")]
    base_asset_volume: Decimal,
    #[serde(rename = "n")]
    number_of_trades: u64,
    #[serde(rename = "x")]
    is_kline_closed: bool,
    #[serde(rename = "q", with = "rust_decimal::serde::str")]
    quote_asset_volume: Decimal,
    #[serde(rename = "V", with = "rust_decimal::serde::str")]
    taker_buy_base_asset_volume: Decimal,
    #[serde(rename = "Q", with = "rust_decimal::serde::str")]
    taker_buy_quote_asset_volume: Decimal,
    #[serde(rename = "B", skip)]
    _ignore: (),
}

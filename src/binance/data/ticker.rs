use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BookTickerData {
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

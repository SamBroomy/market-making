use bincode::{Decode, Encode};
use chrono::{DateTime, Utc, serde::ts_milliseconds};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone, Copy, Serialize, Decode, Encode)]
pub struct OfferData {
    #[serde(with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[bincode(with_serde)]
    pub size: Decimal,
}
impl From<(Decimal, Decimal)> for OfferData {
    fn from((price, size): (Decimal, Decimal)) -> Self {
        Self { price, size }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DepthUpdate {
    #[serde(rename = "E", with = "ts_milliseconds")]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "U")]
    pub first_update_id: u64,
    #[serde(rename = "u")]
    pub final_update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<OfferData>,
    #[serde(rename = "a")]
    pub asks: Vec<OfferData>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, Decode, Encode)]
#[serde(rename_all = "camelCase")]
pub struct DepthSnapshot {
    pub last_update_id: u64,
    pub bids: Vec<OfferData>,
    pub asks: Vec<OfferData>,
}

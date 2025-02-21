use serde::Deserialize;

mod depth_update;
mod historical_data;
mod kline;
mod price;
mod ticker;
mod trade;

pub use depth_update::{DepthSnapshot, DepthUpdate, OfferData};
pub use kline::KlineEventData;
pub use price::AveragePrice;
pub use trade::{AggregateTrade, TradeEventData};

#[derive(Debug, Deserialize)]
#[serde(tag = "e")] // This tells serde to look for the "e" field as the enum discriminator
#[serde(rename_all = "lowercase")] // Makes "Trade" match with "trade" in JSON
pub enum BinanceEvent {
    Trade(TradeEventData),
    Kline(KlineEventData),
    #[serde(rename = "avgPrice")]
    AvgPrice(AveragePrice),
    #[serde(rename = "depthUpdate")]
    DepthUpdate(DepthUpdate),
    #[serde(rename = "aggTrade")]
    AggTrade(AggregateTrade),
    #[serde(other)] // Catch all other event types
    Unknown,
}

impl BinanceEvent {
    pub fn handle_unknown_event(data: &str) -> serde_json::Value {
        serde_json::from_str(data).unwrap_or_default()
    }
}

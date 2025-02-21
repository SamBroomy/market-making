use chrono::{serde::ts_milliseconds, DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
//#[serde(deny_unknown_fields)]
pub struct TradeEventData {
    // #[serde(rename = "e")]
    // event_type: String,
    #[serde(rename = "E", with = "ts_milliseconds")]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "t")]
    pub trade_id: u64,
    #[serde(rename = "p", with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(rename = "q", with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
    #[serde(rename = "T", with = "ts_milliseconds")]
    pub trade_time: DateTime<Utc>,
    #[serde(rename = "m")]
    pub buyer_market_maker: bool,
}

#[derive(Debug, Deserialize)]
pub struct AggregateTrade {
    #[serde(rename = "E", with = "ts_milliseconds")]
    pub event_time: DateTime<Utc>,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "a")]
    pub aggregate_trade_id: u64,
    #[serde(rename = "p", with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(rename = "q", with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
    #[serde(rename = "f")]
    pub first_trade_id: u64,
    #[serde(rename = "l")]
    pub last_trade_id: u64,
    #[serde(rename = "T", with = "ts_milliseconds")]
    pub trade_time: DateTime<Utc>,
    #[serde(rename = "m")]
    pub buyer_market_maker: bool,
    #[serde(rename = "M", skip)]
    _ignore: (),
}

struct Trade {
    price: Decimal,
    quantity: Decimal,
    trade_time: DateTime<Utc>,
    buyer_market_maker: bool,
    num_trades: u64,
}

impl From<TradeEventData> for Trade {
    fn from(event: TradeEventData) -> Self {
        Self {
            price: event.price,
            quantity: event.quantity,
            trade_time: event.trade_time,
            buyer_market_maker: event.buyer_market_maker,
            num_trades: 1,
        }
    }
}

impl From<AggregateTrade> for Trade {
    fn from(event: AggregateTrade) -> Self {
        Self {
            price: event.price,
            quantity: event.quantity,
            trade_time: event.trade_time,
            buyer_market_maker: event.buyer_market_maker,
            num_trades: event.last_trade_id - event.first_trade_id + 1,
        }
    }
}

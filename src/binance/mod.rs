use std::collections::BTreeMap;

use data::{BinanceEvent, DepthUpdate, TradeEventData};
use serde::Deserialize;

pub mod data;

use rust_decimal::Decimal;
use tracing::debug;

#[derive(Debug, Default)]
pub struct VolumeProfile {
    // Price -> Volume data
    volume_by_price: BTreeMap<Decimal, VolumeData>,
    // Configurable price bucket size
    bucket_size: Decimal,
}

#[derive(Debug, Default)]
pub struct VolumeData {
    total_volume: Decimal,
    buy_volume: Decimal,
    sell_volume: Decimal,
    trade_count: u64,
    // For order flow analysis
    bid_volume_delta: Decimal,
    ask_volume_delta: Decimal,
}

impl VolumeProfile {
    pub fn new(bucket_size: Decimal) -> Self {
        Self {
            volume_by_price: BTreeMap::new(),
            bucket_size,
        }
    }

    pub fn get_price_bucket(&self, price: Decimal) -> Decimal {
        (price / self.bucket_size).floor() * self.bucket_size
    }

    pub fn update_from_agg_trade(&mut self, trade: &data::AggregateTrade) {
        let bucket_price = self.get_price_bucket(trade.price);
        let data = self.volume_by_price.entry(bucket_price).or_default();

        data.total_volume += trade.quantity;
        if trade.buyer_market_maker {
            data.sell_volume += trade.quantity;
        } else {
            data.buy_volume += trade.quantity;
        }
        data.trade_count += 1;
    }

    pub fn update_from_trade(&mut self, trade: &TradeEventData) {
        let bucket_price = self.get_price_bucket(trade.price);
        let data = self.volume_by_price.entry(bucket_price).or_default();

        data.total_volume += trade.quantity;
        if trade.buyer_market_maker {
            data.sell_volume += trade.quantity;
        } else {
            data.buy_volume += trade.quantity;
        }

        data.trade_count += 1;
    }

    pub fn update_from_depth(&mut self, update: &DepthUpdate) {
        // Accumulate deltas per bucket: (bid_delta, ask_delta)
        let mut accum: BTreeMap<Decimal, (Decimal, Decimal)> = BTreeMap::new();

        for bid in &update.bids {
            let bucket_price = self.get_price_bucket(bid.price);
            let (bid_delta, _) = accum.entry(bucket_price).or_default();
            *bid_delta += bid.size;
        }

        for ask in &update.asks {
            let bucket_price = self.get_price_bucket(ask.price);
            let (_, ask_delta) = accum.entry(bucket_price).or_default();
            *ask_delta += ask.size;
        }
        // Update the main volume_by_price map once per bucket
        for (bucket_price, (bid_delta, ask_delta)) in accum {
            let data = self.volume_by_price.entry(bucket_price).or_default();
            data.bid_volume_delta += bid_delta;
            data.ask_volume_delta += ask_delta;
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BinanceMessage {
    Wrapped {
        #[serde(rename = "stream", skip)]
        stream: String,
        //#[serde(flatten)]
        data: BinanceEvent,
    },
    Direct(BinanceEvent),
    Protocol(ProtocolMessage),
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ProtocolMessage {
    Heartbeat(u64),
    Response { result: serde_json::Value, id: u64 },
}

impl BinanceMessage {
    pub fn from_str_into_market_data(
        data: &str,
    ) -> Result<BinanceEvent, Option<serde_json::Error>> {
        match serde_json::from_str::<BinanceMessage>(data)? {
            BinanceMessage::Wrapped { data, .. } => Ok(data),
            BinanceMessage::Direct(data) => Ok(data),
            BinanceMessage::Protocol(msg) => {
                match msg {
                    ProtocolMessage::Heartbeat(timestamp) => {
                        debug!("Received heartbeat at {}", timestamp);
                    }
                    ProtocolMessage::Response { result, id } => {
                        debug!("Received response message: id={}, result={:?}", id, result);
                    }
                }
                Err(None)
            }
        }
    }
}
#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    result: serde_json::Value,
    id: u64,
}

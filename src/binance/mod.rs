use data::{
    AggregateTrade, AveragePrice, BinanceEvent, BookTickerEvent, DepthUpdate, KlineEventData,
    MiniTickerData, TickerData, TradeEventData, WindowTickerData,
};
use rust_decimal::Decimal;
use serde::{Deserialize, ser::Error};
use std::collections::BTreeMap;
use tracing::debug;

pub mod data;

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
        stream: String,
        data: serde_json::Value,
    },
    Protocol(ProtocolMessage),
    Direct(serde_json::Value),
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
        let message: BinanceMessage = serde_json::from_str(data)?;

        match message {
            BinanceMessage::Wrapped { stream, data } => {
                Self::from_stream_and_data(&stream, data).map_err(Option::Some)
            }
            BinanceMessage::Direct(data) => {
                // Fallback to parsing the data field directly
                Self::fallback_on_data(data).map_err(Option::Some)
            }
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

    fn from_stream_and_data(
        stream: &str,
        data: serde_json::Value,
    ) -> Result<BinanceEvent, serde_json::Error> {
        let pos = stream
            .find('@')
            .ok_or(serde_json::Error::custom("Unable to get data from stream"))?;

        let stream_type = &stream[pos + 1..];

        match stream_type {
            s if s.starts_with("aggTrade") => {
                serde_json::from_value::<AggregateTrade>(data).map(BinanceEvent::AggTrade)
            }
            s if s.starts_with("depth") => {
                serde_json::from_value::<DepthUpdate>(data).map(BinanceEvent::DepthUpdate)
            }
            s if s.starts_with("kline") => {
                serde_json::from_value::<KlineEventData>(data).map(BinanceEvent::Kline)
            }
            s if s.starts_with("trade") => {
                serde_json::from_value::<TradeEventData>(data).map(BinanceEvent::Trade)
            }
            s if s.starts_with("miniTicker") => {
                serde_json::from_value::<MiniTickerData>(data).map(BinanceEvent::MiniTicker)
            }
            s if s.starts_with("bookTicker") => {
                serde_json::from_value::<BookTickerEvent>(data).map(BinanceEvent::BookTicker)
            }
            s if s.starts_with("avgPrice") => {
                serde_json::from_value::<AveragePrice>(data).map(BinanceEvent::AvgPrice)
            }
            s if s.starts_with("ticker") => {
                if s.find('_').is_some() {
                    serde_json::from_value::<WindowTickerData>(data).map(BinanceEvent::WindowTicker)
                } else {
                    serde_json::from_value::<TickerData>(data).map(BinanceEvent::Ticker)
                }
            }
            _ => Self::fallback_on_data(data),
        }
    }

    fn fallback_on_data(data: serde_json::Value) -> Result<BinanceEvent, serde_json::Error> {
        // Fallback: check for 'e' field in data
        if let Some(event_type) = data.get("e").and_then(|v| v.as_str()) {
            match event_type {
                "trade" => {
                    if let Ok(trade) = serde_json::from_value::<TradeEventData>(data.clone()) {
                        return Ok(BinanceEvent::Trade(trade));
                    }
                }
                "aggTrade" => {
                    if let Ok(agg_trade) = serde_json::from_value::<AggregateTrade>(data.clone()) {
                        return Ok(BinanceEvent::AggTrade(agg_trade));
                    }
                }
                "kline" => {
                    if let Ok(kline) = serde_json::from_value::<KlineEventData>(data.clone()) {
                        return Ok(BinanceEvent::Kline(kline));
                    }
                }
                "depthUpdate" => {
                    if let Ok(depth) = serde_json::from_value::<DepthUpdate>(data.clone()) {
                        return Ok(BinanceEvent::DepthUpdate(depth));
                    }
                }
                "avgPrice" => {
                    if let Ok(avg_price) = serde_json::from_value::<AveragePrice>(data.clone()) {
                        return Ok(BinanceEvent::AvgPrice(avg_price));
                    }
                }
                "24hrMiniTicker" => {
                    if let Ok(mini_ticker) = serde_json::from_value::<MiniTickerData>(data.clone())
                    {
                        return Ok(BinanceEvent::MiniTicker(mini_ticker));
                    }
                }
                "24hrTicker" => {
                    if let Ok(ticker) = serde_json::from_value::<TickerData>(data.clone()) {
                        return Ok(BinanceEvent::Ticker(ticker));
                    }
                }
                // Handle Window Tickers (1hTicker, 4hTicker, 1dTicker)
                s if s.ends_with("Ticker") => {
                    if let Ok(window_ticker) =
                        serde_json::from_value::<WindowTickerData>(data.clone())
                    {
                        return Ok(BinanceEvent::WindowTicker(window_ticker));
                    }
                }
                _ => {}
            }
        }

        // Try BookTicker specifically (no 'e' field)
        if data.get("u").is_some()
            && data.get("s").is_some()
            && data.get("b").is_some()
            && data.get("a").is_some()
        {
            if let Ok(book_ticker) = serde_json::from_value::<BookTickerEvent>(data.clone()) {
                return Ok(BinanceEvent::BookTicker(book_ticker));
            }
        }
        Err(serde_json::Error::custom("Unable to parse data"))
    }
}

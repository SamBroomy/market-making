use serde::{Deserialize, ser::Error};
use tracing::debug;

use super::models::{
    AggregateTrade, AveragePrice, BinanceEvent, BookTickerEvent, DepthUpdate, KlineEventData,
    MiniTickerData, TickerData, TradeEventData, WindowTickerData,
};

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
        let message: Self = serde_json::from_str(data)?;

        match message {
            Self::Wrapped { stream, data } => {
                Self::from_stream_and_data(&stream, data).map_err(Some)
            }
            Self::Direct(data) => {
                // Fallback to parsing the data field directly
                Self::fallback_on_data(&data).map_err(Some)
            }
            Self::Protocol(msg) => {
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
            _ => Self::fallback_on_data(&data),
        }
    }

    fn fallback_on_data(data: &serde_json::Value) -> Result<BinanceEvent, serde_json::Error> {
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
            && let Ok(book_ticker) = serde_json::from_value::<BookTickerEvent>(data.clone())
        {
            return Ok(BinanceEvent::BookTicker(book_ticker));
        }
        Err(serde_json::Error::custom("Unable to parse data"))
    }
}

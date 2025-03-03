mod depth_update;
mod historical_data;
mod kline;
mod price;
mod ticker;
mod trade;

pub use depth_update::{DepthSnapshot, DepthUpdate, OfferData};
pub use kline::KlineEventData;
pub use price::AveragePrice;
pub use ticker::{BookTickerEvent, MiniTickerData, TickerData, WindowTickerData};
pub use trade::{AggregateTrade, TradeEventData};

#[derive(Debug)]
pub enum BinanceEvent {
    Trade(TradeEventData),
    AggTrade(AggregateTrade),
    Kline(KlineEventData),
    AvgPrice(AveragePrice),
    DepthUpdate(DepthUpdate),
    BookTicker(BookTickerEvent),
    MiniTicker(MiniTickerData),
    Ticker(TickerData),
    WindowTicker(WindowTickerData),
}

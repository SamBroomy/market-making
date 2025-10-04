use std::fmt::Debug;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    models::market_data::{
        AggregateTrade, DepthSnapshot, DepthUpdate, TickerData, WindowTickerData,
    },
    settings::ExchangeSettings,
};

pub trait ExchangeConfig: Send + Sync + Debug {
    fn from_settings(settings: ExchangeSettings) -> Self;
    fn rest_url(&self) -> &str;
    fn ws_url(&self) -> &str;
}

#[async_trait]
pub trait ExchangeDataProvider: Clone + Send + Sync + 'static {
    type Config: ExchangeConfig;
    async fn from_config(config: Self::Config) -> Result<Self>
    where
        Self: Sized;
    async fn depth_snapshot(&self, symbol: String, limit: Option<i32>) -> Result<DepthSnapshot>;
    async fn agg_trade(&self, symbol: String) -> Result<UnboundedReceiver<AggregateTrade>>;
    // async fn trades(&self, params: TradeParams) -> Result<UnboundedReceiver<TradeEventData>>;
    async fn diff_book_depth(
        &self,
        symbol: String,
        update_speed: Option<String>,
    ) -> Result<UnboundedReceiver<DepthUpdate>>;
    async fn ticker(&self, symbol: String) -> Result<UnboundedReceiver<TickerData>>;
    async fn rolling_window_ticker(
        &self,
        symbol: String,
        window_size: Option<String>,
    ) -> Result<UnboundedReceiver<WindowTickerData>>;
    async fn disconnect(&self) -> Result<()>;
}

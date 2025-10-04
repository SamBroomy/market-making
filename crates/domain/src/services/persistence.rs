use std::fmt::Debug;

use anyhow::Result;
use async_trait::async_trait;
use secrecy::SecretString;

use crate::models::{
    market_data::{
        AggregateTrade, DepthSnapshot, DepthUpdate, MarketDataSummary, TickerData, TradeSummary,
        WindowTickerData,
    },
    order_book::StateSnapshot,
};

pub trait DataWriterConfig: Send + Sync + Debug {
    #[must_use]
    fn connection_string(&self) -> SecretString;
}

#[async_trait]
pub trait MarketDataWriter: Send + Sync {
    async fn from_config(config: &dyn DataWriterConfig) -> Result<Self>
    where
        Self: Sized;
    async fn write_depth_update(&self, update: &DepthUpdate) -> Result<()>;
    async fn write_depth_snapshot(
        &self,
        snapshot: &DepthSnapshot,
        symbol: &str,
        reason: &str,
    ) -> Result<()>;
    async fn write_ticker(&self, ticker: &TickerData) -> Result<()>;
    async fn write_window_ticker(&self, ticker: &WindowTickerData) -> Result<()>;
    async fn write_aggregate_trade(&self, trade: &AggregateTrade) -> Result<()>;
    async fn write_orderbook_state(&self, state: &StateSnapshot, symbol: &str) -> Result<()>;
    async fn write_orderbook_summary(
        &self,
        summary: &MarketDataSummary,
        symbol: &str,
    ) -> Result<()>;
    async fn write_trade_summary(&self, summary: &TradeSummary, symbol: &str) -> Result<()>;
    async fn disconnect(&self) -> Result<()>;
}

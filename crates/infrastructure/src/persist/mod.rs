mod timescale;

use std::sync::Arc;

use anyhow::Result;
use domain::{
    models::{
        market_data::{
            AggregateTrade, DepthSnapshot, DepthUpdate, MarketDataSummary, TickerData,
            TradeSummary, WindowTickerData,
        },
        order_book::StateSnapshot,
    },
    services::persistence::MarketDataWriter,
};
#[derive(Clone)]
pub struct DataWriter {
    writer: Arc<dyn MarketDataWriter>,
    name: String,
}

impl DataWriter {
    pub fn new(writer: Arc<dyn MarketDataWriter>, name: String) -> Self {
        Self { writer, name }
    }

    pub async fn write_depth_update(&self, update: &DepthUpdate) -> Result<()> {
        self.writer.write_depth_update(update).await
    }

    pub async fn write_depth_snapshot(
        &self,
        snapshot: &DepthSnapshot,
        symbol: &str,
        reason: &str,
    ) -> Result<()> {
        self.writer
            .write_depth_snapshot(snapshot, symbol, reason)
            .await
    }

    pub async fn write_ticker(&self, ticker: &TickerData) -> Result<()> {
        self.writer.write_ticker(ticker).await
    }

    pub async fn write_window_ticker(&self, ticker: &WindowTickerData) -> Result<()> {
        self.writer.write_window_ticker(ticker).await
    }

    pub async fn write_aggregate_trade(&self, trade: &AggregateTrade) -> Result<()> {
        self.writer.write_aggregate_trade(trade).await
    }

    pub async fn write_orderbook_state(&self, state: &StateSnapshot, symbol: &str) -> Result<()> {
        self.writer.write_orderbook_state(state, symbol).await
    }

    pub async fn write_orderbook_summary(
        &self,
        summary: &MarketDataSummary,
        symbol: &str,
    ) -> Result<()> {
        self.writer.write_orderbook_summary(summary, symbol).await
    }

    pub async fn write_trade_summary(&self, summary: &TradeSummary, symbol: &str) -> Result<()> {
        self.writer.write_trade_summary(summary, symbol).await
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.writer.disconnect().await
    }
}

#[must_use]
pub async fn get_data_writer(writer: &str) -> DataWriter {
    match writer.to_lowercase().as_str() {
        "timescale" => {
            let writer = timescale::get_writer().await;
            DataWriter::new(Arc::new(writer), "timescale".to_string())
        }
        _ => panic!("Unsupported message producer: {writer}"),
    }
}

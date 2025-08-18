use anyhow::Result;
use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::error;

use crate::data::binance::models::{
    AggregateTrade, DepthSnapshot, DepthUpdate, TickerData, WindowTickerData,
};

/// Reusable database writer for market data
#[derive(Clone)]
pub struct DatabaseWriter {
    pool: PgPool,
}

impl DatabaseWriter {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn write_depth_update(&self, update: &DepthUpdate) -> Result<()> {
        if let Err(e) = sqlx::query!(
            r"INSERT INTO depth_updates (event_time, symbol, first_update_id, final_update_id, bids, asks)
            VALUES ($1, $2, $3, $4, $5, $6)",
            update.event_time,
            update.symbol,
            Decimal::from(update.first_update_id),
            Decimal::from(update.final_update_id),
            serde_json::to_value(&update.bids)?,
            serde_json::to_value(&update.asks)?,
        )
        .execute(&self.pool)
        .await {
            error!("Failed to insert depth update for {}: {}", update.symbol, e);
            return Err(e.into());
        }
        Ok(())
    }

    pub async fn write_depth_snapshot(
        &self,
        snapshot: &DepthSnapshot,
        symbol: &str,
        reason: &str,
    ) -> Result<()> {
        if let Err(e) = sqlx::query!(
            r"INSERT INTO depth_snapshots (
                event_time, symbol, last_update_id, bids, asks, snapshot_reason
            ) VALUES ($1, $2, $3, $4, $5, $6)",
            Utc::now(),
            symbol,
            Decimal::from(snapshot.last_update_id),
            serde_json::to_value(&snapshot.bids)?,
            serde_json::to_value(&snapshot.asks)?,
            reason,
        )
        .execute(&self.pool)
        .await
        {
            error!("Failed to insert depth snapshot for {}: {}", symbol, e);
            return Err(e.into());
        }
        Ok(())
    }

    pub async fn write_ticker(&self, ticker: &TickerData) -> Result<()> {
        if let Err(e) = sqlx::query!(
            r"INSERT INTO ticker_data (
                event_time, symbol, price_change, price_change_percent, weighted_avg_price,
                first_trade_price, last_price, last_quantity, best_bid_price, best_bid_quantity,
                best_ask_price, best_ask_quantity, open_price, high_price, low_price,
                volume, quote_volume, open_time, close_time, first_trade_id,
                last_trade_id, trade_count
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)",
            ticker.event_time,
            ticker.symbol,
            ticker.price_change,
            ticker.price_change_percent,
            ticker.weighted_avg_price,
            ticker.first_trade_price,
            ticker.last_price,
            ticker.last_quantity,
            ticker.best_bid_price,
            ticker.best_bid_quantity,
            ticker.best_ask_price,
            ticker.best_ask_quantity,
            ticker.open_price,
            ticker.high_price,
            ticker.low_price,
            ticker.volume,
            ticker.quote_volume,
            ticker.open_time,
            ticker.close_time,
            Decimal::from(ticker.first_trade_id),
            Decimal::from(ticker.last_trade_id),
            Decimal::from(ticker.trade_count),
        )
        .execute(&self.pool)
        .await {
            error!("Failed to insert ticker data for {}: {}", ticker.symbol, e);
            return Err(e.into());
        }
        Ok(())
    }

    pub async fn write_window_ticker(&self, ticker: &WindowTickerData) -> Result<()> {
        if let Err(e) = sqlx::query!(
            r"INSERT INTO rolling_window_ticker (
                event_type, event_time, symbol, price_change, price_change_percent,
                open_price, high_price, low_price, close_price, weighted_avg_price,
                volume, quote_volume, open_time, close_time, first_trade_id,
                last_trade_id, trade_count
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            ticker.event_type,
            ticker.event_time,
            ticker.symbol,
            ticker.price_change,
            ticker.price_change_percent,
            ticker.open_price,
            ticker.high_price,
            ticker.low_price,
            ticker.close_price,
            ticker.weighted_avg_price,
            ticker.volume,
            ticker.quote_volume,
            ticker.open_time,
            ticker.close_time,
            Decimal::from(ticker.first_trade_id),
            Decimal::from(ticker.last_trade_id),
            Decimal::from(ticker.trade_count),
        )
        .execute(&self.pool)
        .await
        {
            error!(
                "Failed to insert window ticker data for {}: {}",
                ticker.symbol, e
            );
            return Err(e.into());
        }
        Ok(())
    }

    pub async fn write_aggregate_trade(&self, trade: &AggregateTrade) -> Result<()> {
        if let Err(e) = sqlx::query!(
            r"INSERT INTO aggregate_trades (
                event_time, symbol, aggregate_trade_id, price, quantity,
                first_trade_id, last_trade_id, trade_time, buyer_market_maker
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            trade.event_time,
            trade.symbol,
            Decimal::from(trade.aggregate_trade_id),
            trade.price,
            trade.quantity,
            Decimal::from(trade.first_trade_id),
            Decimal::from(trade.last_trade_id),
            trade.trade_time,
            trade.buyer_market_maker,
        )
        .execute(&self.pool)
        .await
        {
            error!(
                "Failed to insert aggregate trade for {}: {}",
                trade.symbol, e
            );
            return Err(e.into());
        }
        Ok(())
    }
}

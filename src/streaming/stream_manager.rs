use anyhow::{Context, Result};
use binance_sdk::spot::websocket_streams::{
    AggTradeParams, DiffBookDepthParams, RollingWindowTickerParams,
    RollingWindowTickerWindowSizeEnum, TickerParams,
};
use futures_util::future::try_join_all;
use iggy::clients::client::IggyClient;
use sqlx::PgPool;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{error, info};

use super::{BinanceStream, DatabaseWriter, MessageProducer};
use crate::{
    data::binance::{
        BinanceClient,
        models::{AggregateTrade, DepthUpdate, TickerData, WindowTickerData},
    },
    streaming::binance_stream::{DataHandler, DefaultDataHandler, DepthUpdateHandler},
};

/// Manages all streaming tasks for a symbol
pub struct StreamManager {
    symbol: String,
    iggy_client: &'static IggyClient,
    database_writer: DatabaseWriter,
    binance_client: &'static BinanceClient,
}

impl StreamManager {
    pub fn new(
        symbol: String,
        iggy_client: &'static IggyClient,
        pool: PgPool,
        binance_client: &'static BinanceClient,
    ) -> Self {
        let database_writer = DatabaseWriter::new(pool);

        Self {
            symbol,
            iggy_client,
            database_writer,
            binance_client,
        }
    }

    /// Start all streaming tasks for this symbol
    pub async fn start_all_streams(
        &self,
        orderbook_sender: Option<mpsc::UnboundedSender<DepthUpdate>>,
    ) -> Result<()> {
        info!("Starting all streams for symbol: {}", self.symbol);

        let mut tasks = Vec::new();

        // Start depth stream
        let depth_task = self.start_depth_stream(orderbook_sender).await?;
        tasks.push(depth_task);

        // Start ticker stream
        let ticker_task = self.start_ticker_stream().await?;
        tasks.push(ticker_task);

        // Start window ticker stream
        let window_ticker_task = self.start_window_ticker_stream().await?;
        tasks.push(window_ticker_task);

        // Start aggregate trade stream
        let agg_trade_task = self.start_agg_trade_stream().await?;
        tasks.push(agg_trade_task);

        // Wait for all tasks
        let results = try_join_all(tasks).await?;

        for result in results {
            if let Err(e) = result {
                error!("Stream task failed for {}: {}", self.symbol, e);
            }
        }

        info!("All streams completed for symbol: {}", self.symbol);
        Ok(())
    }

    async fn start_depth_stream(
        &self,
        orderbook_sender: Option<mpsc::UnboundedSender<DepthUpdate>>,
    ) -> Result<JoinHandle<Result<()>>> {
        let message_producer =
            MessageProducer::new(self.iggy_client, "diff_book_depth", &self.symbol, 1).await?;

        let default_handler =
            DefaultDataHandler::new(message_producer, self.database_writer.clone());
        let handler = DepthUpdateHandler::new(default_handler, orderbook_sender);

        let stream =
            BinanceStream::new(self.symbol.clone(), "depth".to_string(), Box::new(handler));

        let depth_rx = self
            .binance_client
            .diff_book_depth(
                DiffBookDepthParams::builder(self.symbol.clone())
                    .update_speed("100ms".to_string())
                    .build()?,
            )
            .await
            .context("Failed to get depth stream")?;

        let symbol = self.symbol.clone();
        Ok(tokio::spawn(async move {
            stream
                .run(depth_rx)
                .await
                .with_context(|| format!("Depth stream failed for {symbol}"))
        }))
    }

    async fn start_ticker_stream(&self) -> Result<JoinHandle<Result<()>>> {
        let message_producer =
            MessageProducer::new(self.iggy_client, "ticker", &self.symbol, 1).await?;

        let handler = DefaultDataHandler::new(message_producer, self.database_writer.clone());
        let boxed_handler: Box<dyn DataHandler<TickerData>> = Box::new(handler);
        let stream = BinanceStream::new(self.symbol.clone(), "ticker".to_string(), boxed_handler);

        let ticker_rx = self
            .binance_client
            .ticker(TickerParams::builder(self.symbol.clone()).build()?)
            .await
            .context("Failed to get ticker stream")?;

        let symbol = self.symbol.clone();
        Ok(tokio::spawn(async move {
            stream
                .run(ticker_rx)
                .await
                .with_context(|| format!("Ticker stream failed for {symbol}"))
        }))
    }

    async fn start_window_ticker_stream(&self) -> Result<JoinHandle<Result<()>>> {
        let message_producer = MessageProducer::new(
            self.iggy_client,
            "rolling_window_ticker_1h",
            &self.symbol,
            1,
        )
        .await?;

        let handler = DefaultDataHandler::new(message_producer, self.database_writer.clone());
        let boxed_handler: Box<dyn DataHandler<WindowTickerData>> = Box::new(handler);
        let stream = BinanceStream::new(
            self.symbol.clone(),
            "window_ticker".to_string(),
            boxed_handler,
        );

        let window_rx = self
            .binance_client
            .rolling_window_ticker(
                RollingWindowTickerParams::builder(
                    self.symbol.clone(),
                    RollingWindowTickerWindowSizeEnum::WindowSize1h,
                )
                .build()?,
            )
            .await
            .context("Failed to get rolling window ticker stream")?;

        let symbol = self.symbol.clone();
        Ok(tokio::spawn(async move {
            stream
                .run(window_rx)
                .await
                .with_context(|| format!("Window ticker stream failed for {symbol}"))
        }))
    }

    async fn start_agg_trade_stream(&self) -> Result<JoinHandle<Result<()>>> {
        let message_producer =
            MessageProducer::new(self.iggy_client, "agg_trade", &self.symbol, 1).await?;

        let handler = DefaultDataHandler::new(message_producer, self.database_writer.clone());
        let boxed_handler: Box<dyn DataHandler<AggregateTrade>> = Box::new(handler);
        let stream =
            BinanceStream::new(self.symbol.clone(), "agg_trade".to_string(), boxed_handler);

        let agg_trade_rx = self
            .binance_client
            .agg_trade(AggTradeParams::builder(self.symbol.clone()).build()?)
            .await
            .context("Failed to get aggregate trade stream")?;

        let symbol = self.symbol.clone();
        Ok(tokio::spawn(async move {
            stream
                .run(agg_trade_rx)
                .await
                .with_context(|| format!("Aggregate trade stream failed for {symbol}"))
        }))
    }
}

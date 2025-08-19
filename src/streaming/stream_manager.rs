use std::sync::Arc;

use anyhow::{Context, Result};
use binance_sdk::spot::websocket_streams::{
    AggTradeParams, DiffBookDepthParams, RollingWindowTickerParams,
    RollingWindowTickerWindowSizeEnum, TickerParams,
};
use futures_util::future::try_join_all;
use iggy::clients::client::IggyClient;
use sqlx::PgPool;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{error, info, warn};

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
    binance_client: Arc<BinanceClient>, // Wrapped in Arc for sharing across tasks
    // Pre-created message producers to avoid connection churn
    depth_producer: Arc<MessageProducer>,
    ticker_producer: Arc<MessageProducer>,
    window_ticker_producer: Arc<MessageProducer>,
    agg_trade_producer: Arc<MessageProducer>,
}

impl StreamManager {
    pub async fn new(
        symbol: String,
        iggy_client: &'static IggyClient,
        pool: PgPool,
        binance_client: BinanceClient, // Now takes ownership
    ) -> Result<Self> {
        let database_writer = DatabaseWriter::new(pool);

        // Create all message producers once to avoid connection churn
        let depth_producer =
            Arc::new(MessageProducer::new(iggy_client, "diff_book_depth", &symbol, 1).await?);
        let ticker_producer =
            Arc::new(MessageProducer::new(iggy_client, "ticker", &symbol, 1).await?);
        let window_ticker_producer = Arc::new(
            MessageProducer::new(iggy_client, "rolling_window_ticker_1h", &symbol, 1).await?,
        );
        let agg_trade_producer =
            Arc::new(MessageProducer::new(iggy_client, "agg_trade", &symbol, 1).await?);

        Ok(Self {
            symbol,
            iggy_client,
            database_writer,
            binance_client: Arc::new(binance_client), // Wrap in Arc
            depth_producer,
            ticker_producer,
            window_ticker_producer,
            agg_trade_producer,
        })
    }

    /// Start all streaming tasks for this symbol
    pub async fn start_all_streams(
        &self,
        orderbook_sender: Option<mpsc::UnboundedSender<DepthUpdate>>,
    ) -> Result<()> {
        info!("Starting all streams for symbol: {}", self.symbol);

        let mut tasks = Vec::new();

        // Start depth stream
        let depth_task = self.start_depth_stream(orderbook_sender);
        tasks.push(depth_task);

        // Start ticker stream
        let ticker_task = self.start_ticker_stream();
        tasks.push(ticker_task);

        // Start window ticker stream
        let window_ticker_task = self.start_window_ticker_stream();
        tasks.push(window_ticker_task);

        // Start aggregate trade stream
        let agg_trade_task = self.start_agg_trade_stream();
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

    fn start_depth_stream(
        &self,
        orderbook_sender: Option<mpsc::UnboundedSender<DepthUpdate>>,
    ) -> JoinHandle<Result<()>> {
        let default_handler = DefaultDataHandler::new(
            Arc::clone(&self.depth_producer),
            self.database_writer.clone(),
        );
        let handler = DepthUpdateHandler::new(default_handler, orderbook_sender);

        let stream =
            BinanceStream::new(self.symbol.clone(), "depth".to_string(), Box::new(handler));

        // Move BinanceClient call inside spawned task to ensure stream handler lifetime
        let bc = Arc::clone(&self.binance_client);
        let symbol = self.symbol.clone();
        tokio::spawn(async move {
            let depth_rx = bc
                .diff_book_depth(
                    DiffBookDepthParams::builder(symbol.clone())
                        .update_speed("100ms".to_string())
                        .build()?,
                )
                .await
                .context("Failed to get depth stream")?;

            stream
                .run(depth_rx)
                .await
                .with_context(|| format!("Depth stream failed for {symbol}"))
        })
    }

    fn start_ticker_stream(&self) -> JoinHandle<Result<()>> {
        let handler = DefaultDataHandler::new(
            Arc::clone(&self.ticker_producer),
            self.database_writer.clone(),
        );
        let boxed_handler: Box<dyn DataHandler<TickerData>> = Box::new(handler);
        let stream = BinanceStream::new(self.symbol.clone(), "ticker".to_string(), boxed_handler);

        // Move BinanceClient call inside spawned task to ensure stream handler lifetime
        let bc = Arc::clone(&self.binance_client);
        let symbol = self.symbol.clone();
        tokio::spawn(async move {
            let ticker_rx = bc
                .ticker(TickerParams::builder(symbol.clone()).build()?)
                .await
                .context("Failed to get ticker stream")?;

            stream
                .run(ticker_rx)
                .await
                .with_context(|| format!("Ticker stream failed for {symbol}"))
        })
    }

    fn start_window_ticker_stream(&self) -> JoinHandle<Result<()>> {
        let handler = DefaultDataHandler::new(
            Arc::clone(&self.window_ticker_producer),
            self.database_writer.clone(),
        );
        let boxed_handler: Box<dyn DataHandler<WindowTickerData>> = Box::new(handler);
        let stream = BinanceStream::new(
            self.symbol.clone(),
            "window_ticker".to_string(),
            boxed_handler,
        );

        // Move BinanceClient call inside spawned task to ensure stream handler lifetime
        let bc = Arc::clone(&self.binance_client);
        let symbol = self.symbol.clone();
        tokio::spawn(async move {
            let window_rx = bc
                .rolling_window_ticker(
                    RollingWindowTickerParams::builder(
                        symbol.clone(),
                        RollingWindowTickerWindowSizeEnum::WindowSize1h,
                    )
                    .build()?,
                )
                .await
                .context("Failed to get rolling window ticker stream")?;

            stream
                .run(window_rx)
                .await
                .with_context(|| format!("Window ticker stream failed for {symbol}"))
        })
    }

    fn start_agg_trade_stream(&self) -> JoinHandle<Result<()>> {
        let handler = DefaultDataHandler::new(
            Arc::clone(&self.agg_trade_producer),
            self.database_writer.clone(),
        );
        let boxed_handler: Box<dyn DataHandler<AggregateTrade>> = Box::new(handler);
        let stream =
            BinanceStream::new(self.symbol.clone(), "agg_trade".to_string(), boxed_handler);

        // Move BinanceClient call inside spawned task to ensure stream handler lifetime
        let bc = Arc::clone(&self.binance_client);
        let symbol = self.symbol.clone();
        tokio::spawn(async move {
            let agg_trade_rx = bc
                .agg_trade(AggTradeParams::builder(symbol.clone()).build()?)
                .await
                .context("Failed to get aggregate trade stream")?;

            stream
                .run(agg_trade_rx)
                .await
                .with_context(|| format!("Aggregate trade stream failed for {symbol}"))
        })
    }

    /// Shutdown the stream manager and close WebSocket connections
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down stream manager for symbol: {}", self.symbol);

        // Close the Binance WebSocket connections (this symbol's connection only)
        if let Err(e) = self.binance_client.disconnect().await {
            warn!(
                "WebSocket disconnect error for {}: {} (continuing)",
                self.symbol, e
            );
        } else {
            info!("WebSocket disconnected successfully for {}", self.symbol);
        }

        info!("Stream manager completed gracefully for {}", self.symbol);
        Ok(())
    }
}

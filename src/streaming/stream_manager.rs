use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use binance_sdk::spot::{
    rest_api::DepthParams,
    websocket_streams::{
        AggTradeParams, DiffBookDepthParams, RollingWindowTickerParams, TickerParams,
    },
};
use futures_util::future::try_join_all;
use iggy::clients::client::IggyClient;
use sqlx::PgPool;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};
use tracing::{debug, error, info, warn};

use super::{BinanceStream, DatabaseWriter, MessageProducer};
use crate::{
    book::order_book::{OrderBook, SnapshotRequest},
    data::binance::{
        BinanceClient,
        models::{AggregateTrade, TickerData, WindowTickerData},
    },
    settings::{BinanceSettings, OrderBookUpdateSpeed, ResolvedSymbolConfig, RollingWindowSize},
    shutdown::ShutdownCoordinator,
    streaming::binance_stream::{DataHandler, DefaultDataHandler, DepthUpdateHandler},
};

/// Manages all streaming tasks for a symbol
pub struct StreamManager;

impl StreamManager {
    pub async fn run(
        resolved_config: ResolvedSymbolConfig,
        binance_settings: BinanceSettings,
        iggy_client: &'static IggyClient,
        pool: PgPool,
        shutdown_coordinator: ShutdownCoordinator,
    ) -> Result<()> {
        let symbol = resolved_config.symbol;
        info!("Starting producer for symbol: {}", symbol);
        let bc = Arc::new(BinanceClient::new(&binance_settings).await);
        let database_writer = if resolved_config.persist {
            Some(DatabaseWriter::new(pool.clone()))
        } else {
            None
        };
        let mut tasks = Vec::new();

        if let Some(orderbook_config) = resolved_config.streams.orderbook {
            // Create channels for orderbook communication
            let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();
            let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();
            // Create message producers for orderbook signals and state if message queue enabled for this symbol
            let signals_producer = if resolved_config.message_queue {
                Some(MessageProducer::new(iggy_client, "orderbook_signals", &symbol, 1).await?)
            } else {
                None
            };
            let state_producer = if resolved_config.message_queue {
                Some(MessageProducer::new(iggy_client, "orderbook_state", &symbol, 1).await?)
            } else {
                None
            };
            let snapshot_shutdown_rx = shutdown_coordinator.subscribe();
            let snapshot_task =
                Self::run_snapshot_task(snapshot_request_rx, bc.clone(), snapshot_shutdown_rx);
            tasks.push(snapshot_task);
            let depth_producer = if resolved_config.message_queue {
                Some(MessageProducer::new(iggy_client, "diff_book_depth", &symbol, 1).await?)
            } else {
                None
            };

            let depth_handler = DepthUpdateHandler::new(
                DefaultDataHandler::new(depth_producer, database_writer.clone()),
                Some(depth_for_orderbook_tx),
            );

            let depth_task = Self::start_depth_stream(
                symbol.clone(),
                depth_handler,
                bc.clone(),
                orderbook_config.update_speed,
            );
            tasks.push(depth_task);
            let orderbook = OrderBook::new(
                symbol.clone(),
                Some(orderbook_config.snapshot_limit),
                depth_for_orderbook_rx,
                snapshot_request_tx,
                signals_producer,
                state_producer,
                database_writer.clone(),
            )
            .await?;

            let mut orderbook_shutdown_rx = shutdown_coordinator.subscribe();
            let symbol_ = symbol.clone();
            let orderbook_task = tokio::spawn(async move {
                tokio::select! {
                    result = orderbook.run() => {
                        result
                    }
                    _ = orderbook_shutdown_rx.recv() => {
                        info!("OrderBook received shutdown signal for {}", symbol_);
                        Ok(())
                    }
                }
            });
            tasks.push(orderbook_task);
        }

        if let Some(ticker_config) = resolved_config.streams.ticker {
            let ticker_producer = if resolved_config.message_queue {
                Some(MessageProducer::new(iggy_client, "ticker", &symbol, 1).await?)
            } else {
                None
            };
            let handler = DefaultDataHandler::new(ticker_producer, database_writer.clone());
            let ticker_task = Self::start_ticker_stream(symbol.clone(), handler, bc.clone());
            tasks.push(ticker_task);
        }

        if let Some(window_ticker_config) = resolved_config.streams.window {
            let window_ticker_producer = if resolved_config.message_queue {
                Some(
                    MessageProducer::new(
                        iggy_client,
                        &format!(
                            "rolling_window_ticker_{}",
                            window_ticker_config.rolling_window_size
                        ),
                        &symbol,
                        1,
                    )
                    .await?,
                )
            } else {
                None
            };
            let handler = DefaultDataHandler::new(window_ticker_producer, database_writer.clone());
            let window_ticker_task = Self::start_window_ticker_stream(
                symbol.clone(),
                handler,
                bc.clone(),
                window_ticker_config.rolling_window_size,
            );
            tasks.push(window_ticker_task);
        }

        if let Some(agg_trade_config) = resolved_config.streams.agg_trade {
            let agg_trade_producer = if resolved_config.message_queue {
                Some(MessageProducer::new(iggy_client, "agg_trade", &symbol, 1).await?)
            } else {
                None
            };
            let handler = DefaultDataHandler::new(agg_trade_producer, database_writer.clone());
            let agg_trade_task = Self::start_agg_trade_stream(symbol.clone(), handler, bc.clone());
            tasks.push(agg_trade_task);
        }

        if tasks.is_empty() {
            warn!("No streams enabled for symbol: {}", symbol);
            return Ok(());
        }
        let mut shutdown_rx = shutdown_coordinator.subscribe();
        // Wait for all tasks
        tokio::select! {
            // Handle snapshot requests
            results = try_join_all(tasks) => {
                match results {
                    Ok(results) => {
                        for (i, result) in results.into_iter().enumerate() {
                            if let Err(e) = result {
                                error!("Stream task failed for {}: {}", symbol, e);
                            }
                        }
                        info!("All stream tasks completed for symbol: {}", symbol);
                    },
                    Err(e) => {
                        error!("Failed to join stream tasks for {}: {}", symbol, e);
                    return Err(e.into());
                    },
                }
            }
            _ = shutdown_rx.recv() => {
                info!("StreamManager received shutdown signal for symbol: {}", symbol);

                tokio::time::sleep(Duration::from_millis(100)).await;

                if let Err(e) = bc.disconnect().await {
                                   warn!("Failed to disconnect BinanceClient for {}: {}", symbol, e);
                               } else {
                                   info!("BinanceClient disconnected successfully for {}", symbol);
                               }
                               info!("StreamManager shutdown completed for {}", symbol);
            }
        }

        info!("All enabled streams completed for symbol: {}", symbol);
        Ok(())
    }

    fn run_snapshot_task(
        mut snapshot_rx: mpsc::UnboundedReceiver<SnapshotRequest>,
        bc: Arc<BinanceClient>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Handle snapshot requests
                    request = snapshot_rx.recv() => {
                        if let Some(SnapshotRequest {
                                symbol: req_symbol,
                                limit,
                                response_tx,
                                reason,
                            }) = request {
                            let snapshot_response = bc
                                .depth_snapshot(
                                    DepthParams::builder(req_symbol.clone())
                                        .limit(limit)
                                        .build()?,
                                )
                                .await?;


                            debug!("Snapshot requested for {} (limit: {:?})", req_symbol, limit);

                            let snapshot_result = snapshot_response
                                .data()
                                .await
                                .context("Failed to get depth snapshot");
                            let _ = response_tx.send(snapshot_result);

                            // Send response
                        } else {
                            info!("Snapshot request channel closed");
                            break;
                        }
                    }
                    // Handle shutdown signal
                    _ = shutdown_rx.recv() => {
                        info!("Snapshot task received shutdown signal");
                        // Clean shutdown of this task's BinanceClient
                        if let Err(e) = bc.disconnect().await {
                            warn!("Failed to disconnect snapshot BinanceClient: {}", e);
                        }
                        break;
                    }
                }
            }

            info!("Snapshot task completed gracefully");
            Ok(())
        })
    }

    fn start_depth_stream(
        symbol: String,
        handler: DepthUpdateHandler,
        bc: Arc<BinanceClient>,
        update_speed: OrderBookUpdateSpeed,
    ) -> JoinHandle<Result<()>> {
        let stream = BinanceStream::new(&symbol, "depth", Box::new(handler));

        tokio::spawn(async move {
            let symbol = symbol.clone();
            let depth_rx = bc
                .diff_book_depth(
                    DiffBookDepthParams::builder(symbol.clone())
                        .update_speed(update_speed.to_string())
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

    fn start_ticker_stream(
        symbol: String,
        handler: DefaultDataHandler,
        bc: Arc<BinanceClient>,
    ) -> JoinHandle<Result<()>> {
        let boxed_handler: Box<dyn DataHandler<TickerData>> = Box::new(handler);
        let stream = BinanceStream::new(&symbol, "ticker", boxed_handler);

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

    fn start_window_ticker_stream(
        symbol: String,
        handler: DefaultDataHandler,
        bc: Arc<BinanceClient>,
        rolling_window_size: RollingWindowSize,
    ) -> JoinHandle<Result<()>> {
        let boxed_handler: Box<dyn DataHandler<WindowTickerData>> = Box::new(handler);
        let stream = BinanceStream::new(&symbol, "window_ticker".to_string(), boxed_handler);

        tokio::spawn(async move {
            let window_rx = bc
                .rolling_window_ticker(
                    RollingWindowTickerParams::builder(symbol.clone(), rolling_window_size.into())
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

    fn start_agg_trade_stream(
        symbol: String,
        handler: DefaultDataHandler,
        bc: Arc<BinanceClient>,
    ) -> JoinHandle<Result<()>> {
        let boxed_handler: Box<dyn DataHandler<AggregateTrade>> = Box::new(handler);
        let stream = BinanceStream::new(&symbol, "agg_trade".to_string(), boxed_handler);

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
}

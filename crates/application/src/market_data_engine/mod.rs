use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use domain::{
    models::market_data::{AggregateTrade, DepthUpdate},
    services::exchange::ExchangeDataProvider,
    settings::{
        Settings,
        trading::{OrderBookUpdateSpeed, ResolvedPairConfig, RollingWindowSize},
    },
};
use futures_util::future::try_join_all;
use infrastructure::{
    data_providers::get_provider,
    messaging::{PublisherFactory, StreamProducer, get_stream_producer},
    persist::{DataWriter, get_data_writer},
};
use support::shutdown::ShutdownCoordinator;
use tokio::{
    sync::{
        broadcast,
        mpsc::{self, UnboundedSender},
    },
    task::JoinHandle,
};
use tracing::{debug, error, info, warn};

use crate::handlers::{
    order_book::{OrderBookProcessor, SnapshotRequest},
    process::{
        process_aggregate_trade_updates, process_depth_updates, process_ticker_updates,
        process_window_ticker_updates,
    },
    trade::TradeProcessor,
};

mod stream_config;
use stream_config::StreamConfig;

pub struct MarketDataTicker;

impl MarketDataTicker {
    /// Helper to create a stream producer if message queue is enabled
    async fn create_producer<P: PublisherFactory>(
        publisher_factory: &P,
        stream_name: &str,
        symbol: &str,
        enabled: bool,
    ) -> Result<Option<StreamProducer>> {
        if enabled {
            Ok(Some(
                publisher_factory
                    .create_producer(stream_name, symbol)
                    .await?,
            ))
        } else {
            Ok(None)
        }
    }

    /// Helper to create a `StreamConfig` for a given stream
    async fn create_stream_config<P: PublisherFactory>(
        publisher_factory: &P,
        stream_name: &str,
        symbol: &str,
        message_queue_enabled: bool,
        writer: Option<DataWriter>,
    ) -> Result<StreamConfig> {
        let producer = Self::create_producer(
            publisher_factory,
            stream_name,
            symbol,
            message_queue_enabled,
        )
        .await?;
        Ok(StreamConfig::new(producer, writer))
    }

    pub async fn run<T: ExchangeDataProvider, P: PublisherFactory>(
        resolved_config: ResolvedPairConfig,
        exchange_data_provider: Arc<T>,
        publisher_factory: P,
        data_writer: DataWriter,
        shutdown_coordinator: ShutdownCoordinator,
    ) -> Result<()> {
        let symbol = resolved_config.symbol.clone();
        info!(
            symbol = %symbol,
            message_queue = resolved_config.message_queue,
            persist = resolved_config.persist,
            "Starting market data engine"
        );

        let data_writer = if resolved_config.persist {
            Some(data_writer)
        } else {
            None
        };

        let mut tasks = Vec::new();

        if let Some(orderbook_config) = resolved_config.streams.orderbook {
            // Create channels for orderbook communication
            let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();
            let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();

            // Create message producers for orderbook
            let signals_producer = Self::create_producer(
                &publisher_factory,
                "orderbook_signals",
                &symbol,
                resolved_config.message_queue,
            )
            .await?;

            let state_producer = Self::create_producer(
                &publisher_factory,
                "orderbook_state",
                &symbol,
                resolved_config.message_queue,
            )
            .await?;

            let snapshot_shutdown_rx = shutdown_coordinator.subscribe();
            let snapshot_task = Self::run_snapshot_task(
                snapshot_request_rx,
                Arc::clone(&exchange_data_provider),
                snapshot_shutdown_rx,
            );
            tasks.push(snapshot_task);

            // Create depth stream config
            let depth_config = Self::create_stream_config(
                &publisher_factory,
                "diff_book_depth",
                &symbol,
                resolved_config.message_queue,
                data_writer.clone(),
            )
            .await?;

            let (depth_producer, depth_writer) = depth_config.split();
            let depth_task = Self::start_depth_stream(
                symbol.clone(),
                orderbook_config.update_speed,
                Arc::clone(&exchange_data_provider),
                depth_producer,
                depth_writer,
                Some(depth_for_orderbook_tx),
            );
            tasks.push(depth_task);
            let orderbook = OrderBookProcessor::new(
                symbol.clone(),
                Some(orderbook_config.snapshot_limit),
                depth_for_orderbook_rx,
                snapshot_request_tx,
                signals_producer,
                state_producer,
                data_writer.clone(),
                orderbook_config.publish_interval,
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

        if let Some(_ticker_config) = resolved_config.streams.ticker {
            let ticker_config = Self::create_stream_config(
                &publisher_factory,
                "ticker",
                &symbol,
                resolved_config.message_queue,
                data_writer.clone(),
            )
            .await?;

            let (ticker_producer, ticker_writer) = ticker_config.split();
            let ticker_task = Self::start_ticker_stream(
                symbol.clone(),
                Arc::clone(&exchange_data_provider),
                ticker_producer,
                ticker_writer,
            );
            tasks.push(ticker_task);
        }

        if let Some(window_ticker_config) = resolved_config.streams.window {
            let stream_name = format!(
                "rolling_window_ticker_{}",
                window_ticker_config.rolling_window_size
            );
            let window_config = Self::create_stream_config(
                &publisher_factory,
                &stream_name,
                &symbol,
                resolved_config.message_queue,
                data_writer.clone(),
            )
            .await?;

            let (window_producer, window_writer) = window_config.split();
            let window_ticker_task = Self::start_window_ticker_stream(
                symbol.clone(),
                window_ticker_config.rolling_window_size,
                Arc::clone(&exchange_data_provider),
                window_producer,
                window_writer,
            );
            tasks.push(window_ticker_task);
        }

        if let Some(agg_trade_config) = resolved_config.streams.agg_trade {
            // Create channels for TradeProcessor communication
            let (trade_for_processor_tx, trade_for_processor_rx) = mpsc::unbounded_channel();

            // Create message producer for trade summaries
            let summary_stream_name = format!(
                "agg_trade_summary_{}s",
                agg_trade_config.window_duration.as_secs()
            );
            let summary_producer = Self::create_producer(
                &publisher_factory,
                &summary_stream_name,
                &symbol,
                resolved_config.message_queue,
            )
            .await?;

            // Create and spawn TradeProcessor
            let trade_processor = TradeProcessor::new(
                symbol.clone(),
                trade_for_processor_rx,
                agg_trade_config.window_duration,
                agg_trade_config.publish_interval,
                summary_producer,
                data_writer.clone(),
            );

            let mut processor_shutdown_rx = shutdown_coordinator.subscribe();
            let symbol_clone = symbol.clone();
            let processor_task = tokio::spawn(async move {
                tokio::select! {
                    result = trade_processor.run() => {
                        result
                    }
                    _ = processor_shutdown_rx.recv() => {
                        info!(
                            symbol = %symbol_clone,
                            "Trade processor received shutdown signal"
                        );
                        Ok(())
                    }
                }
            });
            tasks.push(processor_task);

            // Create agg trade stream config
            let agg_trade_config = Self::create_stream_config(
                &publisher_factory,
                "agg_trade",
                &symbol,
                resolved_config.message_queue,
                data_writer.clone(),
            )
            .await?;

            let (agg_trade_producer, agg_trade_writer) = agg_trade_config.split();
            let agg_trade_task = Self::start_agg_trade_stream(
                symbol.clone(),
                Arc::clone(&exchange_data_provider),
                agg_trade_producer,
                agg_trade_writer,
                Some(trade_for_processor_tx),
            );
            tasks.push(agg_trade_task);
        }

        if tasks.is_empty() {
            warn!(symbol = %symbol, "No streams enabled");
            return Ok(());
        }
        let mut shutdown_rx = shutdown_coordinator.subscribe();

        tokio::select! {
            results = try_join_all(tasks) => {
                match results {
                    Ok(results) => {
                        let mut had_errors = false;
                        for (i, result) in results.into_iter().enumerate() {
                            if let Err(e) = result {
                                error!(symbol = %symbol, task = i, error = %e, "Stream task failed");
                                had_errors = true;
                            }
                        }
                        if had_errors {
                            warn!(symbol = %symbol, "Some stream tasks failed");
                        } else {
                            info!(symbol = %symbol, "All stream tasks completed successfully");
                        }
                    },
                    Err(e) => {
                        error!(symbol = %symbol, error = %e, "Failed to join stream tasks");
                        return Err(e.into());
                    },
                }
            }
            _ = shutdown_rx.recv() => {
                info!(symbol = %symbol, "Received shutdown signal, streams will terminate");
                // Give tasks a moment to finish their current operations
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        info!(symbol = %symbol, "Market data engine completed");
        Ok(())
    }

    fn run_snapshot_task<T: ExchangeDataProvider>(
        mut snapshot_rx: mpsc::UnboundedReceiver<SnapshotRequest>,
        exchange_data_provider: Arc<T>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    request = snapshot_rx.recv() => {
                        if let Some(SnapshotRequest {
                                symbol: req_symbol,
                                limit,
                                response_tx,
                                reason,
                            }) = request {
                            debug!(
                                symbol = %req_symbol,
                                limit = ?limit,
                                reason = %reason.as_str(),
                                "Processing snapshot request"
                            );

                            let snapshot_response = exchange_data_provider
                                .depth_snapshot(req_symbol.clone(), limit)
                                .await;

                            if let Err(snapshot) = &snapshot_response {
                                error!(
                                    symbol = %req_symbol,
                                    reason = %reason.as_str(),
                                    error = %snapshot,
                                    "Snapshot request failed"
                                );
                            }

                            if response_tx.send(snapshot_response).is_err() {
                                warn!(
                                    symbol = %req_symbol,
                                    "Failed to send snapshot response - receiver dropped"
                                );
                            }
                        } else {
                            info!("Snapshot request channel closed");
                            break;
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Snapshot task received shutdown signal");
                        break;
                    }
                }
            }

            info!("Snapshot task completed gracefully");
            Ok(())
        })
    }

    fn start_depth_stream<T: ExchangeDataProvider>(
        symbol: String,
        update_speed: OrderBookUpdateSpeed,
        exchange_data_provider: Arc<T>,
        producer: Option<StreamProducer>,
        writer: Option<DataWriter>,
        orderbook_tx: Option<UnboundedSender<DepthUpdate>>,
    ) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            info!(
                symbol = %symbol,
                update_speed = %update_speed,
                "Starting depth stream task"
            );
            let depth_rx = exchange_data_provider
                .diff_book_depth(symbol.clone(), Some(update_speed.to_string()))
                .await
                .context("Failed to get depth stream")?;
            info!(symbol = %symbol, "Depth stream established");

            let result = process_depth_updates(depth_rx, producer, writer, orderbook_tx).await;

            match &result {
                Ok(()) => info!(symbol = %symbol, "Depth stream task completed successfully"),
                Err(e) => error!(symbol = %symbol, error = %e, "Depth stream task failed"),
            }

            result.with_context(|| format!("Depth stream failed for {symbol}"))
        })
    }

    fn start_ticker_stream<T: ExchangeDataProvider>(
        symbol: String,
        exchange_data_provider: Arc<T>,
        producer: Option<StreamProducer>,
        writer: Option<DataWriter>,
    ) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            info!("Starting ticker stream for symbol: {}", symbol);
            let ticker_rx = exchange_data_provider
                .ticker(symbol.clone())
                .await
                .context("Failed to get ticker stream")?;
            info!("Ticker stream established for symbol: {}", symbol);

            process_ticker_updates(ticker_rx, producer, writer)
                .await
                .with_context(|| format!("Ticker stream failed for {symbol}"))
        })
    }

    fn start_window_ticker_stream<T: ExchangeDataProvider>(
        symbol: String,
        rolling_window_size: RollingWindowSize,
        exchange_data_provider: Arc<T>,
        producer: Option<StreamProducer>,
        writer: Option<DataWriter>,
    ) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            let window_rx = exchange_data_provider
                .rolling_window_ticker(symbol.clone(), Some(rolling_window_size.to_string()))
                .await
                .context("Failed to get rolling window ticker stream")?;

            process_window_ticker_updates(window_rx, producer, writer)
                .await
                .with_context(|| format!("Window ticker stream failed for {symbol}"))
        })
    }

    fn start_agg_trade_stream<T: ExchangeDataProvider>(
        symbol: String,
        exchange_data_provider: Arc<T>,
        producer: Option<StreamProducer>,
        writer: Option<DataWriter>,
        trade_tx: Option<UnboundedSender<AggregateTrade>>,
    ) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            let agg_trade_rx = exchange_data_provider
                .agg_trade(symbol.clone())
                .await
                .context("Failed to get aggregate trade stream")?;

            process_aggregate_trade_updates(agg_trade_rx, producer, writer, trade_tx)
                .await
                .with_context(|| format!("Aggregate trade stream failed for {symbol}"))
        })
    }
}

/// Run producers for multiple market pairs concurrently with graceful shutdown
pub async fn run_multi_market_producer(
    settings: Settings,
    shutdown_coordinator: ShutdownCoordinator,
) -> Result<()> {
    let pair_configs = settings.trading.get_pair_configs();
    let delay = settings.exchange.startup_delay;
    let mut tasks = Vec::with_capacity(pair_configs.len());
    let market_data_provider = Arc::new(get_provider(settings.exchange).await);
    let publisher_factory = get_stream_producer("iggy").await;
    let data_writer = get_data_writer("timescale").await;

    for (index, pair_cfg) in pair_configs.into_iter().enumerate() {
        // check for shutdown before starting new pairs
        if shutdown_coordinator.is_shutting_down() {
            info!("Shutdown detected, not starting remaining pairs");
            break;
        }
        let symbol = &pair_cfg.symbol.clone();

        // stagger startup to avoid overwhelming services with configurable delay
        if index > 0 {
            info!(
                symbol = %symbol,
                delay_secs = delay.as_secs(),
                "Waiting before starting producer (rate limiting)"
            );

            // allow shutdown during startup delay
            let mut delay_shutdown_rx = shutdown_coordinator.subscribe();
            tokio::select! {
                () = tokio::time::sleep(delay) => {},
                _ = delay_shutdown_rx.recv() => {
                    info!("Shutdown received during startup delay");
                    break;
                }
            }
        }

        info!(symbol = %symbol, "Starting producer");
        let shutdown_coordinator_ = shutdown_coordinator.clone();
        let symbol_ = symbol.clone();
        let market_data_provider_ = market_data_provider.clone();
        let publisher_factory_ = publisher_factory.clone();
        let data_writer_ = data_writer.clone();
        let task = tokio::spawn(async move {
            // Create a new BinanceClient for this pair

            // Start all streams with built-in shutdown handling
            MarketDataTicker::run(
                pair_cfg,
                market_data_provider_,
                publisher_factory_,
                data_writer_,
                shutdown_coordinator_,
            )
            .await?;

            info!(symbol = %symbol_, "Producer finished");
            Ok(())
        });
        tasks.push(task);
    }

    info!(
        "All {} market pairs producers have been spawned",
        tasks.len()
    );

    // Wait for all tasks to complete or shutdown signal
    let mut main_shutdown_rx = shutdown_coordinator.subscribe();

    tokio::select! {
        results = try_join_all(tasks) => {
            match results {
                Ok(task_results) => {
                    // Check individual task results
                    let mut errors = Vec::new();
                    for (i, result) in task_results.into_iter().enumerate() {
                        match result {
                            Ok(()) => info!(
                                producer_index = i,
                                "Pair producer completed successfully"
                            ),
                            Err(e) => {
                                error!(
                                    producer_index = i,
                                    error = %e,
                                    "Pair producer failed"
                                );
                                errors.push(e);
                            }
                        }
                    }

                    if errors.is_empty() {
                        info!("All pair producers completed successfully");
                    } else {
                        error!(
                            failed_count = errors.len(),
                            "Some pair producers failed"
                        );
                        return Err(errors.into_iter().next().unwrap());
                    }
                }
                Err(e) => {
                    error!(error = %e, "Task join error");
                    return Err(e.into());
                }
            }
        }
        _ = main_shutdown_rx.recv() => {
            info!("Multi-pair producer received shutdown signal");
            info!("Pair producers will shut down via their individual shutdown signals");
        }
    }

    info!("Multi-pair producer finished");

    shutdown_global_resources(market_data_provider, publisher_factory, data_writer).await;
    Ok(())
}

/// Gracefully shutdown all global resources with timeout
pub async fn shutdown_global_resources(
    market_data_provider: Arc<impl ExchangeDataProvider>,
    publisher_factory: impl PublisherFactory,
    data_writer: DataWriter,
) {
    info!("Starting graceful shutdown of global resources...");

    // Define timeout for resource cleanup
    let shutdown_timeout = Duration::from_secs(10);

    // Shutdown Iggy client if it was initialized

    match tokio::time::timeout(shutdown_timeout, publisher_factory.disconnect()).await {
        Ok(Ok(())) => {
            info!("Publisher factory disconnected successfully");
        }
        Ok(Err(e)) => {
            error!(error = %e, "Failed to disconnect publisher factory");
        }
        Err(_) => {
            warn!(
                timeout_secs = shutdown_timeout.as_secs(),
                "Publisher factory disconnection timed out"
            );
        }
    }

    match tokio::time::timeout(shutdown_timeout, data_writer.disconnect()).await {
        Ok(Ok(())) => {
            info!("Data writer disconnected successfully");
        }
        Ok(Err(e)) => {
            error!(error = %e, "Failed to disconnect data writer");
        }
        Err(_) => {
            warn!(
                timeout_secs = shutdown_timeout.as_secs(),
                "Data writer disconnection timed out"
            );
        }
    }

    info!("Global resource shutdown completed");

    // Give a brief moment for any remaining WebSocket cleanup
    tokio::time::sleep(Duration::from_millis(500)).await;
}

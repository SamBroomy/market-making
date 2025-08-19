use std::time::Duration;

use anyhow::{Context, Result};
use binance_sdk::spot::rest_api::DepthParams;
use futures_util::future::try_join_all;
use iggy::{
    client::{Client, SystemClient},
    clients::client::IggyClient,
};
use sqlx::PgPool;
use tokio::sync::{OnceCell, broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::{
    book::order_book::{OrderBook, SnapshotRequest},
    config::Config,
    data::binance::BinanceClient,
    shutdown::ShutdownCoordinator,
    streaming::{DatabaseWriter, MessageProducer, StreamManager},
};

// Global singletons for shared resources (DB and Iggy only - not Binance)
static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();
static IGGY_CLIENT: OnceCell<IggyClient> = OnceCell::const_new();

async fn get_db_pool(config: &Config) -> &'static PgPool {
    DB_POOL
        .get_or_init(|| async {
            let database_url = config.get_database_url();

            info!("Connecting to TimescaleDB at: {database_url}");
            let pool = PgPool::connect(&database_url)
                .await
                .context("Failed to connect to TimescaleDB")
                .unwrap();

            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .context("Failed to run database migrations")
                .unwrap();

            pool
        })
        .await
}

async fn get_iggy_client(config: &Config) -> &'static IggyClient {
    IGGY_CLIENT
        .get_or_init(|| async {
            let iggy_connection = config.get_iggy_connection_string();

            info!("Connecting to Iggy message queue at: {iggy_connection}");
            let client = IggyClient::from_connection_string(&iggy_connection)
                .context("Failed to create Iggy client")
                .unwrap();

            client
                .connect()
                .await
                .context("Failed to connect to Iggy")
                .unwrap();
            client
                .ping()
                .await
                .context("Failed to ping Iggy server")
                .unwrap();
            client
        })
        .await
}

// Create a new BinanceClient instance for each symbol (no more singleton)
async fn create_binance_client(config: &Config) -> BinanceClient {
    BinanceClient::new(config).await
}

/// Gracefully shutdown all global resources with timeout
pub async fn shutdown_global_resources() {
    info!("Starting graceful shutdown of global resources...");

    // Define timeout for resource cleanup
    let shutdown_timeout = Duration::from_secs(10);

    // Shutdown Iggy client if it was initialized
    if let Some(iggy_client) = IGGY_CLIENT.get() {
        info!("Shutting down Iggy client...");

        match tokio::time::timeout(shutdown_timeout, iggy_client.shutdown()).await {
            Ok(Ok(())) => {
                info!("Iggy client shut down successfully");
            }
            Ok(Err(e)) => {
                error!("Failed to shutdown Iggy client: {}", e);
            }
            Err(_) => {
                warn!(
                    "Iggy client shutdown timed out after {:?}",
                    shutdown_timeout
                );
            }
        }
    }

    // Close database pool if it was initialized
    if let Some(db_pool) = DB_POOL.get() {
        info!("Closing database connection pool...");

        match tokio::time::timeout(shutdown_timeout, db_pool.close()).await {
            Ok(()) => {
                info!("Database pool closed successfully");
            }
            Err(_) => {
                warn!(
                    "Database pool closure timed out after {:?}",
                    shutdown_timeout
                );
            }
        }
    }

    info!("Global resource shutdown completed");
    
    // Give a brief moment for any remaining WebSocket cleanup
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// Handles snapshot requests for orderbooks with graceful shutdown
async fn run_snapshot_task(
    mut snapshot_rx: mpsc::UnboundedReceiver<SnapshotRequest>,
    bc: BinanceClient, // Now takes ownership instead of static reference
    database_writer: DatabaseWriter,
    config: Config,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
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

                    // Note: Rate limit headers monitoring would be ideal here
                    // but RestApiResponse may not expose headers directly via binance-sdk
                    debug!("Snapshot requested for {} (limit: {:?})", req_symbol, limit);

                    let snapshot_result = snapshot_response
                        .data()
                        .await
                        .context("Failed to get depth snapshot");

                    // Persist to database if successful and enabled
                    if config.get_enable_database()
                        && let Ok(snapshot) = &snapshot_result
                        && let Err(e) = database_writer
                            .write_depth_snapshot(snapshot, &req_symbol, reason.as_str())
                            .await
                    {
                        error!("Failed to write snapshot to database: {}", e);
                    }

                    // Send response
                    let _ = response_tx.send(snapshot_result);
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
}

/// Runs all streams and orderbook for a single symbol with graceful shutdown
pub async fn run_symbol_producer(
    symbol: String,
    config: &Config,
    shutdown_coordinator: &ShutdownCoordinator,
) -> Result<()> {
    info!(
        "Starting producer for symbol: {} (snapshot limit: {})",
        symbol,
        config.get_snapshot_limit()
    );

    // Create per-symbol BinanceClient instance (no more singleton)
    let bc = create_binance_client(config).await;
    let iggy_client = get_iggy_client(config).await;
    let pool = get_db_pool(config).await.clone();
    let database_writer = DatabaseWriter::new(pool.clone());

    // Create channels for orderbook communication
    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();
    let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();

    // Create message producers for orderbook signals and state (if enabled)
    let signals_producer = if config.get_enable_signals() && config.get_enable_streaming() {
        Some(MessageProducer::new(iggy_client, "orderbook_signals", &symbol, 1).await?)
    } else {
        None
    };

    let state_producer = if config.get_enable_state() && config.get_enable_streaming() {
        Some(MessageProducer::new(iggy_client, "orderbook_state", &symbol, 1).await?)
    } else {
        None
    };

    // Start snapshot task with shutdown handling - needs a separate BinanceClient instance
    let bc_for_snapshots = create_binance_client(config).await;
    let config_clone = config.clone();
    let snapshot_shutdown_rx = shutdown_coordinator.subscribe();
    let snapshot_task = tokio::spawn(run_snapshot_task(
        snapshot_request_rx,
        bc_for_snapshots,
        database_writer.clone(),
        config_clone,
        snapshot_shutdown_rx,
    ));

    // Clone symbol for use in tasks
    let symbol_for_streams = symbol.clone();
    let symbol_for_orderbook = symbol.clone();

    // Start all market data streams (ticker, trades, etc.) with its own BinanceClient
    let stream_manager = StreamManager::new(symbol.clone(), iggy_client, pool, bc).await?;
    let mut streams_shutdown_rx = shutdown_coordinator.subscribe();
    let streams_task = tokio::spawn(async move {
        tokio::select! {
            result = stream_manager.start_all_streams(Some(depth_for_orderbook_tx)) => {
                result
            }
            _ = streams_shutdown_rx.recv() => {
                info!("Stream manager received shutdown signal for {}", symbol_for_streams);
                // Properly shutdown the stream manager and close WebSocket connections
                stream_manager.shutdown().await
            }
        }
    });
    // Allow some time for streams to initialize and receive a few depth updates before starting orderbook
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Start enhanced orderbook with configurable snapshot limit
    let orderbook = OrderBook::new(
        symbol.clone(),
        Some(config.get_snapshot_limit()),
        depth_for_orderbook_rx,
        snapshot_request_tx,
        signals_producer,
        state_producer,
        if config.get_enable_database() {
            Some(database_writer)
        } else {
            None
        },
    )
    .await?;

    let mut orderbook_shutdown_rx = shutdown_coordinator.subscribe();
    let orderbook_task = tokio::spawn(async move {
        tokio::select! {
            result = orderbook.run() => {
                result
            }
            _ = orderbook_shutdown_rx.recv() => {
                info!("OrderBook received shutdown signal for {}", symbol_for_orderbook);
                Ok(())
            }
        }
    });

    // Wait for all tasks or shutdown signal
    let mut main_shutdown_rx = shutdown_coordinator.subscribe();
    let tasks = [snapshot_task, streams_task, orderbook_task];

    tokio::select! {
        results = try_join_all(tasks) => {
            match results {
                Ok(task_results) => {
                    // Check individual task results
                    for (i, result) in task_results.into_iter().enumerate() {
                        match result {
                            Ok(()) => info!("Task {} for {} completed successfully", i, symbol),
                            Err(e) => error!("Task {} for {} failed: {}", i, symbol, e),
                        }
                    }
                    info!("All tasks for {} completed", symbol);
                }
                Err(e) => {
                    error!("Task join error for {}: {}", symbol, e);
                    return Err(e.into());
                }
            }
        }
        _ = main_shutdown_rx.recv() => {
            info!("Symbol producer for {} received shutdown signal", symbol);
            info!("Tasks for {} will shut down via their individual shutdown signals", symbol);
        }
    }

    info!("Producer for {} finished", symbol);
    Ok(())
}

/// Run producers for multiple symbols concurrently with graceful shutdown
pub async fn run_multi_symbol_producer(
    symbols: Vec<String>,
    config: &Config,
    shutdown_coordinator: &ShutdownCoordinator,
) -> Result<()> {
    info!("Starting multi-symbol producer for: {:?}", symbols);
    info!(
        "Configuration: snapshot_limit={}, startup_delay={}s, shutdown_timeout={}s",
        config.get_snapshot_limit(),
        config.get_startup_delay().as_secs(),
        30 // Default shutdown timeout
    );

    let mut tasks = Vec::with_capacity(symbols.len());

    for (index, symbol) in symbols.into_iter().enumerate() {
        // Check for shutdown before starting new symbols
        if shutdown_coordinator.is_shutting_down() {
            info!("Shutdown detected, not starting remaining symbols");
            break;
        }

        // Stagger startup to avoid overwhelming services with configurable delay
        if index > 0 {
            let delay = config.get_startup_delay();
            info!(
                "Waiting {}s before starting producer for {} (rate limiting)",
                delay.as_secs(),
                symbol
            );

            // Use select to allow shutdown during startup delay
            let mut delay_shutdown_rx = shutdown_coordinator.subscribe();
            tokio::select! {
                () = tokio::time::sleep(delay) => {},
                _ = delay_shutdown_rx.recv() => {
                    info!("Shutdown received during startup delay, stopping");
                    break;
                }
            }
        }

        info!("Starting producer for symbol: {}", symbol);
        let symbol_clone = symbol.clone();
        let config_clone = config.clone();
        let shutdown_coordinator_clone = shutdown_coordinator.clone();

        let task = tokio::spawn(async move {
            run_symbol_producer(symbol_clone, &config_clone, &shutdown_coordinator_clone).await
        });
        tasks.push(task);
    }

    info!("All {} symbol producers have been spawned", tasks.len());

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
                            Ok(()) => info!("Symbol producer {} completed successfully", i),
                            Err(e) => {
                                error!("Symbol producer {} failed: {}", i, e);
                                errors.push(e);
                            }
                        }
                    }

                    if errors.is_empty() {
                        info!("All symbol producers completed successfully");
                    } else {
                        error!("{} symbol producers failed", errors.len());
                        return Err(errors.into_iter().next().unwrap());
                    }
                }
                Err(e) => {
                    error!("Task join error: {}", e);
                    return Err(e.into());
                }
            }
        }
        _ = main_shutdown_rx.recv() => {
            info!("Multi-symbol producer received shutdown signal");
            info!("Symbol producers will shut down via their individual shutdown signals");
        }
    }

    info!("Multi-symbol producer finished");
    Ok(())
}

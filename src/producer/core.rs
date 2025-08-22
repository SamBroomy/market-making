use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::future::try_join_all;
use iggy::{
    client::{Client, SystemClient},
    clients::client::IggyClient,
};
use secrecy::ExposeSecret;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use tracing::{error, info, warn};

use crate::{
    data::binance::BinanceClient,
    settings::{BinanceSettings, Settings},
    shutdown::ShutdownCoordinator,
    streaming::StreamManager,
};

// Global singletons for shared resources (DB and Iggy only - not Binance)
static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();
static IGGY_CLIENT: OnceCell<IggyClient> = OnceCell::const_new();

async fn get_db_pool() -> PgPool {
    DB_POOL
        .get_or_init(|| async {
            let database_settings = Settings::get_database_settings();
            let database_url = database_settings
                .connection_string()
                .expose_secret()
                .to_string();

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
        .clone()
}

async fn get_iggy_client() -> &'static IggyClient {
    IGGY_CLIENT
        .get_or_init(|| async {
            let iggy_settings = Settings::get_iggy_settings();
            let iggy_connection = iggy_settings.connection_string();

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

// Create a new BinanceClient instance for each pair
async fn create_binance_client(settings: &BinanceSettings) -> BinanceClient {
    BinanceClient::new(settings).await
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

/// Run producers for multiple market pairs concurrently with graceful shutdown
pub async fn run_multi_market_producer(
    settings: Settings,
    shutdown_coordinator: ShutdownCoordinator,
) -> Result<()> {
    let pair_configs = settings.trading.get_pair_configs();

    let mut tasks = Vec::with_capacity(pair_configs.len());
    let binance_settings = settings.binance;

    for (index, pair_cfg) in pair_configs.into_iter().enumerate() {
        // check for shutdown before starting new pairs
        if shutdown_coordinator.is_shutting_down() {
            info!("Shutdown detected, not starting remaining pairs");
            break;
        }
        let symbol = &pair_cfg.symbol.clone();

        // stagger startup to avoid overwhelming services with configurable delay
        if index > 0 {
            let delay = binance_settings.get_startup_delay();
            info!(
                "Waiting {}s before starting producer for {} (rate limiting)",
                delay.as_secs(),
                symbol
            );

            // allow shutdown during startup delay
            let mut delay_shutdown_rx = shutdown_coordinator.subscribe();
            tokio::select! {
                () = tokio::time::sleep(delay) => {},
                _ = delay_shutdown_rx.recv() => {
                    info!("Shutdown received during startup delay, stopping");
                    break;
                }
            }
        }

        info!("Starting producer for symbol: {}", &symbol);
        let shutdown_coordinator_ = shutdown_coordinator.clone();
        let symbol_ = symbol.clone();
        let task = tokio::spawn(async move {
            // Create a new BinanceClient for this pair
            let binance_client = create_binance_client(&binance_settings.clone()).await;

            // Start all streams with built-in shutdown handling
            StreamManager::run(
                pair_cfg,
                binance_settings,
                get_iggy_client().await,
                get_db_pool().await,
                shutdown_coordinator_,
            )
            .await?;

            info!("Producer for {} finished", symbol_);
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
                            Ok(()) => info!("Pair producer {} completed successfully", i),
                            Err(e) => {
                                error!("Pair producer {} failed: {}", i, e);
                                errors.push(e);
                            }
                        }
                    }

                    if errors.is_empty() {
                        info!("All pair producers completed successfully");
                    } else {
                        error!("{} pair producers failed", errors.len());
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
            info!("Multi-pair producer received shutdown signal");
            info!("Pair producers will shut down via their individual shutdown signals");
        }
    }

    info!("Multi-pair producer finished");
    Ok(())
}

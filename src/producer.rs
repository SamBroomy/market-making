use std::{
    env,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result};
use binance_sdk::spot::rest_api::DepthParams;
use futures_util::future::try_join_all;
use iggy::{
    client::{Client, SystemClient},
    clients::client::IggyClient,
};
use is_docker::is_docker;
use market_making::{
    book::order_book::{OrderBook, SnapshotRequest},
    data::binance::BinanceClient,
    streaming::{DatabaseWriter, MessageProducer, StreamManager},
};
use sqlx::PgPool;
use tokio::sync::{OnceCell, mpsc};
use tracing::{error, info};

// lobal shutdown signal
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn is_shutting_down() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}

fn initiate_shutdown() {
    SHUTDOWN.store(true, Ordering::Relaxed);
    info!("Shutdown signal received");
}

static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();
static IGGY_CLIENT: OnceCell<IggyClient> = OnceCell::const_new();
static BINANCE_CLIENT: OnceCell<BinanceClient> = OnceCell::const_new();

async fn get_db_pool() -> &'static PgPool {
    DB_POOL
        .get_or_init(|| async {
            let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
                if is_docker() {
                    "postgres://postgres:password@timescaledb:5432/market_data".to_string()
                } else {
                    "postgres://postgres:password@localhost:5432/market_data".to_string()
                }
            });

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

async fn get_iggy_client() -> &'static IggyClient {
    IGGY_CLIENT
        .get_or_init(|| async {
            let iggy_connection = env::var("IGGY_CONNECTION_STRING").unwrap_or_else(|_| {
                if is_docker() {
                    "iggy://iggy:Secret123!@iggy:3000".to_string()
                } else {
                    "iggy://iggy:Secret123!@localhost:5100".to_string()
                }
            });

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

async fn get_binance_client() -> &'static BinanceClient {
    BINANCE_CLIENT
        .get_or_init(|| async { BinanceClient::new().await })
        .await
}

/// Handles snapshot requests for orderbooks
async fn run_snapshot_task(
    mut snapshot_rx: mpsc::UnboundedReceiver<SnapshotRequest>,
    bc: &'static BinanceClient,
    database_writer: DatabaseWriter,
) -> Result<()> {
    info!("Starting snapshot task");

    while let Some(SnapshotRequest {
        symbol: req_symbol,
        limit,
        response_tx,
        reason,
    }) = snapshot_rx.recv().await
    {
        let snapshot_result = bc
            .depth_snapshot(
                DepthParams::builder(req_symbol.clone())
                    .limit(limit)
                    .build()?,
            )
            .await?
            .data()
            .await
            .context("Failed to get depth snapshot");

        // Persist to database if successful
        if let Ok(snapshot) = &snapshot_result
            && let Err(e) = database_writer
                .write_depth_snapshot(snapshot, &req_symbol, reason.as_str())
                .await
        {
            error!("Failed to write snapshot to database: {}", e);
        }

        // Send response
        let _ = response_tx.send(snapshot_result);
    }

    info!("Snapshot task completed");
    Ok(())
}

/// Runs all streams and orderbook for a single symbol
async fn run_symbol_producer(symbol: String) -> Result<()> {
    info!("Starting producer for symbol: {}", symbol);

    let bc = get_binance_client().await;
    let iggy_client = get_iggy_client().await;
    let pool = get_db_pool().await.clone();
    let database_writer = DatabaseWriter::new(pool.clone());

    // Create channels for orderbook communication
    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();
    let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();

    // Create message producers for orderbook signals and state
    let signals_producer =
        MessageProducer::new(iggy_client, "orderbook_signals", &symbol, 1).await?;

    let state_producer = MessageProducer::new(iggy_client, "orderbook_state", &symbol, 1).await?;

    // Start snapshot task
    let snapshot_task = tokio::spawn(run_snapshot_task(
        snapshot_request_rx,
        bc,
        database_writer.clone(),
    ));

    // Start all market data streams (ticker, trades, etc.)
    let stream_manager = StreamManager::new(symbol.clone(), iggy_client, pool, bc);
    let streams_task = tokio::spawn(async move {
        stream_manager
            .start_all_streams(Some(depth_for_orderbook_tx))
            .await
    });

    // Start enhanced orderbook with dual streaming
    let orderbook = OrderBook::new(
        symbol.clone(),
        Some(999),
        depth_for_orderbook_rx,
        snapshot_request_tx,
        Some(signals_producer),
        Some(state_producer),
        Some(database_writer),
    )
    .await?;

    let orderbook_task = tokio::spawn(async move { orderbook.run().await });

    // Wait for all tasks
    let results = tokio::try_join!(snapshot_task, streams_task, orderbook_task);

    match results {
        Ok(_) => info!("All tasks for {} completed successfully", symbol),
        Err(e) => error!("Task failed for {}: {}", symbol, e),
    }

    Ok(())
}

/// Run producers for multiple symbols concurrently
pub async fn run_multi_symbol_producer(symbols: Vec<String>) -> Result<()> {
    info!("Starting multi-symbol producer for: {:?}", symbols);

    let mut tasks = Vec::with_capacity(symbols.len());

    for (index, symbol) in symbols.into_iter().enumerate() {
        // Stagger startup to avoid overwhelming services
        if index > 0 {
            let delay_seconds = 10;
            info!(
                "Waiting {} seconds before starting producer for {}",
                delay_seconds, symbol
            );
            tokio::time::sleep(std::time::Duration::from_secs(delay_seconds)).await;
        }

        info!("Starting producer for symbol: {}", symbol);
        let task = tokio::spawn(run_symbol_producer(symbol.clone()));
        tasks.push(task);
    }

    info!("All {} symbol producers have been spawned", tasks.len());

    // Wait for all tasks to complete
    let results = try_join_all(tasks).await?;

    // Check for any errors
    for result in results {
        result?;
    }

    info!("All symbol producers completed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    // Get symbols from environment or use defaults
    let symbols_str =
        env::var("SYMBOLS").unwrap_or_else(|_| "BTCUSDT,ETHUSDT,MAGICUSDT,ETHBTC".to_string());
    let symbols: Vec<String> = symbols_str
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .collect();

    info!("Starting market data producer for symbols: {:?}", symbols);

    if let Err(e) = run_multi_symbol_producer(symbols).await {
        error!("Producer failed: {}", e);
        return Err(e);
    }

    Ok(())
}

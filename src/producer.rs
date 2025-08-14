use std::{
    env,
    str::FromStr,
    sync::{Arc, LazyLock},
};

use anyhow::{Context, Result};
use binance_sdk::spot::{
    rest_api::DepthParams,
    websocket_streams::{
        DiffBookDepthParams, RollingWindowTickerParams, RollingWindowTickerWindowSizeEnum,
        TickerParams,
    },
};
use chrono::Utc;
use futures_util::future::try_join_all;
use iggy::{
    client::{Client, SystemClient},
    clients::{client::IggyClient, producer::IggyProducer},
    messages::send_messages::{Message, Partitioning},
    utils::{duration::IggyDuration, expiry::IggyExpiry, topic_size::MaxTopicSize},
};
use is_docker::is_docker;
use market_making::{
    book::{OrderBook, SnapshotRequest},
    data::binance::{
        BinanceClient,
        models::{DepthUpdate, TickerData, WindowTickerData},
    },
};
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

const PARTITIONS: u32 = 1;

// Lazy static database pool
static DB_POOL: LazyLock<PgPool> = LazyLock::new(|| {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        if is_docker() {
            "postgres://postgres:password@timescaledb:5432/market_data".to_string()
        } else {
            "postgres://postgres:password@localhost:5432/market_data".to_string()
        }
    });

    tokio::runtime::Handle::current().block_on(async {
        info!("Connecting to TimescaleDB at: {database_url}");
        PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to TimescaleDB")
    })
});

// Lazy static Iggy client
static IGGY_CLIENT: LazyLock<IggyClient> = LazyLock::new(|| {
    let iggy_connection = env::var("IGGY_CONNECTION_STRING").unwrap_or_else(|_| {
        if is_docker() {
            "iggy://iggy:Secret123!@iggy:3000".to_string()
        } else {
            "iggy://iggy:Secret123!@localhost:5100".to_string()
        }
    });

    tokio::runtime::Handle::current().block_on(async {
        println!("Connecting to Iggy message queue at: {iggy_connection}");
        let client = IggyClient::from_connection_string(&iggy_connection)
            .expect("Failed to create Iggy client");

        client.connect().await.expect("Failed to connect to Iggy");
        client.ping().await.expect("Failed to ping Iggy server");

        client
    })
});

async fn create_producer(stream: &str, topic: &str) -> Result<IggyProducer> {
    let mut producer = IGGY_CLIENT
        .producer(stream, topic)
        .context("Failed to create producer")?
        .batch_size(1000)
        // .direct(
        //     // Use either direct (instant) or background message sending
        //     DirectConfig::builder()
        //         .batch_length(1000)
        //         .linger_time(IggyDuration::from_str("5ms")?)
        //         .build(),
        // )
        .send_interval(IggyDuration::from_str("1ms")?)
        .partitioning(Partitioning::balanced())
        .create_stream_if_not_exists()
        .create_topic_if_not_exists(
            PARTITIONS,
            None,
            IggyExpiry::ExpireDuration(IggyDuration::from_str("1d")?),
            MaxTopicSize::ServerDefault,
        )
        .build();

    producer.init().await?;

    Ok(producer)
}

async fn run_depth_task(
    symbol: String,
    mut depth_rx: mpsc::UnboundedReceiver<DepthUpdate>,
    depth_for_orderbook_tx: mpsc::UnboundedSender<DepthUpdate>,
) -> Result<()> {
    let depth_producer = create_producer("BINANCE", "diff_book_depth")
        .await
        .context("Failed to create depth producer")?;

    info!("Starting depth task for symbol: {}", symbol);

    while let Some(depth) = depth_rx.recv().await {
        // Forward to order book
        if depth_for_orderbook_tx.send(depth.clone()).is_err() {
            warn!("Order book receiver for {} is closed", symbol);
            break;
        }

        // Send to message queue
        let message = Message::from_str(&serde_json::to_string(&depth)?)
            .context("Failed to create message from depth data")?;

        if let Err(e) = depth_producer.send_one(message).await {
            error!("Failed to send depth message for {}: {}", symbol, e);
            continue; // Don't break, just skip this message
        }

        // Insert into database
        if let Err(e) = sqlx::query!(
            r"INSERT INTO depth_updates (event_time, symbol, first_update_id, final_update_id, bids, asks)
            VALUES ($1, $2, $3, $4, $5, $6)",
            depth.event_time,
            depth.symbol,
            Decimal::from(depth.first_update_id),
            Decimal::from(depth.final_update_id),
            serde_json::to_value(&depth.bids).unwrap(),
            serde_json::to_value(&depth.asks).unwrap(),
        )
        .execute(&*DB_POOL)
        .await {
            error!("Failed to insert depth update for {}: {}", symbol, e);
        }
    }

    info!("Depth task for {} completed", symbol);
    Ok(())
}

async fn run_ticker_task(
    symbol: String,
    mut ticker_rx: mpsc::UnboundedReceiver<TickerData>,
) -> Result<()> {
    let ticker_producer = create_producer("BINANCE", "ticker")
        .await
        .context("Failed to create ticker producer")?;
    info!("Starting ticker task for symbol: {}", symbol);

    while let Some(ticker) = ticker_rx.recv().await {
        // Send to message queue
        let message = Message::from_str(&serde_json::to_string(&ticker)?)
            .context("Failed to create message from ticker data")?;

        if let Err(e) = ticker_producer.send_one(message).await {
            error!("Failed to send ticker message for {}: {}", symbol, e);
            continue;
        }

        // Insert into database
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
        .execute(&*DB_POOL)
        .await {
            error!("Failed to insert ticker data for {}: {}", symbol, e);
        }
    }

    info!("Ticker task for {} completed", symbol);
    Ok(())
}

async fn run_window_ticker_task(
    symbol: String,
    mut window_rx: mpsc::UnboundedReceiver<WindowTickerData>,
) -> Result<()> {
    let window_producer = create_producer("BINANCE", "rolling_window_ticker_1h")
        .await
        .context("Failed to create window ticker producer")?;
    info!("Starting window ticker task for symbol: {}", symbol);

    while let Some(ticker) = window_rx.recv().await {
        // Send to message queue
        let message = Message::from_str(&serde_json::to_string(&ticker)?)
            .context("Failed to create message from window ticker data")?;

        if let Err(e) = window_producer.send_one(message).await {
            error!("Failed to send window ticker message for {}: {}", symbol, e);
            continue;
        }

        // Insert into database
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
        .execute(&*DB_POOL)
        .await
        {
            error!("Failed to insert window ticker data for {}: {}", symbol, e);
        }
    }

    info!("Window ticker task for {} completed", symbol);
    Ok(())
}

async fn run_snapshot_task(
    symbol: String,
    mut snapshot_rx: mpsc::UnboundedReceiver<SnapshotRequest>,
    bc: &BinanceClient,
) -> Result<()> {
    let snapshot_producer = create_producer("BINANCE", "depth_snapshot")
        .await
        .context("Failed to create snapshot producer")?;
    info!("Starting snapshot task for symbol: {}", symbol);

    while let Some(SnapshotRequest {
        symbol: req_symbol,
        limit,
        response_tx,
        reason,
    }) = snapshot_rx.recv().await
    {
        let snapshot = bc
            .depth_snapshot(
                DepthParams::builder(req_symbol.clone())
                    .limit(limit)
                    .build()?,
            )
            .await?
            .data()
            .await
            .context("Failed to get depth snapshot");

        if let Ok(snapshot) = &snapshot {
            // Send to message queue
            if let Err(e) = snapshot_producer
                .send_one(
                    Message::from_str(&serde_json::to_string(snapshot)?)
                        .context("Failed to create message from snapshot")?,
                )
                .await
            {
                error!("Failed to send snapshot message for {}: {}", symbol, e);
            }

            // Insert into database
            if let Err(e) = sqlx::query!(
                r"INSERT INTO depth_snapshots (
                    event_time, symbol, last_update_id, bids, asks, snapshot_reason
                ) VALUES ($1, $2, $3, $4, $5, $6)",
                Utc::now(),
                req_symbol,
                Decimal::from(snapshot.last_update_id),
                serde_json::to_value(&snapshot.bids).unwrap(),
                serde_json::to_value(&snapshot.asks).unwrap(),
                reason.as_str(),
            )
            .execute(&*DB_POOL)
            .await
            {
                error!("Failed to insert depth snapshot for {}: {}", symbol, e);
            }
        }

        let _ = response_tx.send(snapshot);
    }

    info!("Snapshot task for {} completed", symbol);
    Ok(())
}

async fn run_symbol_producer(bc: Arc<BinanceClient>, symbol: String) -> Result<()> {
    info!("Starting producer for symbol: {}", symbol);

    // Create data streams
    let depth_rx = bc
        .diff_book_depth(
            DiffBookDepthParams::builder(symbol.clone())
                .update_speed("100ms".to_string())
                .build()?,
        )
        .await?;

    let ticker_rx = bc
        .ticker(TickerParams::builder(symbol.clone()).build()?)
        .await?;

    let ticker_window_1h_rx = bc
        .rolling_window_ticker(
            RollingWindowTickerParams::builder(
                symbol.clone(),
                RollingWindowTickerWindowSizeEnum::WindowSize1h,
            )
            .build()?,
        )
        .await?;

    // Create channels
    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();
    let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();

    // Create order book
    let order_book = OrderBook::new(
        symbol.clone(),
        Some(100),
        depth_for_orderbook_rx,
        snapshot_request_tx,
    )
    .await?;

    // Spawn all tasks
    let tasks = tokio::try_join!(
        run_depth_task(symbol.clone(), depth_rx, depth_for_orderbook_tx,),
        run_ticker_task(symbol.clone(), ticker_rx),
        run_window_ticker_task(symbol.clone(), ticker_window_1h_rx,),
        run_snapshot_task(symbol.clone(), snapshot_request_rx, &bc),
        async move { order_book.run().await }
    );

    match tasks {
        Ok(_) => info!("All tasks for {} completed successfully", symbol),
        Err(e) => error!("Task failed for {}: {}", symbol, e),
    }

    Ok(())
}

/// Run producers for multiple symbols concurrently
pub async fn run_multi_symbol_producer(symbols: Vec<String>) -> Result<()> {
    // Initialize lazy statics
    LazyLock::force(&DB_POOL);
    LazyLock::force(&IGGY_CLIENT);
    let bc = Arc::new(BinanceClient::new().await);

    info!("Starting multi-symbol producer for: {:?}", symbols);

    let tasks: Vec<_> = symbols
        .into_iter()
        .map(|symbol| tokio::spawn(run_symbol_producer(Arc::clone(&bc), symbol)))
        .collect();

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
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string()))
        .init();

    // Get symbols from environment or use defaults
    let symbols_str = env::var("SYMBOLS")
        .unwrap_or_else(|_| "BTCUSDT,ETHUSDT,BNBUSDT,SOLUSDT,DOTUSDT,MAGICUSDT,BTCETH".to_string());
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

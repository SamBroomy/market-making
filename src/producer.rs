use std::{env, str::FromStr};

use anyhow::{Context, Result};
use binance_sdk::spot::{
    rest_api::DepthParams,
    websocket_streams::{
        AggTradeParams, DiffBookDepthParams, RollingWindowTickerParams,
        RollingWindowTickerWindowSizeEnum, TickerParams,
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
        models::{AggregateTrade, DepthUpdate, TickerData, WindowTickerData},
    },
};
use rust_decimal::Decimal;
use sqlx::PgPool;
use tokio::sync::{OnceCell, mpsc};
use tracing::{debug, error, info, warn};
const PARTITIONS: u32 = 1;

static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();

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

static IGGY_CLIENT: OnceCell<IggyClient> = OnceCell::const_new();

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

static BINANCE_CLIENT: OnceCell<BinanceClient> = OnceCell::const_new();

async fn get_binance_client() -> &'static BinanceClient {
    BINANCE_CLIENT
        .get_or_init(|| async { BinanceClient::new().await })
        .await
}

async fn create_producer(stream: &str, topic: &str) -> Result<IggyProducer> {
    let mut producer = get_iggy_client()
        .await
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
    let pool = get_db_pool().await;
    let depth_producer = create_producer("diff_book_depth", &symbol)
        .await
        .context("Failed to create depth producer")?;

    info!("Starting depth task for symbol: {}", symbol);

    while let Some(depth) = depth_rx.recv().await {
        debug!("Depth Update - {}: {:?}", symbol, depth);
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
        .execute(pool)
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
    let pool = get_db_pool().await;
    let ticker_producer = create_producer("ticker", &symbol)
        .await
        .context("Failed to create ticker producer")?;
    info!("Starting ticker task for symbol: {}", symbol);

    while let Some(ticker) = ticker_rx.recv().await {
        debug!("Ticker - {}: {:?}", symbol, ticker);
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
        .execute(pool)
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
    let pool = get_db_pool().await;
    let window_producer = create_producer("rolling_window_ticker_1h", &symbol)
        .await
        .context("Failed to create window ticker producer")?;
    info!("Starting window ticker task for symbol: {}", symbol);

    while let Some(ticker) = window_rx.recv().await {
        debug!("Window Ticker - {}: {:?}", symbol, ticker);
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
        .execute(pool)
        .await
        {
            error!("Failed to insert window ticker data for {}: {}", symbol, e);
        }
    }

    info!("Window ticker task for {} completed", symbol);
    Ok(())
}

// CREATE TABLE aggregate_trades (
//     event_time TIMESTAMPTZ NOT NULL,
//     symbol TEXT NOT NULL,
//     aggregate_trade_id NUMERIC NOT NULL,
//     price DECIMAL(20,8) NOT NULL,
//     quantity DECIMAL(20,8) NOT NULL,
//     first_trade_id NUMERIC NOT NULL,
//     last_trade_id NUMERIC NOT NULL,
//     trade_time TIMESTAMPTZ NOT NULL,
//     buyer_market_maker BOOLEAN NOT NULL
// ) WITH (
//     tsdb.hypertable,
//     tsdb.partition_column='event_time',
//     tsdb.segmentby='symbol',
//     tsdb.orderby='event_time DESC',
//     tsdb.chunk_interval='1d'
// );
async fn run_agg_trade_task(
    symbol: String,
    mut agg_trade_rx: mpsc::UnboundedReceiver<AggregateTrade>,
) -> Result<()> {
    let pool = get_db_pool().await;
    let agg_trade_producer = create_producer("agg_trade", &symbol)
        .await
        .context("Failed to create aggregate trade producer")?;
    info!("Starting aggregate trade task for symbol: {}", symbol);

    while let Some(agg_trade) = agg_trade_rx.recv().await {
        debug!("Aggregate Trade - {}: {:?}", symbol, agg_trade);
        // Send to message queue
        let message = Message::from_str(&serde_json::to_string(&agg_trade)?)
            .context("Failed to create message from aggregate trade data")?;

        if let Err(e) = agg_trade_producer.send_one(message).await {
            error!(
                "Failed to send aggregate trade message for {}: {}",
                symbol, e
            );
            continue;
        }

        // Insert into database
        if let Err(e) = sqlx::query!(
            r"INSERT INTO aggregate_trades (
                event_time, symbol, aggregate_trade_id, price, quantity,
                first_trade_id, last_trade_id, trade_time, buyer_market_maker
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            agg_trade.event_time,
            agg_trade.symbol,
            Decimal::from(agg_trade.aggregate_trade_id),
            agg_trade.price,
            agg_trade.quantity,
            Decimal::from(agg_trade.first_trade_id),
            Decimal::from(agg_trade.last_trade_id),
            agg_trade.trade_time,
            agg_trade.buyer_market_maker,
        )
        .execute(pool)
        .await
        {
            error!("Failed to insert aggregate trade for {}: {}", symbol, e);
        }
    }

    info!("Aggregate trade task for {} completed", symbol);
    Ok(())
}

async fn run_snapshot_task(
    symbol: String,
    mut snapshot_rx: mpsc::UnboundedReceiver<SnapshotRequest>,
    bc: &BinanceClient,
) -> Result<()> {
    let pool = get_db_pool().await;
    let snapshot_producer = create_producer("depth_snapshot", &symbol)
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
            debug!("Snapshot for {}: {:?}", req_symbol, snapshot);
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
            .execute(pool)
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

async fn run_symbol_producer(
    bc: &'static BinanceClient,
    symbol: String,
    pool: &PgPool,
) -> Result<()> {
    info!("Starting producer for symbol: {}", symbol);
    // Create channels
    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();
    let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();

    let symbol_ = symbol.clone();
    let depth_task = tokio::spawn(async move {
        let depth_rx = bc
            .diff_book_depth(
                DiffBookDepthParams::builder(symbol_.clone())
                    .update_speed("100ms".to_string())
                    .build()?,
            )
            .await
            .context("Failed to get depth stream")?;

        run_depth_task(symbol_, depth_rx, depth_for_orderbook_tx).await
    });
    let symbol_ = symbol.clone();
    let ticker_task = tokio::spawn(async move {
        let ticker_rx = bc
            .ticker(TickerParams::builder(symbol_.clone()).build()?)
            .await
            .context("Failed to get ticker stream")?;

        run_ticker_task(symbol_, ticker_rx).await
    });
    let symbol_ = symbol.clone();
    let ticker_window_task = tokio::spawn(async move {
        let ticker_window_1h_rx = bc
            .rolling_window_ticker(
                RollingWindowTickerParams::builder(
                    symbol_.clone(),
                    RollingWindowTickerWindowSizeEnum::WindowSize1h,
                )
                .build()?,
            )
            .await
            .context("Failed to get rolling window ticker stream")?;

        run_window_ticker_task(symbol_, ticker_window_1h_rx).await
    });
    let symbol_ = symbol.clone();
    let agg_trade_task = tokio::spawn(async {
        let agg_trade_rx = bc
            .agg_trade(AggTradeParams::builder(symbol_.clone()).build()?)
            .await
            .context("Failed to get aggregate trade stream")?;

        run_agg_trade_task(symbol_, agg_trade_rx).await
    });
    let symbol_ = symbol.clone();
    let snapshot_task =
        tokio::spawn(async move { run_snapshot_task(symbol_, snapshot_request_rx, bc).await });
    let symbol_ = symbol.clone();
    let order_book = OrderBook::new(
        symbol_,
        Some(100),
        depth_for_orderbook_rx,
        snapshot_request_tx,
    )
    .await?;
    let symbol_ = symbol.clone();
    let order_book_task = tokio::spawn(async move {
        order_book.run().await?;
        info!("Order book task for {} completed", symbol_);
        Ok::<(), anyhow::Error>(())
    });

    // Spawn all tasks
    let tasks = tokio::try_join!(
        depth_task,
        ticker_task,
        ticker_window_task,
        snapshot_task,
        order_book_task
    );

    match tasks {
        Ok(_) => info!("All tasks for {} completed successfully", symbol),
        Err(e) => error!("Task failed for {}: {}", symbol, e),
    }

    Ok(())
}

/// Run producers for multiple symbols concurrently
pub async fn run_multi_symbol_producer(symbols: Vec<String>) -> Result<()> {
    let bc = get_binance_client().await;
    let pool = get_db_pool().await;

    info!("Starting multi-symbol producer for: {:?}", symbols);

    let mut tasks = Vec::with_capacity(symbols.len());

    for (index, symbol) in symbols.into_iter().enumerate() {
        if index > 0 {
            let delay_seconds = 5;
            info!(
                "Waiting {} seconds before starting producer for {}",
                delay_seconds, symbol
            );
            tokio::time::sleep(std::time::Duration::from_secs(delay_seconds)).await;
        }

        info!("Starting producer for symbol: {}", symbol);
        let task = tokio::spawn(run_symbol_producer(bc, symbol.clone(), pool));
        tasks.push(task);

        info!("Producer spawned for symbol: {}", symbol);
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

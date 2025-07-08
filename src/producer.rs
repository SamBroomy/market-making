use std::env;

use anyhow::{Context, Result};
use binance_sdk::spot::{
    rest_api::DepthParams,
    websocket_streams::{
        DiffBookDepthParams, RollingWindowTickerParams, RollingWindowTickerWindowSizeEnum,
        TickerParams,
    },
};
use fluvio::{
    DeliverySemantic, Fluvio, FluvioClusterConfig, TopicProducerConfigBuilder, TopicProducerPool,
    metadata::topic::{CleanupPolicy, SegmentBasedPolicy, TopicSpec},
};
use market_making::{
    book::{OrderBook, SnapshotRequest},
    data::binance::BinanceClient,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

async fn create_producer(
    client: &Fluvio,
    topic: &str,
    partition: impl Into<Option<u32>>,
    replicas: impl Into<Option<u32>>,
) -> Result<TopicProducerPool> {
    // Create a topic
    let admin = client.admin().await;
    let partition = partition.into().unwrap_or(1).max(1);

    let replicas = replicas.into().unwrap_or(1).max(1);
    let topic_str = topic.to_string().trim().to_lowercase();

    let mut topic_spec = TopicSpec::new_computed(partition, replicas, None);
    topic_spec.set_cleanup_policy(CleanupPolicy::Segment(SegmentBasedPolicy {
        time_in_seconds: 60 * 60 * 24 * 365,
    }));
    let topic = admin.create(topic_str.clone(), false, topic_spec).await;

    info!(
        "Created topic: {:?} - Partitions: {}, Replicas: {}",
        topic_str, partition, replicas
    );

    // List topics
    let topics = admin.all::<TopicSpec>().await?;
    let topic_names = topics
        .iter()
        .map(|topic| topic.name.clone())
        .collect::<Vec<String>>();

    warn!("Topics:\n  - {}", topic_names.join("\n  - "));
    // Produce to a topic
    let config = TopicProducerConfigBuilder::default()
        .delivery_semantic(DeliverySemantic::AtMostOnce)
        .build()?;
    client.topic_producer_with_config(&topic_str, config).await
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();
    let symbol = env::var("SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());

    let client = {
        let mut config = if is_docker::is_docker() {
            // Inside Docker - connect to sc service directly
            FluvioClusterConfig::new("sc:9003")
        } else {
            // Local development - connect to mapped port
            FluvioClusterConfig::new("127.0.0.1:9103")
        };
        config.update_metadata_by_name("installation", "docker")?;
        Fluvio::connect_with_config(&config).await?
    };
    // let client = Fluvio::connect()
    //     .await
    //     .context("Failed to connect to Fluvio")?;
    warn!("{:#?}", client.metrics());
    warn!("{:#?}", client.platform_version());

    info!("Starting producer for symbol: {}", symbol);
    let symbol_producer = create_producer(&client, &symbol, None, None).await?;
    let depth_producer = symbol_producer.clone();
    let ticker_producer = symbol_producer.clone();
    let ticker_window_producer = symbol_producer.clone();
    let snapshot_producer = symbol_producer.clone();

    println!("Starting Binance Order Book Example");
    info!("Running!");

    let bc = BinanceClient::new().await;

    let mut depth_rx = bc
        .diff_book_depth(
            DiffBookDepthParams::builder(symbol.clone())
                .update_speed("100ms".to_string())
                .build()?,
        )
        .await?;

    let mut ticker_rx = bc
        .ticker(TickerParams::builder(symbol.clone()).build()?)
        .await?;

    let mut ticker_window_1h_rx = bc
        .rolling_window_ticker(
            RollingWindowTickerParams::builder(
                symbol.clone(),
                RollingWindowTickerWindowSizeEnum::WindowSize1h,
            )
            .build()?,
        )
        .await?;

    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();

    // Create channels for fan-out pattern
    let (depth_for_stream_tx, mut depth_for_stream_rx) = mpsc::unbounded_channel();
    let (depth_for_orderbook_tx, depth_for_orderbook_rx) = mpsc::unbounded_channel();

    // Fan-out task: consumes depth_rx and forwards to both stream and order book
    let depth_fanout_task = tokio::spawn(async move {
        info!("Starting depth fan-out task");
        while let Some(depth) = depth_rx.recv().await {
            // Forward to stream processor
            if let Ok(serialized_depth) = serde_json::to_vec(&depth) {
                debug!("Serialized depth");
                if let Err(e) = depth_for_stream_tx.send(serialized_depth) {
                    error!("Failed to forward depth to stream: {}", e);
                    break;
                }
            } else {
                error!("Failed to serialize depth update");
            }

            // Forward to order book
            info!("Depth: {:?}", depth);
            if let Err(e) = depth_for_orderbook_tx.send(depth) {
                error!("Failed to forward depth to order book: {}", e);
                break;
            }
        }
        info!("Depth fan-out task completed");
        Ok::<_, anyhow::Error>(())
    });

    let depth_task = tokio::spawn(async move {
        info!("Starting depth processing task");
        while let Some(depth) = depth_for_stream_rx.recv().await {
            if let Err(e) = depth_producer.send("diff_book_depth", depth).await {
                error!("Failed to send depth message: {}", e);
                break;
            }
            depth_producer.flush().await?;
        }
        info!("Depth processing task completed");
        Ok::<_, anyhow::Error>(())
    });

    let ticker_task = tokio::spawn(async move {
        info!("Starting ticker processing task");
        while let Some(ticker) = ticker_rx.recv().await {
            if let Err(e) = ticker_producer
                .send("ticker", serde_json::to_vec(&ticker)?)
                .await
            {
                error!("Failed to send ticker message: {}", e);
                break;
            }
            ticker_producer.flush().await?;
            info!("Ticker: {:?}", ticker);
        }
        info!("Ticker processing task completed");
        Ok::<_, anyhow::Error>(())
    });
    let ticker_window_task = tokio::spawn(async move {
        info!("Starting rolling window ticker processing task");
        while let Some(ticker) = ticker_window_1h_rx.recv().await {
            if let Err(e) = ticker_window_producer
                .send("rolling_window_ticker_1h", serde_json::to_vec(&ticker)?)
                .await
            {
                error!("Failed to send rolling window ticker message: {}", e);
                break;
            }
            ticker_window_producer.flush().await?;
            info!("Rolling Window Ticker 1h: {:?}", ticker);
        }
        info!("Rolling window ticker processing task completed");
        Ok::<_, anyhow::Error>(())
    });
    let snapshot_task = tokio::spawn(async move {
        let mut rx = snapshot_request_rx;
        while let Some(SnapshotRequest {
            symbol,
            limit,
            response_tx,
        }) = rx.recv().await
        {
            info!(symbol, "Received snapshot request with limit: {:?}", limit);

            let snapshot = bc
                .depth_snapshot(DepthParams::builder(symbol).limit(limit).build()?)
                .await?
                .data()
                .await
                .context("Failed to get depth snapshot");

            if let Ok(snapshot) = &snapshot
                && let Err(e) = snapshot_producer
                    .send("depth_snapshot", serde_json::to_vec(&snapshot)?)
                    .await
            {
                tracing::error!("Failed to send snapshot message: {}", e);
            }

            let _ = response_tx.send(snapshot);
        }
        Ok::<_, anyhow::Error>(())
    });

    info!("Creating order book for symbol: {}", symbol);
    let order_book = OrderBook::new(
        symbol.clone(),
        Some(100),
        depth_for_orderbook_rx,
        snapshot_request_tx,
    )
    .await?;

    let order_book_task = tokio::spawn(async move {
        info!("Running order book for symbol: {}", symbol);
        order_book.run().await
    });

    //     tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    //     Ok(())
    // }

    let results = tokio::try_join!(
        depth_fanout_task,
        depth_task,
        ticker_task,
        ticker_window_task,
        snapshot_task,
        order_book_task
    );

    match results {
        Ok((fanout_res, depth_res, ticker_res, window_res, snapshot_res, book_res)) => {
            fanout_res?;
            depth_res?;
            ticker_res?;
            window_res?;
            snapshot_res?;
            book_res?;
            info!("All producer tasks completed successfully");
        }
        Err(e) => {
            tracing::error!("Producer task failed: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

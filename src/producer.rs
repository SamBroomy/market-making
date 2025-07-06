use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result};
use binance_sdk::{
    config::ConfigurationWebsocketApi,
    spot::{
        self,
        rest_api::DepthParams,
        websocket_streams::{
            DiffBookDepthParams, RollingWindowTickerParams, RollingWindowTickerWindowSizeEnum,
            TickerParams,
        },
    },
};
use binance_spot_connector_rust::{market::depth, market_stream::ticker};
use crossbeam_channel::unbounded;
use iggy::{
    bytes_serializable::BytesSerializable,
    client::{Client, SystemClient, UserClient},
    clients::{client::IggyClient, producer::IggyProducer},
    messages::send_messages::{Message, Partitioning},
    users::defaults::{DEFAULT_ROOT_PASSWORD, DEFAULT_ROOT_USERNAME},
    utils::{duration::IggyDuration, expiry::IggyExpiry, topic_size::MaxTopicSize},
};
use market_making::{
    book::{OrderBook, SnapshotRequest},
    data::binance::BinanceClient,
};
use tokio::sync::mpsc;
use tracing::{error, info};

async fn create_producer(client: &IggyClient, stream: &str, topic: &str) -> Result<IggyProducer> {
    let mut procuder = client
        .producer(stream, topic)
        .context("Failed to create producer")?
        .batch_size(1000)
        .send_interval(IggyDuration::from_str("1ms")?)
        .partitioning(Partitioning::balanced())
        .create_stream_if_not_exists()
        .create_topic_if_not_exists(
            1,
            None,
            IggyExpiry::ServerDefault,
            MaxTopicSize::ServerDefault,
        )
        .build();

    procuder
        .init()
        .await
        .context("Failed to initialize producer")?;
    Ok(procuder)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::fmt()
        .with_env_filter("warn,iggy=info,binance_sdk=info")
        .init();
    let symbol = "BTCUSDT".to_string();

    let client = IggyClient::from_connection_string("iggy://iggy:Secret123!@localhost:5100")
        .expect("Failed to create Iggy client");
    client.connect().await?;
    client.ping().await.expect("Failed to ping Iggy server");

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
    // Create producers
    let depth_producer = create_producer(&client, &symbol, "diff_book_depth").await?;
    let ticker_producer = create_producer(&client, &symbol, "ticker").await?;
    let ticker_window_producer =
        create_producer(&client, &symbol, "rolling_window_ticker_1h").await?;
    let snapshot_producer = create_producer(&client, &symbol, "depth_snapshot").await?;
    let orderbook_events_producer = create_producer(&client, &symbol, "orderbook_events").await?;

    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();

    let depth_task = tokio::spawn(async move {
        info!("Starting depth processing task");
        while let Some(depth) = depth_rx.recv().await {
            if let Err(e) = depth_producer
                .send_one(
                    Message::from_str(&serde_json::to_string(&depth)?)
                        .context("Failed to create message from depth")?,
                )
                .await
            {
                error!("Failed to send depth message: {}", e);
                break;
            }
            info!("Depth: {:?}", depth);
        }
        info!("Depth processing task completed");
        Ok::<_, anyhow::Error>(())
    });

    let ticker_task = tokio::spawn(async move {
        info!("Starting ticker processing task");
        while let Some(ticker) = ticker_rx.recv().await {
            if let Err(e) = ticker_producer
                .send_one(
                    Message::from_str(&serde_json::to_string(&ticker)?)
                        .context("Failed to create message from ticker")?,
                )
                .await
            {
                error!("Failed to send ticker message: {}", e);
                break;
            }
            info!("Ticker: {:?}", ticker);
        }
        info!("Ticker processing task completed");
        Ok::<_, anyhow::Error>(())
    });

    let ticker_window_task = tokio::spawn(async move {
        info!("Starting rolling window ticker processing task");
        while let Some(ticker) = ticker_window_1h_rx.recv().await {
            if let Err(e) = ticker_window_producer
                .send_one(
                    Message::from_str(&serde_json::to_string(&ticker)?)
                        .context("Failed to create message from rolling window ticker")?,
                )
                .await
            {
                error!("Failed to send rolling window ticker message: {}", e);
                break;
            }
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

            if let Ok(snapshot) = &snapshot {
                if let Err(e) = snapshot_producer
                    .send_one(
                        Message::from_str(&serde_json::to_string(snapshot)?)
                            .context("Failed to create message from snapshot")?,
                    )
                    .await
                {
                    tracing::error!("Failed to send snapshot message: {}", e);
                }
            }

            let _ = response_tx.send(snapshot);
        }
        Ok::<_, anyhow::Error>(())
    });

    info!("Creating order book for symbol: {}", symbol);
    let order_book =
        OrderBook::new(symbol.clone(), Some(1000), depth_rx, snapshot_request_tx).await?;

    let order_book_task = tokio::spawn(async move {
        info!("Running order book for symbol: {}", symbol);
        order_book.run().await
    });

    let results = tokio::try_join!(
        depth_task,
        ticker_task,
        ticker_window_task,
        snapshot_task,
        order_book_task
    );

    match results {
        Ok((depth_res, ticker_res, window_res, snapshot_res, book_res)) => {
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

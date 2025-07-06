#![allow(unused_variables)]
#![allow(dead_code)]
use std::time::Duration;

use anyhow::Result;
use binance_spot_connector_rust::{
    hyper::BinanceHttpClient,
    market::{self},
    market_stream::{
        agg_trade::AggTradeStream, diff_depth::DiffDepthStream,
        rolling_window_ticker::RollingWindowTickerStream, ticker::TickerStream,
    },
    tokio_tungstenite::BinanceWebSocketClient,
};
use chrono::prelude::*;
use futures_util::StreamExt;
use marketmakerlib::{
    data::binance::{
        models::{
            AggregateTrade, BinanceEvent, DepthSnapshot, DepthUpdate, TickerData, WindowTickerData,
        },
        protocol::BinanceMessage,
    },
    order_book_state::OrderBookState,
};
use surrealdb::{Surreal, engine::remote::ws::Ws, opt::auth::Root};
use tokio::select;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    let db = Surreal::new::<Ws>("127.0.0.1:8000").await?;

    db.signin(Root {
        username: "root",
        password: "root",
    })
    .await?;

    let utc: DateTime<Utc> = Utc::now();
    let db_name = format!("binance-{utc}");

    &db_name;

    db.use_ns("order-book").use_db(db_name).await?;

    tracing_subscriber::fmt::init();
    info!("Running!");

    let mut order_book_state = OrderBookState::default();

    let client = BinanceHttpClient::default();
    // Establish connection
    let (mut conn, _) = BinanceWebSocketClient::connect_async_default()
        .await
        .expect("Failed to connect");

    let symbol = "BTCUSDT";

    let (message_tx, mut message_rx) = tokio::sync::mpsc::channel(10_000);

    let (depth_tx, mut depth_rx) = tokio::sync::mpsc::channel(2_000);
    let (agg_tx, mut agg_rx) = tokio::sync::mpsc::channel(2_000);
    let (ticker_tx, mut ticker_rx) = tokio::sync::mpsc::channel(500);
    let (window_ticker_tx, mut window_ticker_rx) = tokio::sync::mpsc::channel(500);

    // Subscribe to streams
    conn.subscribe(vec![
        &DiffDepthStream::from_100ms(symbol).into(),
        &AggTradeStream::new(symbol).into(),
        &TickerStream::from_symbol(symbol).into(),
        // &KlineStream::new(symbol, KlineInterval::Minutes1).into(),
        // &KlineStream::new(symbol, KlineInterval::Minutes3).into(),
        // &KlineStream::new(symbol, KlineInterval::Minutes15).into(),
        // &KlineStream::new(symbol, KlineInterval::Minutes30).into(),
        &RollingWindowTickerStream::from_symbol("1h", symbol).into(),
    ])
    .await;
    //     //&AvgPriceStream::new(symbol).into(),
    //     //&TradeStream::new(symbol).into(),
    //     //&KlineStream::new(symbol, KlineInterval::Minutes1).into(),
    //     &DiffDepthStream::from_100ms(symbol).into(),
    //     &AggTradeStream::new(symbol).into(),
    //     //&BookTickerStream::from_symbol(symbol).into(),
    // ])
    // .await;

    // Start a timer for 10 seconds
    let timer = tokio::time::Instant::now();
    let duration = Duration::new(60 * 60 * 24, 0);
    // Initialize counters and timing
    let start_time = tokio::time::Instant::now();
    let mut last_check = start_time;
    let mut total_messages = 0;
    let mut messages_since_last_check = 0;
    let check_interval = Duration::from_secs(1); // Check every second

    let stream_handler = tokio::spawn(async move {
        while let Some(message) = conn.as_mut().next().await {
            match message {
                Ok(message) => message_tx.send(message).await?,
                Err(_) => break,
            }
            if timer.elapsed() >= duration {
                info!("10 seconds elapsed, exiting loop.");
                break; // Exit the loop after 10 seconds
            }
        }
        conn.close().await.expect("Failed to close connection");
        info!("Exiting stream handler, closed connection");
        Ok::<_, anyhow::Error>(())
    });

    let sender = tokio::spawn(async move {
        while let Some(message) = message_rx.recv().await {
            total_messages += 1;
            messages_since_last_check += 1;
            // Check throughput every second
            if last_check.elapsed() >= check_interval {
                let pending = message_rx.len();
                let messages_per_second =
                    f64::from(messages_since_last_check) / last_check.elapsed().as_secs_f64();

                info!(
                    "Throughput: {:.2} msgs/sec, Total: {}, Pending: {}",
                    messages_per_second, total_messages, pending
                );
                if pending >= 100 {
                    warn!("Back-logged");
                }

                messages_since_last_check = 0;
                last_check = tokio::time::Instant::now();
            }

            let binary_data = message.into_text()?;
            match BinanceMessage::from_str_into_market_data(&binary_data) {
                Ok(event) => match event {
                    BinanceEvent::AggTrade(trade) => {
                        agg_tx.send(trade).await.expect("Failed to send trade");
                    }
                    BinanceEvent::DepthUpdate(depth) => {
                        depth_tx.send(depth).await.expect("Failed to send depth");
                    }
                    BinanceEvent::BookTicker(ticker) => (),
                    BinanceEvent::MiniTicker(ticker) => (),
                    BinanceEvent::Ticker(ticker) => {
                        ticker_tx.send(ticker).await.expect("Failed to send ticker");
                    }
                    BinanceEvent::AvgPrice(avg_price) => (),
                    BinanceEvent::Kline(kline) => (),
                    BinanceEvent::Trade(trade) => (),
                    BinanceEvent::WindowTicker(ticker) => {
                        window_ticker_tx
                            .send(ticker)
                            .await
                            .expect("Failed to send window ticker");
                    }
                },
                Err(e) => {
                    if let Some(e) = e {
                        error!("Failed to parse event: {}", e);
                        error!(
                            "Data: {:?}",
                            serde_json::from_str::<serde_json::Value>(&binary_data)
                        );
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    warn!("Sleeping for 5 seconds to allow for snapshot processing...");
    tokio::time::sleep(Duration::from_secs(5)).await;
    warn!("Waking up...");
    let data = client
        .send(market::depth(symbol).limit(5_000))
        .await
        .expect("Failed to get depth")
        .into_body_str()
        .await
        .expect("Failed to read response body");
    let snapshot =
        serde_json::from_str::<DepthSnapshot>(&data).expect("Failed to parse depth snapshot");

    order_book_state.apply_snapshot(&snapshot);
    db.create::<Option<DepthSnapshot>>(("depth_snapshots", Utc::now().timestamp_micros()))
        .content(snapshot)
        .await
        .expect("Failed to insert depth snapshot into database");

    // Start normal processing
    info!("Starting normal update processing...");

    let mut i = 0;
    loop {
        i += 1;
        select! {
            Some(depth) = depth_rx.recv() => {
                info!("Depth Update");
                order_book_state.process_update(&depth)?;
                db.create::<Option<DepthUpdate>>(("depth_updates",Utc::now().timestamp_micros()))
                    .content(depth)
                    .await
                    .expect("Failed to insert depth update into database");
            }
            Some(agg) = agg_rx.recv() => {
                info!("AggTrade");

                db.create::<Option<AggregateTrade>>(("aggregate_trades",Utc::now().timestamp_micros()))
                    .content(agg)
                    .await
                    .expect("Failed to insert aggregate trade into database");
            }
            Some(ticker) = ticker_rx.recv() => {
                info!("Ticker");
                db.create::<Option<TickerData>>(("tickers",Utc::now().timestamp_micros()))
                    .content(ticker)
                    .await
                    .expect("Failed to insert ticker into database");

            }
            Some(window_ticker) = window_ticker_rx.recv() => {
                info!("WindowTicker");
                db.create::<Option<WindowTickerData>>(("window_tickers",Utc::now().timestamp_micros()))
                    .content(window_ticker)
                    .await
                    .expect("Failed to insert window ticker into database");
            }
            else => {
                break;
            }


        }
        if timer.elapsed() >= duration {
            info!("10 seconds elapsed, exiting loop.");
            break; // Exit the loop after 10 seconds
        }
    }

    drop(depth_rx);
    drop(agg_rx);

    let (_, _) = tokio::join!(stream_handler, sender);
    info!("Exiting main loop");

    let total_time = start_time.elapsed();
    let average_throughput = f64::from(total_messages) / total_time.as_secs_f64();
    info!(
        "Final stats - Total messages: {}, Average throughput: {:.2} msgs/sec, Total time: {:.2}s",
        total_messages,
        average_throughput,
        total_time.as_secs_f64()
    );
    Ok(())
}

use anyhow::Result;
use binance_spot_connector_rust::{
    hyper::BinanceHttpClient,
    market,
    market_stream::{agg_trade::AggTradeStream, diff_depth::DiffDepthStream},
    tokio_tungstenite::BinanceWebSocketClient,
};
use futures_util::StreamExt;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use std::{collections::VecDeque, time::Duration};
use tokio::select;
use tracing::{debug, info, warn};

use marketmakerlib::{
    binance::{
        data::{BinanceEvent, DepthSnapshot},
        BinanceMessage, VolumeProfile,
    },
    order_book_state::OrderBookState,
    recent_trades::RecentTrades,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    info!("Running!");

    let mut order_book_state = OrderBookState::default();

    let client = BinanceHttpClient::default();
    // Establish connection
    let (mut conn, _) = BinanceWebSocketClient::connect_async_default()
        .await
        .expect("Failed to connect");

    let symbol = "BTCUSDT";

    let (message_tx, mut message_rx) = tokio::sync::mpsc::channel(1000);

    let (depth_tx, mut depth_rx) = tokio::sync::mpsc::channel(200);
    let (agg_tx, mut agg_rx) = tokio::sync::mpsc::channel(200);

    // Subscribe to streams
    conn.subscribe(vec![
        //&AvgPriceStream::new(symbol).into(),
        //&TradeStream::new(symbol).into(),
        //&KlineStream::new(symbol, KlineInterval::Minutes1).into(),
        &DiffDepthStream::from_100ms(symbol).into(),
        &AggTradeStream::new(symbol).into(),
        //&BookTickerStream::from_symbol(symbol).into(),
    ])
    .await;

    // Start a timer for 10 seconds
    let timer = tokio::time::Instant::now();
    let duration = Duration::new(200, 0);
    // Initialize counters and timing
    let start_time = tokio::time::Instant::now();
    let mut last_check = start_time;
    let mut total_messages = 0;
    let mut messages_since_last_check = 0;
    let check_interval = Duration::from_secs(1); // Check every second

    let stream_handler = tokio::spawn(async move {
        while let Some(message) = conn.as_mut().next().await {
            total_messages += 1;
            messages_since_last_check += 1;
            // Check throughput every second
            if last_check.elapsed() >= check_interval {
                let messages_per_second =
                    messages_since_last_check as f64 / last_check.elapsed().as_secs_f64();

                let pending = 1; //conn.as_mut().count().await;

                log::info!(
                    "Throughput: {:.2} msgs/sec, Total: {}, Pending: {}",
                    messages_per_second,
                    total_messages,
                    pending
                );

                // Reset counters
                messages_since_last_check = 0;
                last_check = tokio::time::Instant::now();
            }

            match message {
                Ok(message) => message_tx.send(message).await?,
                Err(_) => break,
            }
            if timer.elapsed() >= duration {
                log::info!("10 seconds elapsed, exiting loop.");
                break; // Exit the loop after 10 seconds
            }
        }
        conn.close().await.expect("Failed to close connection");
        info!("Exiting stream handler, closed connection");
        Ok::<_, anyhow::Error>(())
    });

    let sender = tokio::spawn(async move {
        while let Some(message) = message_rx.recv().await {
            let binary_data = message.into_text()?;
            match BinanceMessage::from_str_into_market_data(&binary_data) {
                Ok(event) => match event {
                    BinanceEvent::AggTrade(trade) => {
                        info!("AggTrade");
                        agg_tx.send(trade).await.expect("Failed to send trade");
                    }
                    BinanceEvent::DepthUpdate(depth) => {
                        info!("Depth update");
                        depth_tx.send(depth).await.expect("Failed to send depth");
                    }
                    _ => {
                        warn!("Unknown event: {:?}", event);
                    }
                },
                Err(e) => {
                    if let Some(e) = e {
                        log::error!("Failed to parse event: {}", e);
                        log::error!(
                            "Data: {:?}",
                            serde_json::from_str::<serde_json::Value>(&binary_data)
                        );
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });

    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut vp = VolumeProfile::new(20.into());
    let mut rt = RecentTrades::new(50);
    let data = client
        .send(market::depth(symbol).limit(5_000))
        .await
        .expect("Failed to get depth")
        .into_body_str()
        .await
        .expect("Failed to read response body");
    let snapshot =
        serde_json::from_str::<DepthSnapshot>(&data).expect("Failed to parse depth snapshot");

    order_book_state.apply_snapshot(snapshot);

    info!("Processing buffered updates...");
    let mut buffer = Vec::new();
    depth_rx.recv_many(&mut buffer, usize::MAX).await;
    let buffer = buffer.into_iter().collect::<VecDeque<_>>();

    order_book_state.process_buffer(buffer)?;
    // Start normal processing
    info!("Starting normal update processing...");

    let mut k = Decimal::ZERO;
    let base_k = Decimal::ONE;
    let mut mid = Decimal::ZERO;
    let mut last_stink = Decimal::MIN;
    let mut spread = Decimal::ZERO;
    let mut imbalance = Decimal::ZERO;
    let mut sigma = Decimal::ZERO;
    let mut best_bid = Decimal::MIN;

    let mut buffer = Vec::new();
    agg_rx.recv_many(&mut buffer, usize::MAX).await;
    rt.update_many(buffer.into_iter());

    loop {
        select! {
            Some(depth) = depth_rx.recv() => {
                order_book_state.process_update(depth)?;
                let s = order_book_state.spread().unwrap();
                info!("Updated spread: {} -> {}  diff {}", spread, s, spread - s);
                spread = s;
                let i = order_book_state.imbalance().unwrap();
                info!("Updated imbalance: {} -> {}", imbalance, i);
                imbalance = i;
                k = if imbalance < Decimal::from_f32(-0.3).unwrap() {
                    base_k + Decimal::ONE
                } else {
                    base_k
                };
                let m = order_book_state.mid_price().unwrap();
                info!("Updated mid-price: {} -> {}", mid, m);
                mid = m;
                let ba = order_book_state.best_bid().unwrap();
                info!("New best Bid: {} -> {}", best_bid, ba);
                best_bid = ba;

            }
            Some(trade) = agg_rx.recv() => {
                if trade.buyer_market_maker && (trade.price <= last_stink)  {
                    warn!("Stink Bid HIT! Bid: {} - Trade: {}", last_stink, trade.price);
                }
                rt.update(trade);
                let s = rt.calculate_volitility().unwrap();
                info!("Updated sigma: {} -> {}", sigma, s);
                if s > (sigma * Decimal::from_f32(1.2).unwrap()) {
                    info!("Vol increased, should potentially move bids");
                }
                sigma = s;
                if spread > sigma {
                    info!("Wide enough for profit");
                    continue;
                }

            }
            else => {
                break;
            }
        }

        let stink_bid = mid - (k * sigma);
        info!("New Stink bid: {}", stink_bid);
        last_stink = stink_bid;

        if timer.elapsed() >= duration {
            log::info!("10 seconds elapsed, exiting loop.");
            break; // Exit the loop after 10 seconds
        }
    }
    drop(depth_rx);
    drop(agg_rx);
    let (_, _) = tokio::join!(stream_handler, sender);
    info!("Exiting main loop");

    // At the end, show final statistics
    let total_time = start_time.elapsed();
    let average_throughput = total_messages as f64 / total_time.as_secs_f64();
    log::info!(
        "Final stats - Total messages: {}, Average throughput: {:.2} msgs/sec, Total time: {:.2}s",
        total_messages,
        average_throughput,
        total_time.as_secs_f64()
    );
    log::info!("Volume profile: {:?}", vp);
    Ok(())
}

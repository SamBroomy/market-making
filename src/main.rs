use anyhow::Result;
use binance_spot_connector_rust::{
    hyper::BinanceHttpClient,
    market::{self, klines::KlineInterval},
    market_stream::{
        agg_trade::AggTradeStream, avg_price::AvgPriceStream, book_ticker::BookTickerStream,
        diff_depth::DiffDepthStream, kline::KlineStream, mini_ticker::MiniTickerStream,
        rolling_window_ticker::RollingWindowTickerStream, ticker::TickerStream, trade::TradeStream,
    },
    tokio_tungstenite::BinanceWebSocketClient,
};
use futures_util::StreamExt;
use rust_decimal::{Decimal, prelude::FromPrimitive};
use std::{collections::VecDeque, time::Duration};
use tokio::select;
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;

use marketmakerlib::{
    binance::{
        BinanceMessage, VolumeProfile,
        data::{AveragePrice, BinanceEvent, DepthSnapshot},
    },
    market_maker::{MarketMaker, MarketMakerConfig},
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

    let (message_tx, mut message_rx) = tokio::sync::mpsc::channel(10_000);

    let (depth_tx, mut depth_rx) = tokio::sync::mpsc::channel(2_000);
    let (agg_tx, mut agg_rx) = tokio::sync::mpsc::channel(2_000);
    let (book_ticker_tx, mut book_ticker_rx) = tokio::sync::mpsc::channel(5_000);
    let (mini_ticker_tx, mut mini_ticker_rx) = tokio::sync::mpsc::channel(500);
    let (ticker_tx, mut ticker_rx) = tokio::sync::mpsc::channel(500);
    let (avg_price_tx, mut avg_price_rx) = tokio::sync::mpsc::channel(500);
    let (kline_tx, mut kline_rx) = tokio::sync::mpsc::channel(500);
    let (trade_tx, mut trade_rx) = tokio::sync::mpsc::channel(500);
    let (window_ticker_tx, mut window_ticker_rx) = tokio::sync::mpsc::channel(500);

    // Subscribe to streams
    conn.subscribe(vec![
        &DiffDepthStream::from_100ms(symbol).into(),
        &AggTradeStream::new(symbol).into(),
        &BookTickerStream::from_symbol(symbol).into(),
        &MiniTickerStream::from_symbol(symbol).into(),
        &TickerStream::from_symbol(symbol).into(),
        &AvgPriceStream::new(symbol).into(),
        &KlineStream::new(symbol, KlineInterval::Minutes3).into(),
        //&TradeStream::new(symbol).into(),
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
    let duration = Duration::new(500, 0);
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
                    messages_since_last_check as f64 / last_check.elapsed().as_secs_f64();

                info!(
                    "Throughput: {:.2} msgs/sec, Total: {}, Pending: {}",
                    messages_per_second, total_messages, pending
                );
                if pending >= 100 {
                    warn!("Back-logged")
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
                    BinanceEvent::BookTicker(ticker) => {
                        book_ticker_tx
                            .send(ticker)
                            .await
                            .expect("Failed to send book ticker");
                    }
                    BinanceEvent::MiniTicker(ticker) => {
                        mini_ticker_tx
                            .send(ticker)
                            .await
                            .expect("Failed to send mini ticker");
                    }
                    BinanceEvent::Ticker(ticker) => {
                        ticker_tx.send(ticker).await.expect("Failed to send ticker");
                    }
                    BinanceEvent::AvgPrice(avg_price) => {
                        avg_price_tx
                            .send(avg_price)
                            .await
                            .expect("Failed to send avg price");
                    }
                    BinanceEvent::Kline(kline) => {
                        kline_tx.send(kline).await.expect("Failed to send kline");
                    }
                    BinanceEvent::Trade(trade) => {
                        trade_tx.send(trade).await.expect("Failed to send trade");
                    }
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
    let mut rt = RecentTrades::new(100);
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
    let mut buffer = Vec::new();
    agg_rx.recv_many(&mut buffer, usize::MAX).await;
    rt.update_many(buffer.into_iter());
    let mut market_maker = MarketMaker::new(MarketMakerConfig::default(), order_book_state, rt);
    let mut i = 0;
    loop {
        i += 1;
        select! {
            Some(depth) = depth_rx.recv() => {
                info!("Depth Update");
                market_maker.handle_depth_update(depth)?;
            }
            Some(trade) = agg_rx.recv() => {
                info!("AggTrade");
                market_maker.handle_trade(trade)?;
            }
            Some(book_ticker) = book_ticker_rx.recv() => {
                info!("BookTicker: {:?}", book_ticker);

            }
            Some(mini_ticker) = mini_ticker_rx.recv() => {
                info!("Mini Ticker");

                debug!("MiniTicker: {:?}", mini_ticker);

            }
            Some(ticker) = ticker_rx.recv() => {
                info!("Ticker");
                debug!("Ticker: {:?}", ticker);
            }
            Some(avg_price) = avg_price_rx.recv() => {
                info!("AvgPrice");
                debug!("AvgPrice: {:?}", avg_price);

            }
            Some(kline) = kline_rx.recv() => {
                info!("Kline");
                debug!("Kline: {:?}", kline);
            }
            Some(trade) = trade_rx.recv() => {
                info!("Trade");
                debug!("Trade: {:?}", trade);
            }
            Some(window_ticker) = window_ticker_rx.recv() => {
                info!("WindowTicker");
                debug!("WindowTicker: {:?}", window_ticker);
            }
            else => {
                break;
            }


        }

        if i % 100 == 0 {
            info!("Statistics: {}", market_maker.get_statistics());
            i = 0;
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

    info!("{:?}", market_maker);

    let total_time = start_time.elapsed();
    let average_throughput = total_messages as f64 / total_time.as_secs_f64();
    info!(
        "Final stats - Total messages: {}, Average throughput: {:.2} msgs/sec, Total time: {:.2}s",
        total_messages,
        average_throughput,
        total_time.as_secs_f64()
    );
    Ok(())
}

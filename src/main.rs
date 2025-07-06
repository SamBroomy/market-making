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
use binance_spot_connector_rust::market::depth;
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
use tracing::info;

async fn create_producer(client: &IggyClient, stream: &str, topic: &str) -> Result<IggyProducer> {
    let mut procuder = client
        .producer(stream, topic)
        .context("Failed to create producer")?
        .batch_size(1000)
        .send_interval(IggyDuration::from_str("1ms")?)
        .partitioning(Partitioning::balanced())
        .create_stream_if_not_exists()
        .create_topic_if_not_exists(1, None, IggyExpiry::NeverExpire, MaxTopicSize::Unlimited)
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
        .with_env_filter("debug")
        .init();
    let symbol = "BTCUSDT".to_string();

    let client = IggyClient::from_connection_string("iggy://iggy:Secret123!@localhost:5100")
        .expect("Failed to create Iggy client");
    client.connect().await?;
    client.ping().await.expect("Failed to ping Iggy server");

    // let mut producer = client
    //     .producer(&symbol, "diff_book_depth")
    //     .expect("Failed to create producer")
    //     .batch_size(1000)
    //     .send_interval(IggyDuration::from_str("1ms")?)
    //     .partitioning(Partitioning::balanced())
    //     .create_stream_if_not_exists()
    //     .create_topic_if_not_exists(1, None, IggyExpiry::NeverExpire, MaxTopicSize::Unlimited)
    //     .build();
    // producer
    //     .init()
    //     .await
    //     .expect("Failed to initialize producer");

    // let messages = vec![Message::from_str("hello")?, Message::from_str("world")?];
    // producer.send(messages).await?;

    println!("Starting Binance Order Book Example");
    info!("Running!");

    let bc = BinanceClient::new().await;

    let depth_rx = bc
        .diff_book_depth(
            DiffBookDepthParams::builder(symbol.clone())
                .update_speed("100ms".to_string())
                .build()?,
        )
        .await?;

    let depth_rx_clone = depth_rx.clone();

    let depth_producer = create_producer(&client, &symbol, "diff_book_depth").await?;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        info!("Tokio runtime started for depth");
        rt.block_on(async move {
            info!("Starting depth processing");
            while let Ok(depth) = depth_rx_clone.clone().recv() {
                depth_producer
                    .send(vec![
                        Message::from_str(&serde_json::to_string(&depth).unwrap())
                            .expect("Failed to create message from depth"),
                    ])
                    .await
                    .expect("Failed to send depth message");

                info!("Depth: {:?}", depth);
            }
        });
    });

    let ticker_rx = bc
        .ticker(TickerParams::builder(symbol.clone()).build()?)
        .await?;

    let ticker_producer = create_producer(&client, &symbol, "ticker").await?;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        info!("Tokio runtime started for ticker");
        rt.block_on(async move {
            info!("Starting ticker processing");
            while let Ok(ticker) = ticker_rx.recv() {
                ticker_producer
                    .send(vec![
                        Message::from_str(&serde_json::to_string(&ticker).unwrap())
                            .expect("Failed to create message from ticker"),
                    ])
                    .await
                    .expect("Failed to send ticker message");

                info!("Ticker: {:?}", ticker);
            }
        });
    });

    let ticker_window_1h_rx = bc
        .rolling_window_ticker(
            RollingWindowTickerParams::builder(
                symbol.clone(),
                RollingWindowTickerWindowSizeEnum::WindowSize1h,
            )
            .build()?,
        )
        .await?;

    let ticker_window_producer =
        create_producer(&client, &symbol, "rolling_window_ticker_1h").await?;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        info!("Tokio runtime started for rolling window ticker");
        rt.block_on(async move {
            info!("Starting rolling window ticker processing");
            while let Ok(ticker) = ticker_window_1h_rx.recv() {
                ticker_window_producer
                    .send(vec![
                        Message::from_str(&serde_json::to_string(&ticker).unwrap())
                            .expect("Failed to create message from rolling window ticker"),
                    ])
                    .await
                    .expect("Failed to send rolling window ticker message");
                info!("Rolling Window Ticker 1h: {:?}", ticker);
            }
        });
    });

    let snapshot_producer = create_producer(&client, &symbol, "depth_snapshot").await?;

    let (snapshot_request_tx, snapshot_request_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        let bc = bc.clone();
        let mut rx = snapshot_request_rx;

        while let Some(SnapshotRequest {
            symbol,
            limit,
            response_tx,
        }) = rx.recv().await
        {
            info!(symbol, "Received snapshot request with limit: {:?}", limit);

            // Request the depth snapshot from Binance
            let snapshot = bc
                .depth_snapshot(DepthParams::builder(symbol).limit(limit).build()?)
                .await?
                .data()
                .await
                .context("Failed to get depth snapshot");

            if let Ok(snapshot) = &snapshot {
                snapshot_producer
                    .send_one(
                        Message::from_str(
                            &serde_json::to_string(snapshot).expect("Failed to serialize snapshot"),
                        )
                        .expect("Failed to create message from snapshot"),
                    )
                    .await
                    .expect("Failed to send snapshot message");
            }

            let _ = response_tx.send(snapshot);
        }

        Ok::<_, anyhow::Error>(())
    });

    info!("Creating order book for symbol: {}", symbol);

    let order_book =
        OrderBook::new(symbol.clone(), Some(1000), depth_rx, snapshot_request_tx).await?;

    info!("Running order book for symbol: {}", symbol);
    let handle = order_book.run();
    match handle.join() {
        Ok(result) => result?,
        Err(e) => return Err(anyhow::anyhow!("Order book thread panicked: {:?}", e)),
    }

    info!("Order book processing completed for symbol: {}", symbol);

    Ok(())
}
//     let mut order_book_state = OrderBookState::default();

//     let client = BinanceHttpClient::default();
//     // Establish connection
//     let (mut conn, _) = BinanceWebSocketClient::connect_async_default()
//         .await
//         .expect("Failed to connect");

//     let symbol = "BTCUSDT";

//     let (message_tx, mut message_rx) = tokio::sync::mpsc::channel(10_000);

//     let (depth_tx, mut depth_rx) = tokio::sync::mpsc::channel(2_000);
//     let (agg_tx, mut agg_rx) = tokio::sync::mpsc::channel(2_000);
//     let (book_ticker_tx, mut book_ticker_rx) = tokio::sync::mpsc::channel(5_000);
//     let (mini_ticker_tx, mut mini_ticker_rx) = tokio::sync::mpsc::channel(500);
//     let (ticker_tx, mut ticker_rx) = tokio::sync::mpsc::channel(500);
//     let (avg_price_tx, mut avg_price_rx) = tokio::sync::mpsc::channel(500);
//     let (kline_tx, mut kline_rx) = tokio::sync::mpsc::channel(500);
//     let (trade_tx, mut trade_rx) = tokio::sync::mpsc::channel(500);
//     let (window_ticker_tx, mut window_ticker_rx) = tokio::sync::mpsc::channel(500);

//     // Subscribe to streams
//     conn.subscribe(vec![
//         &DiffDepthStream::from_100ms(symbol).into(),
//         &AggTradeStream::new(symbol).into(),
//         &BookTickerStream::from_symbol(symbol).into(),
//         &MiniTickerStream::from_symbol(symbol).into(),
//         &TickerStream::from_symbol(symbol).into(),
//         &AvgPriceStream::new(symbol).into(),
//         &KlineStream::new(symbol, KlineInterval::Minutes3).into(),
//         //&TradeStream::new(symbol).into(),
//         &RollingWindowTickerStream::from_symbol("1h", symbol).into(),
//     ])
//     .await;
//     //     //&AvgPriceStream::new(symbol).into(),
//     //     //&TradeStream::new(symbol).into(),
//     //     //&KlineStream::new(symbol, KlineInterval::Minutes1).into(),
//     //     &DiffDepthStream::from_100ms(symbol).into(),
//     //     &AggTradeStream::new(symbol).into(),
//     //     //&BookTickerStream::from_symbol(symbol).into(),
//     // ])
//     // .await;

//     // Start a timer for 10 seconds
//     let timer = tokio::time::Instant::now();
//     let duration = Duration::new(10, 0);
//     // Initialize counters and timing
//     let start_time = tokio::time::Instant::now();
//     let mut last_check = start_time;
//     let mut total_messages = 0;
//     let mut messages_since_last_check = 0;
//     let check_interval = Duration::from_secs(1); // Check every second

//     let stream_handler = tokio::spawn(async move {
//         while let Some(message) = conn.as_mut().next().await {
//             match message {
//                 Ok(message) => message_tx.send(message).await?,
//                 Err(_) => break,
//             }
//             if timer.elapsed() >= duration {
//                 info!("10 seconds elapsed, exiting loop.");
//                 break; // Exit the loop after 10 seconds
//             }
//         }
//         conn.close().await.expect("Failed to close connection");
//         info!("Exiting stream handler, closed connection");
//         Ok::<_, anyhow::Error>(())
//     });

//     let sender = tokio::spawn(async move {
//         while let Some(message) = message_rx.recv().await {
//             total_messages += 1;
//             messages_since_last_check += 1;
//             // Check throughput every second
//             if last_check.elapsed() >= check_interval {
//                 let pending = message_rx.len();
//                 let messages_per_second =
//                     messages_since_last_check as f64 / last_check.elapsed().as_secs_f64();

//                 info!(
//                     "Throughput: {:.2} msgs/sec, Total: {}, Pending: {}",
//                     messages_per_second, total_messages, pending
//                 );
//                 if pending >= 100 {
//                     warn!("Back-logged")
//                 }

//                 messages_since_last_check = 0;
//                 last_check = tokio::time::Instant::now();
//             }

//             let binary_data = message.into_text()?;
//             match BinanceMessage::from_str_into_market_data(&binary_data) {
//                 Ok(event) => match event {
//                     BinanceEvent::AggTrade(trade) => {
//                         agg_tx.send(trade).await.expect("Failed to send trade");
//                     }
//                     BinanceEvent::DepthUpdate(depth) => {
//                         depth_tx.send(depth).await.expect("Failed to send depth");
//                     }
//                     BinanceEvent::BookTicker(ticker) => {
//                         book_ticker_tx
//                             .send(ticker)
//                             .await
//                             .expect("Failed to send book ticker");
//                     }
//                     BinanceEvent::MiniTicker(ticker) => {
//                         mini_ticker_tx
//                             .send(ticker)
//                             .await
//                             .expect("Failed to send mini ticker");
//                     }
//                     BinanceEvent::Ticker(ticker) => {
//                         ticker_tx.send(ticker).await.expect("Failed to send ticker");
//                     }
//                     BinanceEvent::AvgPrice(avg_price) => {
//                         avg_price_tx
//                             .send(avg_price)
//                             .await
//                             .expect("Failed to send avg price");
//                     }
//                     BinanceEvent::Kline(kline) => {
//                         kline_tx.send(kline).await.expect("Failed to send kline");
//                     }
//                     BinanceEvent::Trade(trade) => {
//                         trade_tx.send(trade).await.expect("Failed to send trade");
//                     }
//                     BinanceEvent::WindowTicker(ticker) => {
//                         window_ticker_tx
//                             .send(ticker)
//                             .await
//                             .expect("Failed to send window ticker");
//                     }
//                 },
//                 Err(e) => {
//                     if let Some(e) = e {
//                         error!("Failed to parse event: {}", e);
//                         error!(
//                             "Data: {:?}",
//                             serde_json::from_str::<serde_json::Value>(&binary_data)
//                         );
//                     }
//                 }
//             }
//         }
//         Ok::<_, anyhow::Error>(())
//     });

//     warn!("Sleeping for 5 seconds to allow for snapshot processing...");
//     tokio::time::sleep(Duration::from_secs(5)).await;
//     warn!("Waking up...");
//     let mut rt = RecentTrades::new(100);
//     let data = client
//         .send(market::depth(symbol).limit(5_000))
//         .await
//         .expect("Failed to get depth")
//         .into_body_str()
//         .await
//         .expect("Failed to read response body");
//     let snapshot =
//         serde_json::from_str::<DepthSnapshot>(&data).expect("Failed to parse depth snapshot");

//     order_book_state.apply_snapshot(&snapshot);

//     info!("Processing buffered updates...");
//     let mut buffer = Vec::new();
//     depth_rx.recv_many(&mut buffer, usize::MAX).await;
//     let buffer = buffer.into_iter().collect::<VecDeque<_>>();

//     order_book_state.process_buffer(buffer)?;
//     // Start normal processing
//     info!("Starting normal update processing...");
//     let mut buffer = Vec::new();
//     agg_rx.recv_many(&mut buffer, usize::MAX).await;
//     rt.update_many(buffer.into_iter());
//     let mut market_maker = MarketMaker::new(MarketMakerConfig::default(), order_book_state, rt);
//     let mut i = 0;
//     loop {
//         i += 1;
//         select! {
//             Some(depth) = depth_rx.recv() => {
//                 info!("Depth Update");
//                 market_maker.handle_depth_update(depth)?;
//             }
//             Some(trade) = agg_rx.recv() => {
//                 info!("AggTrade");
//                 market_maker.handle_trade(trade)?;
//             }
//             Some(book_ticker) = book_ticker_rx.recv() => {
//                 info!("BookTicker: {:?}", book_ticker);

//             }
//             Some(mini_ticker) = mini_ticker_rx.recv() => {
//                 info!("Mini Ticker");

//                 debug!("MiniTicker: {:?}", mini_ticker);

//             }
//             Some(ticker) = ticker_rx.recv() => {
//                 info!("Ticker");
//                 debug!("Ticker: {:?}", ticker);
//             }
//             Some(avg_price) = avg_price_rx.recv() => {
//                 info!("AvgPrice");
//                 debug!("AvgPrice: {:?}", avg_price);

//             }
//             Some(kline) = kline_rx.recv() => {
//                 info!("Kline");
//                 debug!("Kline: {:?}", kline);
//             }
//             Some(trade) = trade_rx.recv() => {
//                 info!("Trade");
//                 debug!("Trade: {:?}", trade);
//             }
//             Some(window_ticker) = window_ticker_rx.recv() => {
//                 info!("WindowTicker");
//                 debug!("WindowTicker: {:?}", window_ticker);
//             }
//             else => {
//                 break;
//             }

//         }

//         if i % 100 == 0 {
//             info!("Statistics: {}", market_maker.get_statistics());
//             i = 0;
//         }

//         if timer.elapsed() >= duration {
//             info!("10 seconds elapsed, exiting loop.");
//             break; // Exit the loop after 10 seconds
//         }
//     }

//     drop(depth_rx);
//     drop(agg_rx);

//     let (_, _) = tokio::join!(stream_handler, sender);
//     info!("Exiting main loop");

//     info!("{:?}", market_maker);

//     let total_time = start_time.elapsed();
//     let average_throughput = total_messages as f64 / total_time.as_secs_f64();
//     info!(
//         "Final stats - Total messages: {}, Average throughput: {:.2} msgs/sec, Total time: {:.2}s",
//         total_messages,
//         average_throughput,
//         total_time.as_secs_f64()
//     );
//     Ok(())
// }

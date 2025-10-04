use anyhow::Result;
use domain::models::market_data::{AggregateTrade, DepthUpdate, TickerData, WindowTickerData};
use infrastructure::{messaging::StreamProducer, persist::DataWriter};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{debug, error, warn};

pub async fn process_depth_updates(
    mut rx: UnboundedReceiver<DepthUpdate>,
    producer: Option<StreamProducer>,
    writer: Option<DataWriter>,
    orderbook_tx: Option<UnboundedSender<DepthUpdate>>,
) -> Result<()> {
    debug!("Starting depth update processor");
    loop {
        if let Some(update) = rx.recv().await {
            if let Some(ref w) = writer
                && let Err(e) = w.write_depth_update(&update).await
            {
                error!("Failed to write depth update to database: {}", e);
            }

            if let Some(ref p) = producer
                && let Err(e) = p.send(&update).await
            {
                error!("Failed to send depth update to message queue: {}", e);
            }

            if let Some(ref tx) = orderbook_tx
                && let Err(e) = tx.send(update)
            {
                error!("Failed to send depth update to orderbook channel: {}", e);
            }
        } else {
            warn!("Depth update channel closed");
            break;
        }
    }
    Ok(())
}

pub async fn process_ticker_updates(
    mut rx: UnboundedReceiver<TickerData>,
    producer: Option<StreamProducer>,
    writer: Option<DataWriter>,
) -> Result<()> {
    debug!("Starting ticker update processor");
    while let Some(ticker) = rx.recv().await {
        if let Some(ref w) = writer
            && let Err(e) = w.write_ticker(&ticker).await
        {
            error!("Failed to write ticker to database: {}", e);
        }
        if let Some(ref p) = producer
            && let Err(e) = p.send(&ticker).await
        {
            error!("Failed to send ticker to message queue: {}", e);
        }
    }
    Ok(())
}

pub async fn process_window_ticker_updates(
    mut rx: UnboundedReceiver<WindowTickerData>,
    producer: Option<StreamProducer>,
    writer: Option<DataWriter>,
) -> Result<()> {
    debug!("Starting window ticker update processor");
    while let Some(ticker) = rx.recv().await {
        if let Some(ref w) = writer
            && let Err(e) = w.write_window_ticker(&ticker).await
        {
            error!("Failed to write window ticker to database: {}", e);
        }
        if let Some(ref p) = producer
            && let Err(e) = p.send(&ticker).await
        {
            error!("Failed to send window ticker to message queue: {}", e);
        }
    }
    Ok(())
}

pub async fn process_aggregate_trade_updates(
    mut rx: UnboundedReceiver<AggregateTrade>,
    producer: Option<StreamProducer>,
    writer: Option<DataWriter>,
    trade_tx: Option<UnboundedSender<AggregateTrade>>,
) -> Result<()> {
    debug!("Starting aggregate trade update processor");
    while let Some(trade) = rx.recv().await {
        if let Some(ref w) = writer
            && let Err(e) = w.write_aggregate_trade(&trade).await
        {
            error!("Failed to write aggregate trade to database: {}", e);
        }
        if let Some(ref p) = producer
            && let Err(e) = p.send(&trade).await
        {
            error!("Failed to send aggregate trade to message queue: {}", e);
        }
        if let Some(ref tx) = trade_tx {
            let _ = tx.send(trade);
        }
    }
    Ok(())
}

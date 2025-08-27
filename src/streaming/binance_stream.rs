use std::fmt::Debug;

use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::{DatabaseWriter, MessageProducer};

/// Generic handler for any type of market data
#[async_trait]
pub trait DataHandler<T>: Send + Sync {
    async fn handle_data(&self, data: &T) -> Result<()>;
    async fn handle_data_owned(&self, data: T) -> Result<()>;
}

/// Combines message queue and database writing (both optional)
pub struct DefaultDataHandler {
    message_producer: Option<MessageProducer>,
    database_writer: Option<DatabaseWriter>,
}

impl DefaultDataHandler {
    #[must_use]
    pub fn new(
        message_producer: Option<MessageProducer>,
        database_writer: Option<DatabaseWriter>,
    ) -> Self {
        Self {
            message_producer,
            database_writer,
        }
    }
}

/// Generic Binance data stream processor
pub struct BinanceStream<T> {
    symbol: String,
    stream_name: String,
    handler: Box<dyn DataHandler<T>>,
}

impl<T> BinanceStream<T>
where
    T: Serialize + Debug + Send + 'static,
{
    #[must_use]
    pub fn new(
        symbol: impl Into<String>,
        stream_name: impl Into<String>,
        handler: Box<dyn DataHandler<T>>,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            stream_name: stream_name.into(),
            handler,
        }
    }

    pub async fn run(&self, mut receiver: mpsc::UnboundedReceiver<T>) -> Result<()> {
        info!(
            "Starting {} stream for symbol: {}",
            self.stream_name, self.symbol
        );

        while let Some(data) = receiver.recv().await {
            debug!("{} - {}: {:?}", self.stream_name, self.symbol, data);

            if let Err(e) = self.handler.handle_data_owned(data).await {
                error!(
                    "Failed to handle {} data for {}: {}",
                    self.stream_name, self.symbol, e
                );
            }
        }

        info!("{} stream for {} completed", self.stream_name, self.symbol);
        Ok(())
    }
}

/// Macro to implement `DataHandler` for common market data types
macro_rules! impl_data_handler {
    ($data_type:ty, $db_method:ident) => {
        #[async_trait]
        impl DataHandler<$data_type> for DefaultDataHandler {
            async fn handle_data(&self, data: &$data_type) -> Result<()> {
                // Send to message queue if enabled
                if let Some(ref producer) = self.message_producer {
                    producer.send_json(&data).await?;
                }

                // Write to database if enabled
                if let Some(ref writer) = self.database_writer {
                    writer.$db_method(&data).await?;
                }

                Ok(())
            }

            #[inline]
            async fn handle_data_owned(&self, data: $data_type) -> Result<()> {
                self.handle_data(&data).await
            }
        }
    };
}

// Implement handlers for all market data types
use crate::data::binance::models::{AggregateTrade, DepthUpdate, TickerData, WindowTickerData};

impl_data_handler!(TickerData, write_ticker);
impl_data_handler!(WindowTickerData, write_window_ticker);
impl_data_handler!(AggregateTrade, write_aggregate_trade);
impl_data_handler!(DepthUpdate, write_depth_update);

// Special handler for DepthUpdate (needs forwarding to orderbook)
pub struct DepthUpdateHandler {
    default_handler: DefaultDataHandler,
    orderbook_sender: Option<mpsc::UnboundedSender<DepthUpdate>>,
}

impl DepthUpdateHandler {
    #[must_use]
    pub fn new(
        default_handler: DefaultDataHandler,
        orderbook_sender: Option<mpsc::UnboundedSender<DepthUpdate>>,
    ) -> Self {
        Self {
            default_handler,
            orderbook_sender,
        }
    }
}
#[async_trait]
impl DataHandler<DepthUpdate> for DepthUpdateHandler {
    async fn handle_data_owned(&self, data: DepthUpdate) -> Result<()> {
        // Handle default processing (message queue + database)
        self.default_handler.handle_data(&data).await?;

        let symbol = data.symbol.clone();

        // Forward to orderbook if configured
        if let Some(sender) = &self.orderbook_sender
            && sender.send(data).is_err()
        {
            warn!("Order book receiver for {} is closed", symbol);
        }

        Ok(())
    }

    async fn handle_data(&self, data: &DepthUpdate) -> Result<()> {
        unreachable!("DepthUpdateHandler requires owned data");
    }
}

// Handler for AggTrade events that forwards to TradeProcessor
pub struct AggTradeHandler {
    default_handler: DefaultDataHandler,
    trade_processor_sender: Option<mpsc::UnboundedSender<AggregateTrade>>,
}
impl AggTradeHandler {
    #[must_use]
    pub fn new(
        default_handler: DefaultDataHandler,
        trade_processor_sender: Option<mpsc::UnboundedSender<AggregateTrade>>,
    ) -> Self {
        Self {
            default_handler,
            trade_processor_sender,
        }
    }
}
#[async_trait]
impl DataHandler<AggregateTrade> for AggTradeHandler {
    async fn handle_data_owned(&self, data: AggregateTrade) -> Result<()> {
        self.default_handler.handle_data(&data).await?;

        let symbol = data.symbol.clone();

        if let Some(sender) = &self.trade_processor_sender
            && sender.send(data).is_err()
        {
            warn!("TradeProcessor receiver for {} is closed", symbol);
        }

        Ok(())
    }

    async fn handle_data(&self, data: &AggregateTrade) -> Result<()> {
        unreachable!("AggTradeHandler requires owned data");
    }
}

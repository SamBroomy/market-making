use std::{fmt::Debug, sync::Arc};

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
}

/// Combines message queue and database writing
pub struct DefaultDataHandler {
    message_producer: Arc<MessageProducer>,
    database_writer: DatabaseWriter,
}

impl DefaultDataHandler {
    #[must_use]
    pub fn new(message_producer: Arc<MessageProducer>, database_writer: DatabaseWriter) -> Self {
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
    pub fn new(symbol: String, stream_name: String, handler: Box<dyn DataHandler<T>>) -> Self {
        Self {
            symbol,
            stream_name,
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

            if let Err(e) = self.handler.handle_data(&data).await {
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
                // Send to message queue
                self.message_producer.send_json(data).await?;

                // Write to database
                self.database_writer.$db_method(data).await?;

                Ok(())
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
    async fn handle_data(&self, data: &DepthUpdate) -> Result<()> {
        // Handle default processing (message queue + database)
        self.default_handler.handle_data(data).await?;

        // Forward to orderbook if configured
        if let Some(sender) = &self.orderbook_sender
            && sender.send(data.clone()).is_err()
        {
            warn!("Order book receiver for {} is closed", data.symbol);
        }

        Ok(())
    }
}

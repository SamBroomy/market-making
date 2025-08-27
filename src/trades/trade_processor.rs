use std::time::Duration;

use anyhow::Result;
use chrono::Duration as ChronoDuration;
use tokio::{sync::mpsc, time::interval};
use tracing::{error, info};

use crate::{
    data::binance::models::AggregateTrade,
    streaming::{DatabaseWriter, MessageProducer},
    trades::TradeTracker,
};

pub struct TradeProcessor {
    symbol: String,
    trade_receiver: mpsc::UnboundedReceiver<AggregateTrade>,
    trade_tracker: TradeTracker,
    summary_producer: Option<MessageProducer>,
    database_writer: Option<DatabaseWriter>,
    publish_interval: Duration,
}

impl TradeProcessor {
    #[must_use]
    pub fn new(
        symbol: String,
        trade_receiver: mpsc::UnboundedReceiver<AggregateTrade>,
        window_duration: ChronoDuration,
        publish_interval: Duration,
        summary_producer: Option<MessageProducer>,
        database_writer: Option<DatabaseWriter>,
    ) -> Self {
        Self {
            symbol,
            trade_receiver,
            trade_tracker: TradeTracker::new(window_duration),
            summary_producer,
            database_writer,
            publish_interval,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        info!(
            "Starting TradeProcessor for symbol: {} with {}s publish interval",
            self.symbol,
            self.publish_interval.as_secs()
        );

        let mut publish_timer = interval(self.publish_interval);

        loop {
            tokio::select! {
                // Process incoming trades
                trade = self.trade_receiver.recv() => {
                    if let Some(agg_trade) = trade {
                        self.trade_tracker.add_trade(&agg_trade);
                    } else {
                        info!("Trade receiver closed for {}", self.symbol);
                        break;
                    }
                }

                // Publish summary at intervals
                _ = publish_timer.tick() => {
                    if let Err(e) = self.publish_summary().await {
                        error!("Failed to publish trade summary for {}: {}", self.symbol, e);
                    }
                }
            }
        }

        info!("TradeProcessor completed for symbol: {}", self.symbol);
        Ok(())
    }

    async fn publish_summary(&self) -> Result<()> {
        let summary = self.trade_tracker.summary();

        // Send to message queue if enabled
        if let Some(ref producer) = self.summary_producer
            && let Err(e) = producer.send_json(&summary).await
        {
            error!(
                "Failed to send trade summary to message queue for {}: {}",
                self.symbol, e
            );
        }

        // Write to database if enabled
        if let Some(ref writer) = self.database_writer
            && let Err(e) = writer.write_trade_summary(&summary, &self.symbol).await
        {
            error!(
                "Failed to write trade summary to database for {}: {}",
                self.symbol, e
            );
        }

        Ok(())
    }
}

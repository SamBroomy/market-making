use std::time::Duration;

use anyhow::Result;
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
    publish_interval: Option<Duration>,
}

impl TradeProcessor {
    #[must_use]
    pub fn new(
        symbol: String,
        trade_receiver: mpsc::UnboundedReceiver<AggregateTrade>,
        window_duration: Duration,
        publish_interval: Option<Duration>,
        summary_producer: Option<MessageProducer>,
        database_writer: Option<DatabaseWriter>,
    ) -> Self {
        let window_duration = chrono::Duration::from_std(window_duration)
            .expect("Window duration must be a valid chrono::Duration");
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
            "Starting TradeProcessor for symbol: {} with {:?}s publish interval",
            self.symbol,
            self.publish_interval.map(|d| d.as_secs())
        );

        // Use a very short interval for immediate publishing when None
        let timer_duration = self.publish_interval.unwrap_or(Duration::MAX); // effectively disables interval

        let mut publish_timer = interval(timer_duration);
        let mut new_trade_received = false;

        loop {
            tokio::select! {
                // Process incoming trades
                trade = self.trade_receiver.recv() => {
                    if let Some(agg_trade) = trade {
                        new_trade_received = true;
                        self.trade_tracker.add_trade(&agg_trade);

                        // If no publish interval, publish immediately after each trade
                        if self.publish_interval.is_none() &&
                            let Err(e) = self.publish_summary().await {
                                error!("Failed to publish trade summary for {}: {}", self.symbol, e);
                            }

                    } else {
                        info!("Trade receiver closed for {}", self.symbol);
                        break;
                    }
                }

                // Publish summary at intervals (only when interval is Some)
                _ = publish_timer.tick(), if self.publish_interval.is_some() && new_trade_received => {
                    // TODO: only publish if there are new trades since last publish
                    if let Err(e) = self.publish_summary().await {
                        error!("Failed to publish trade summary for {}: {}", self.symbol, e);
                    }
                    new_trade_received = false;
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

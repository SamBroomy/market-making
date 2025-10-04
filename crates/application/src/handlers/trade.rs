use std::time::Duration;

use anyhow::Result;
use domain::models::{market_data::AggregateTrade, trades::TradeTracker};
use infrastructure::{messaging::StreamProducer, persist::DataWriter};
use tokio::{sync::mpsc, time::interval};
use tracing::{error, info};

pub struct TradeProcessor {
    symbol: String,
    trade_receiver: mpsc::UnboundedReceiver<AggregateTrade>,
    trade_tracker: TradeTracker,
    summary_producer: Option<StreamProducer>,
    database_writer: Option<DataWriter>,
    publish_interval: Option<Duration>,
}

impl TradeProcessor {
    #[must_use]
    pub fn new(
        symbol: String,
        trade_receiver: mpsc::UnboundedReceiver<AggregateTrade>,
        window_duration: Duration,
        publish_interval: Option<Duration>,
        summary_producer: Option<StreamProducer>,
        database_writer: Option<DataWriter>,
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
            symbol = %self.symbol,
            publish_interval_secs = ?self.publish_interval.map(|d| d.as_secs()),
            "Starting trade processor"
        );

        let timer_duration = self.publish_interval.unwrap_or(Duration::MAX);
        let mut publish_timer = interval(timer_duration);
        let mut new_trade_received = false;

        loop {
            tokio::select! {
                trade = self.trade_receiver.recv() => {
                    if let Some(agg_trade) = trade {
                        new_trade_received = true;
                        self.trade_tracker.add_trade(&agg_trade);

                        // If no publish interval, publish immediately after each trade
                        if self.publish_interval.is_none() &&
                            let Err(e) = self.publish_summary().await {
                                error!(
                                    symbol = %self.symbol,
                                    error = %e,
                                    "Failed to publish trade summary"
                                );
                            }

                    } else {
                        info!(
                            symbol = %self.symbol,
                            "Trade receiver closed"
                        );
                        break;
                    }
                }

                _ = publish_timer.tick(), if self.publish_interval.is_some() && new_trade_received => {
                    if let Err(e) = self.publish_summary().await {
                        error!(
                            symbol = %self.symbol,
                            error = %e,
                            "Failed to publish trade summary"
                        );
                    }
                    new_trade_received = false;
                }
            }
        }

        info!(
            symbol = %self.symbol,
            "Trade processor completed"
        );
        Ok(())
    }

    async fn publish_summary(&self) -> Result<()> {
        let summary = self.trade_tracker.summary();

        // Send to message queue if enabled
        if let Some(ref producer) = self.summary_producer
            && let Err(e) = producer.send(&summary).await
        {
            error!(
                symbol = %self.symbol,
                error = %e,
                "Failed to send trade summary to message queue"
            );
        }

        // Write to database if enabled
        if let Some(ref writer) = self.database_writer
            && let Err(e) = writer.write_trade_summary(&summary, &self.symbol).await
        {
            error!(
                symbol = %self.symbol,
                error = %e,
                "Failed to write trade summary to database"
            );
        }

        Ok(())
    }
}

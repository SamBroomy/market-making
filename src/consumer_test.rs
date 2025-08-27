use std::{env, str::FromStr};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use iggy::{
    client::{Client, SystemClient},
    clients::{
        client::IggyClient,
        consumer::{AutoCommit, AutoCommitWhen, IggyConsumer},
    },
    messages::poll_messages::PollingStrategy,
    utils::duration::IggyDuration,
};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt, util::SubscriberInitExt};

const STREAM_NAME: &str = "my-stream";
const TOPIC_NAME: &str = "my-topic";
const PARTITIONS: u32 = 1;
const REPLICAS: u32 = 1;
const PARTITION_NUM: u32 = 0;

async fn create_consumer(client: &IggyClient, stream: &str, topic: &str) -> Result<IggyConsumer> {
    let mut consumer = client
        .consumer_group("my-consumer-group", stream, topic)?
        .auto_commit(AutoCommit::IntervalOrWhen(
            IggyDuration::from_str("1s")?,
            AutoCommitWhen::ConsumingAllMessages,
        ))
        .create_consumer_group_if_not_exists()
        .auto_join_consumer_group()
        .polling_strategy(PollingStrategy::next())
        .poll_interval(IggyDuration::from_str("1ms")?)
        .batch_size(1000)
        .build();
    consumer.init().await?;
    Ok(consumer)
}

#[tokio::main]
async fn main() -> Result<()> {
    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("DEBUG")))
        .init();

    let iggy_connection = env::var("IGGY_CONNECTION_STRING").unwrap_or_else(|_| {
        // Check if we're running in Docker by looking for the iggy hostname
        if env::var("DOCKER_ENV").is_ok() {
            // Running inside Docker - use internal network

            "iggy://iggy:Secret123!@iggy:3000".to_string()
        } else {
            // Running locally - use mapped port

            "iggy://iggy:Secret123!@localhost:5100".to_string()
        }
    });
    println!("Connecting to Iggy message queue at: {iggy_connection}");
    let client = IggyClient::from_connection_string(&iggy_connection)?;

    client.connect().await?;

    client.ping().await.context("Failed to ping Iggy server")?;

    let consumer = create_consumer(&client, STREAM_NAME, TOPIC_NAME)
        .await
        .context("Failed to create consumer")?;

    let stream_handle = tokio::spawn(async move {
        let mut consumer = consumer;
        while let Some(message) = consumer.next().await {
            match message {
                Ok(record) => {
                    let polled_message = record.message;

                    let message = String::from_utf8_lossy(&polled_message.payload);
                    println!(
                        "Consumed message at offset {}: {}",
                        polled_message.offset, message
                    );
                }
                Err(e) => {
                    eprintln!("Error consuming message: {e}");
                }
            }
        }
    });

    let result = tokio::try_join!(stream_handle);

    Ok(())
}

// use std::time::Duration;

// use market_making::{
//     producer::{run_multi_market_producer, shutdown_global_resources},
//     settings::Settings,
//     shutdown::{ShutdownCoordinator, setup_signal_handlers},
// };
// use tracing::{error, info};

// #[tokio::main]
// async fn main() -> Result<()> {
//     // Load configuration from YAML files and environment variables
//     let settings = match Settings::get_configuration() {
//         Ok(settings) => settings,
//         Err(e) => {
//             eprintln!("Failed to load configuration: {e}");
//             std::process::exit(1);
//         }
//     };

//     // Initialize tracing with configured log level
//     tracing_subscriber::fmt::fmt()
//         .with_env_filter(&settings.logging.level)
//         .init();

//     // Print configuration summary
//     settings.print_summary();

//     // Create shutdown coordinator with reduced timeout (most cleanup happens in <5s)
//     let shutdown_timeout = Duration::from_secs(10); // 10 seconds for graceful shutdown
//     let shutdown_coordinator = ShutdownCoordinator::new(shutdown_timeout);

//     // Setup signal handlers for graceful shutdown
//     setup_signal_handlers(shutdown_coordinator.clone());

//     // Run the producer with graceful shutdown
//     let producer_result = run_multi_market_producer(settings, shutdown_coordinator).await;

//     // Shutdown global resources (Iggy client, database pool, etc.)
//     shutdown_global_resources().await;

//     // All cleanup completed - no need to wait further
//     info!("Graceful shutdown completed successfully");

//     // Check producer result
//     match producer_result {
//         Ok(()) => {
//             info!("Market data producer completed successfully");
//             Ok(())
//         }
//         Err(e) => {
//             error!("Producer failed: {}", e);
//             Err(e)
//         }
//     }
// }

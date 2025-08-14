use std::{env, str::FromStr};

use anyhow::{Context, Result};
use chrono::Local;
use iggy::{
    client::{Client, SystemClient},
    clients::{client::IggyClient, producer::IggyProducer},
    messages::send_messages::{Message, Partitioning},
    utils::{duration::IggyDuration, expiry::IggyExpiry, topic_size::MaxTopicSize},
};
use sqlx::PgPool;
//use iggy::prelude::*;
use tracing::info;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt, util::SubscriberInitExt};
const STREAM_NAME: &str = "my-stream";
const TOPIC_NAME: &str = "my-topic";
const PARTITIONS: u32 = 1;
const REPLICAS: u32 = 1;

const PARTITION_NUM: u32 = 0;

const STREAM_ID: u32 = 1;
const TOPIC_ID: u32 = 1;

async fn create_producer(client: &IggyClient, stream: &str, topic: &str) -> Result<IggyProducer> {
    let mut producer = client
        .producer(stream, topic)
        .context("Failed to create producer")?
        .batch_size(1000)
        // .direct(
        //     // Use either direct (instant) or background message sending
        //     DirectConfig::builder()
        //         .batch_length(1000)
        //         .linger_time(IggyDuration::from_str("5ms")?)
        //         .build(),
        // )
        .send_interval(IggyDuration::from_str("1ms")?)
        .partitioning(Partitioning::balanced())
        .create_stream_if_not_exists()
        .create_topic_if_not_exists(
            PARTITIONS,
            None,
            IggyExpiry::ServerDefault,
            MaxTopicSize::ServerDefault,
        )
        .build();

    producer.init().await?;

    Ok(producer)
}

#[tokio::main]
async fn main() -> Result<()> {
    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("DEBUG")))
        .init();
    // Connect to Fluvio cluster
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

    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        if env::var("DOCKER_ENV").is_ok() {
            "postgres://postgres:password@timescaledb:5432/market_data".to_string()
        } else {
            "postgres://postgres:password@localhost:5432/market_data".to_string()
        }
    });
    info!("Connecting to TimescaleDB at: {database_url}");

    let pool = PgPool::connect(&database_url)
        .await
        .context("Failed to connect to TimescaleDB")?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS test_data (time TIMESTAMPTZ NOT NULL,
        value TEXT NOT NULL,
        symbol TEXT NOT NULL)
        WITH (tsdb.hypertable, tsdb.partition_column='time', tsdb.segmentby='symbol', tsdb.orderby='time DESC');",
    )
    .execute(&pool)
    .await
    .context("Failed to create table in TimescaleDB")?;

    println!("Connecting to Iggy message queue at: {iggy_connection}");
    let client = IggyClient::from_connection_string(&iggy_connection)?;

    client.connect().await?;

    client.ping().await.context("Failed to ping Iggy server")?;

    // if (client.create_stream("my-stream", Some(STREAM_ID)).await).is_ok() {
    //     info!("Stream was created.");
    // } else {
    //     warn!("Stream already exists and will not be created again.");
    // }

    // if (client
    //     .create_topic(
    //         &STREAM_ID.try_into().unwrap(),
    //         "my-topic",
    //         1,
    //         CompressionAlgorithm::default(),
    //         None,
    //         Some(TOPIC_ID),
    //         IggyExpiry::NeverExpire,
    //         MaxTopicSize::ServerDefault,
    //     )
    //     .await)
    //     .is_ok()
    // {
    //     info!("Topic was created.");
    // } else {
    //     warn!("Topic already exists and will not be created again.");
    // }

    info!("Successfully connected to Iggy message queue");

    // let client = Fluvio::connect()
    //     .await
    //     .expect("Failed to connect to Fluvio");

    // Create a topic
    let producer = create_producer(&client, STREAM_NAME, TOPIC_NAME).await?;

    for i in 0..i64::MAX {
        let record = format!("Time is {}", Local::now().timestamp_micros());
        println!("{i} - Sending record: {record}");
        producer
            .send(vec![Message::from_str(&record)?])
            .await
            .context("Failed to send message")?;

        // Insert into TimescaleDB
        sqlx::query("INSERT INTO test_data (time, value, symbol) VALUES ($1, $2, $3)")
            .bind(Local::now())
            .bind(&record)
            .bind("TEST")
            .execute(&pool)
            .await
            .context("Failed to insert data into TimescaleDB")?;

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    Ok(())
}

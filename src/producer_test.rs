use anyhow::Result;
use chrono::Local;
use fluvio::{
    Fluvio, FluvioClusterConfig, Offset, RecordKey,
    consumer::{ConsumerConfigExtBuilder, ConsumerStream, OffsetManagementStrategy},
    metadata::topic::TopicSpec,
};
use futures_util::StreamExt;

const TOPIC_NAME: &str = "hello-rust-1";
const PARTITIONS: u32 = 1;
const REPLICAS: u32 = 1;

const PARTITION_NUM: u32 = 0;

#[tokio::main]
async fn main() -> Result<()> {
    // Connect to Fluvio cluster
    let port = if is_docker::is_docker() {
        9003 // Docker environment
    } else {
        9103 // Local environment
    };
    let mut config = FluvioClusterConfig::new(format!("127.0.0.1:{port}"));
    config.update_metadata_by_name("installation", "docker")?;
    let fluvio = Fluvio::connect_with_config(&config)
        .await
        .expect("Failed to connect to Fluvio with config");

    // Create a topic
    let admin = fluvio.admin().await;
    let topic_spec = TopicSpec::new_computed(PARTITIONS, REPLICAS, None);
    let _topic_create = admin
        .create(TOPIC_NAME.to_string(), false, topic_spec)
        .await;

    // List topics
    let topics = admin
        .all::<TopicSpec>()
        .await
        .expect("Failed to list topics");
    let topic_names = topics
        .iter()
        .map(|topic| topic.name.clone())
        .collect::<Vec<String>>();

    println!("Topics:\n  - {}", topic_names.join("\n  - "));
    let fluvio = Fluvio::connect()
        .await
        .expect("Failed to connect to Fluvio");
    println!("Connected to Fluvio");
    // Create a record
    let record = format!("Hello World! - Time is {}", Local::now().to_rfc2822());

    // Produce to a topic
    let producer = fluvio::producer(TOPIC_NAME)
        .await
        .expect("Failed to create producer");
    producer
        .send(RecordKey::NULL, record.clone())
        .await
        .expect("Failed to send record");

    // Fluvio batches outgoing records by default,
    // call flush to ensure the record is sent
    producer.flush().await.expect("Failed to flush");

    println!("Sent record: {record}");

    // Create key and value
    let key = "Hello";
    let value = "Fluvio";

    // create producer & send key/value
    let producer = fluvio::producer(TOPIC_NAME)
        .await
        .expect("Failed to create producer");
    producer
        .send(key, value)
        .await
        .expect("Failed to send record");
    producer.flush().await.expect("Failed to flush");

    println!("Sent [{key}] {value}");

    println!("Consuming records from topic: {TOPIC_NAME}");

    // Consume last record from topic
    let config = ConsumerConfigExtBuilder::default()
        .topic(TOPIC_NAME)
        .offset_start(Offset::beginning())
        .offset_strategy(OffsetManagementStrategy::Auto)
        .offset_consumer("my-consumer".to_string())
        .disable_continuous(true)
        .build()
        .expect("Failed to build consumer config");

    // Create consumer & stream one record
    let mut stream = fluvio
        .consumer_with_config(config)
        .await
        .expect("Failed to create consumer");

    while let Some(Ok(record)) = stream.next().await {
        let key = record.key().map_or("NULL".to_string(), |k| {
            String::from_utf8_lossy(k).to_string()
        });
        let value = String::from_utf8_lossy(record.value());
        println!("Consumed record: [{key}] {value}");
    }
    stream
        .offset_commit()
        .await
        .expect("Failed to commit offset");
    stream
        .offset_flush()
        .await
        .expect("Failed to flush offsets");

    Ok(())
}

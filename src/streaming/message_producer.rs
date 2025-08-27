use std::str::FromStr;

use anyhow::{Context, Result};
use bincode::Encode;
use iggy::{
    bytes_serializable::BytesSerializable,
    clients::{client::IggyClient, producer::IggyProducer},
    messages::send_messages::{Message, Partitioning},
    utils::{duration::IggyDuration, expiry::IggyExpiry, topic_size::MaxTopicSize},
};
use serde::Serialize;
use tracing::{error, info};

/// Reusable message producer for any serializable data
pub struct MessageProducer {
    producer: IggyProducer,
    stream_name: String,
    topic_name: String,
}

impl MessageProducer {
    pub async fn new(
        client: &IggyClient,
        stream_name: &str,
        topic_name: &str,
        partitions: u32,
    ) -> Result<Self> {
        let mut producer = client
            .producer(stream_name, topic_name)
            .context("Failed to create producer")?
            .batch_size(1000)
            .send_interval(IggyDuration::from_str("1ms")?)
            .partitioning(Partitioning::balanced())
            .create_stream_if_not_exists()
            .create_topic_if_not_exists(
                partitions,
                None,
                IggyExpiry::ExpireDuration(IggyDuration::from_str("4h")?),
                MaxTopicSize::ServerDefault,
            )
            .build();

        producer
            .init()
            .await
            .context("Failed to initialize producer")?;

        info!(
            "Created message producer for stream: {}, topic: {}",
            stream_name, topic_name
        );

        Ok(Self {
            producer,
            stream_name: stream_name.to_string(),
            topic_name: topic_name.to_string(),
        })
    }

    async fn send(&self, message: Message) -> Result<()> {
        if let Err(e) = self.producer.send_one(message).await {
            error!(
                "Failed to send message to stream: {}, topic: {}, error: {}",
                self.stream_name, self.topic_name, e
            );
            return Err(e.into());
        }
        Ok(())
    }

    fn message_json<T: Serialize>(data: &T) -> Result<Message> {
        Message::from_str(&serde_json::to_string(data)?).context("Failed to create JSON message")
    }

    fn message_bincode<T: Encode>(data: &T) -> Result<Message> {
        let bytes = bincode::encode_to_vec(data, bincode::config::standard())?;
        Message::from_bytes(bytes.into()).context("Failed to create bincode message")
    }

    pub async fn send_json<T: Serialize>(&self, data: &T) -> Result<()> {
        self.send(Self::message_json(data)?).await
    }

    pub async fn send_bincode<T: Encode>(&self, data: &T) -> Result<()> {
        self.send(Self::message_bincode(data)?).await
    }
}

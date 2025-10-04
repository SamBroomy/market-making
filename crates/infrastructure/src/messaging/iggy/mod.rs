pub mod config;

use std::{str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use bincode::Encode;
use bytes::Bytes;
use domain::services::messaging::{MessageConfig, MessagePublisher};
// use iggy::{
//     bytes_serializable::BytesSerializable, client::{Client, SystemClient}, clients::{client::IggyClient, producer::IggyProducer}, messages::send_messages::{Message, Partitioning}, prelude::SystemClient, utils::{duration::IggyDuration, expiry::IggyExpiry, topic_size::MaxTopicSize}
// };
use iggy::prelude::*;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tokio::sync::OnceCell;
use tracing::{error, info};

use crate::messaging::{PublisherFactory, StreamProducer, iggy::config::IggySettings};

#[derive(Debug, Clone)]
pub struct IggyClientFactory(Arc<IggyClient>);

#[async_trait]
impl PublisherFactory for IggyClientFactory {
    type Config = IggySettings;

    async fn from_config(config: Self::Config) -> Result<Self>
    where
        Self: Sized,
    {
        let client =
            IggyClient::builder_from_connection_string(config.connection_string().expose_secret())
                .context("Failed to create Iggy client")?
                .build()?;
        client
            .connect()
            .await
            .context("Failed to connect to Iggy")
            .unwrap();
        client
            .ping()
            .await
            .context("Failed to ping Iggy server")
            .unwrap();

        info!("Connected to Iggy at {:?}", config);
        Ok(Self(Arc::new(client)))
    }

    async fn create_producer(&self, stream_name: &str, topic_name: &str) -> Result<StreamProducer> {
        let producer = self
            .0
            .producer(stream_name, topic_name)
            .context("Failed to create producer")?
            // .batch_size(1000)
            // .send_interval(IggyDuration::from_str("1ms")?)
            // .partitioning(Partitioning::balanced())
            .direct(
                DirectConfig::builder()
                    .batch_length(1000)
                    .linger_time(IggyDuration::from_str("1ms")?)
                    .build(),
            )
            .partitioning(Partitioning::balanced())
            .create_stream_if_not_exists()
            .create_topic_if_not_exists(
                1,
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

        let iggy_producer = IggyMessageProducer::new(producer, stream_name, topic_name);
        Ok(StreamProducer::new(
            Arc::new(iggy_producer),
            format!("iggy-{stream_name}-{topic_name}"),
        ))
    }

    async fn disconnect(&self) -> Result<()> {
        self.0
            .disconnect()
            .await
            .context("Failed to disconnect Iggy client")?;

        info!("Disconnected from Iggy");
        Ok(())
    }
}
pub struct IggyMessageProducer {
    producer: IggyProducer,
    stream_name: String,
    topic_name: String,
}

impl IggyMessageProducer {
    #[must_use]
    pub fn new(producer: IggyProducer, stream_name: &str, topic_name: &str) -> Self {
        Self {
            producer,
            stream_name: stream_name.to_string(),
            topic_name: topic_name.to_string(),
        }
    }

    async fn send(&self, message: IggyMessage) -> Result<()> {
        if let Err(e) = self.producer.send_one(message).await {
            error!(
                "Failed to send message to stream: {}, topic: {}, error: {}",
                self.stream_name, self.topic_name, e
            );
            return Err(e.into());
        }
        Ok(())
    }

    async fn send_all(&self, messages: Vec<IggyMessage>) -> Result<()> {
        if let Err(e) = self.producer.send(messages).await {
            error!(
                "Failed to send messages to stream: {}, topic: {}, error: {}",
                self.stream_name, self.topic_name, e
            );
            return Err(e.into());
        }
        Ok(())
    }

    fn message_json<T: Serialize>(data: &T) -> Result<IggyMessage> {
        IggyMessage::from_str(&serde_json::to_string(data)?)
            .context("Failed to create JSON message")
    }

    fn message_bincode<T: Encode>(data: &T) -> Result<IggyMessage> {
        let bytes = bincode::encode_to_vec(data, bincode::config::standard())?;
        IggyMessage::from_bytes(bytes.into()).context("Failed to create bincode message")
    }

    pub async fn send_json<T: Serialize>(&self, data: &T) -> Result<()> {
        self.send(Self::message_json(data)?).await
    }

    pub async fn send_bincode<T: Encode>(&self, data: &T) -> Result<()> {
        self.send(Self::message_bincode(data)?).await
    }
}

#[async_trait]
impl MessagePublisher for IggyMessageProducer {
    async fn send_bytes(&self, data: &[u8]) -> Result<()> {
        self.send(IggyMessage::from_bytes(Bytes::copy_from_slice(data))?)
            .await
    }

    async fn send_str(&self, data: &str) -> Result<()> {
        self.send(IggyMessage::from_str(data)?).await
    }
}

static CACHED_RESULT: OnceCell<IggyClientFactory> = OnceCell::const_new();
/// Read Iggy settings from infrastructure environment variables
#[must_use]
pub async fn get_iggy() -> IggyClientFactory {
    CACHED_RESULT
        .get_or_init(|| async {
            let is_docker = is_docker::is_docker();

            let settings = IggySettings {
                username: std::env::var("IGGY_USERNAME").unwrap_or_else(|_| "iggy".to_string()),
                password: SecretString::new(
                    std::env::var("IGGY_PASSWORD")
                        .unwrap_or_else(|_| "Secret123!".to_string())
                        .into(),
                ),
                port: if is_docker {
                    // Inside Docker, Iggy runs on port 3000
                    3000
                } else {
                    // Outside Docker, mapped to port from env or default 5100
                    std::env::var("IGGY_PORT")
                        .unwrap_or_else(|_| "5100".to_string())
                        .parse()
                        .unwrap_or(5100)
                },
                host: if is_docker {
                    "iggy".to_string()
                } else {
                    "localhost".to_string()
                },
            };

            info!("Iggy settings: {:?}", settings);

            IggyClientFactory::from_config(settings)
                .await
                .expect("Failed to create Iggy client")
        })
        .await
        .clone()
}

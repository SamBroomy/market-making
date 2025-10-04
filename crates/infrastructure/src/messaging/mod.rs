use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bincode::Encode;
use domain::services::messaging::{MessageConfig, MessagePublisher};
use serde::Serialize;

pub mod iggy;

#[derive(Clone)]
pub struct StreamProducer {
    publisher: Arc<dyn MessagePublisher>,
    name: String,
}

impl StreamProducer {
    #[must_use]
    pub fn new(publisher: Arc<dyn MessagePublisher>, name: String) -> Self {
        Self { publisher, name }
    }

    pub async fn send<T: Serialize>(&self, data: &T) -> Result<()> {
        self.publisher.send_str(&serde_json::to_string(data)?).await
    }

    pub async fn send_bincode<T: Encode>(&self, data: &T) -> Result<()> {
        self.publisher
            .send_bytes(&bincode::encode_to_vec(data, bincode::config::standard())?)
            .await
    }
}

#[async_trait]
pub trait PublisherFactory: Clone {
    type Config: MessageConfig;
    async fn from_config(config: Self::Config) -> Result<Self>
    where
        Self: Sized;
    async fn create_producer(&self, stream_name: &str, topic_name: &str) -> Result<StreamProducer>;
    async fn disconnect(&self) -> Result<()>;
}

#[must_use]
pub async fn get_stream_producer(producer: &str) -> impl PublisherFactory {
    match producer.to_lowercase().as_str() {
        "iggy" => iggy::get_iggy().await,
        _ => panic!("Unsupported message producer: {producer}"),
    }
}

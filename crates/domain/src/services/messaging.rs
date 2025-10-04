use std::fmt::Debug;

use anyhow::Result;
use async_trait::async_trait;
use secrecy::SecretString;

pub trait MessageConfig: Send + Sync + Debug {
    #[must_use]
    fn connection_string(&self) -> SecretString;
}

#[async_trait]
pub trait MessagePublisher: Send + Sync {
    async fn send_bytes(&self, data: &[u8]) -> Result<()>;
    async fn send_str(&self, data: &str) -> Result<()>;
}

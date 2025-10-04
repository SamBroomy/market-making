use domain::services::messaging::MessageConfig;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_aux::field_attributes::deserialize_number_from_string;
/// Iggy message queue settings
#[derive(Debug, Clone, Deserialize)]
pub struct IggySettings {
    pub username: String,
    pub password: SecretString,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    pub host: String,
}

impl MessageConfig for IggySettings {
    fn connection_string(&self) -> SecretString {
        SecretString::new(
            format!(
                "iggy://{}:{}@{}:{}",
                self.username,
                self.password.expose_secret(),
                self.host,
                self.port
            )
            .into(),
        )
    }
}

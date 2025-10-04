use serde::Deserialize;

/// Logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingSettings {
    pub level: String,
}

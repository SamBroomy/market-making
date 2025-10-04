mod environment;
mod exchange;
mod logging;
pub mod trading;

use std::sync::OnceLock;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

pub use crate::settings::{
    environment::AppEnvironment, exchange::ExchangeSettings, logging::LoggingSettings,
    trading::TradingSettings,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub exchange: ExchangeSettings,
    pub trading: TradingSettings,
    pub logging: LoggingSettings,
}

impl Settings {
    fn validate(self) -> Result<Self, ConfigError> {
        // Validate that at least one market pair is configured
        if self.trading.is_empty() {
            return Err(ConfigError::Message(
                "At least one trading market must be configured".to_string(),
            ));
        }

        Ok(self)
    }

    pub fn app_environment() -> AppEnvironment {
        static CACHED_ENVIRONMENT: OnceLock<AppEnvironment> = OnceLock::new();
        *CACHED_ENVIRONMENT.get_or_init(|| {
            let env_var =
                std::env::var("MARKET_ENVIRONMENT").unwrap_or_else(|_| "development".into());
            env_var
                .try_into()
                .expect("Failed to parse MARKET_ENVIRONMENT")
        })
    }

    /// Load settings from configuration files and environment variables
    pub fn get_configuration() -> Result<Self, ConfigError> {
        let base_path = std::env::current_dir().expect("Failed to determine the current directory");
        let configuration_directory = base_path.join("configuration");

        // Detect the running environment
        let environment: AppEnvironment = Self::app_environment();

        let environment_filename = format!("{}.toml", environment.as_str());
        let settings = Config::builder()
            // Load base configuration
            .add_source(File::from(configuration_directory.join("base.toml")))
            // Layer on environment-specific configuration
            .add_source(
                File::from(configuration_directory.join(environment_filename)).required(false),
            )
            .add_source(
                Environment::with_prefix("MARKET")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?;

        settings.try_deserialize::<Self>().and_then(Self::validate)
    }

    /// Print configuration summary
    pub fn print_summary(&self) {
        println!("Market Making Producer Configuration Summary");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Environment info
        let environment = Self::app_environment();
        println!("Environment: {}", environment.as_str());
        println!("Log Level: {}", self.logging.level);

        // Trading configuration
        let resolved_pairs = self.trading.get_pair_configs();
        println!("Trading pairs: {} configured", resolved_pairs.len());

        for config in &resolved_pairs {
            println!("   • {} →", config.symbol);

            // Stream info
            let mut streams = Vec::new();
            if let Some(ob) = &config.streams.orderbook {
                streams.push(format!(
                    "orderbook({}@{:?})",
                    ob.snapshot_limit, ob.update_speed
                ));
            }
            if config.streams.ticker.is_some() {
                streams.push("ticker".to_string());
            }
            if let Some(w) = &config.streams.window {
                streams.push(format!("window({:?})", w.rolling_window_size));
            }

            if let Some(at) = &config.streams.agg_trade {
                streams.push(format!(
                    "agg_trade(window={:?}s, publish={:?}s)",
                    at.window_duration, at.publish_interval
                ));
            }

            if streams.is_empty() {
                println!("     Streams: None enabled");
            } else {
                println!("     Streams: {}", streams.join(", "));
            }

            println!(
                "     Options: Persist: {} | Message Queue: {}",
                config.persist, config.message_queue
            );
        }
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }
}

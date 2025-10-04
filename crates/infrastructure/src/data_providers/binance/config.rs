use std::time::Duration;

use domain::{services::exchange::ExchangeConfig, settings::ExchangeSettings};
use serde::Deserialize;

fn startup_delay_default() -> Duration {
    Duration::from_secs(5)
}

/// Binance API settings
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BinanceSettings {
    pub use_testnet: bool,
    #[serde(with = "humantime_serde", default = "startup_delay_default")]
    pub startup_delay: Duration,
}

impl Default for BinanceSettings {
    fn default() -> Self {
        Self {
            use_testnet: false,
            startup_delay: startup_delay_default(),
        }
    }
}

impl ExchangeConfig for BinanceSettings {
    fn from_settings(settings: ExchangeSettings) -> Self {
        Self {
            use_testnet: settings.use_testnet,
            startup_delay: settings.startup_delay,
        }
    }

    /// Get Binance REST API URL
    fn rest_url(&self) -> &str {
        if self.use_testnet {
            binance_sdk::constants::SPOT_REST_API_TESTNET_URL
        } else {
            binance_sdk::constants::SPOT_REST_API_PROD_URL
        }
    }

    fn ws_url(&self) -> &str {
        if self.use_testnet {
            binance_sdk::constants::SPOT_WS_STREAMS_TESTNET_URL
        } else {
            binance_sdk::constants::SPOT_WS_STREAMS_PROD_URL
        }
    }
}

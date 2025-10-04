use std::time::Duration;

use serde::Deserialize;

fn startup_delay_default() -> Duration {
    Duration::from_secs(5)
}
#[derive(Debug, Clone, Deserialize)]
pub struct ExchangeSettings {
    pub name: String, // "binance", "coinbase", etc.
    pub use_testnet: bool,
    #[serde(with = "humantime_serde", default = "startup_delay_default")]
    pub startup_delay: Duration,
}

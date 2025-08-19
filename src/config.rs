use std::time::Duration;

use anyhow::Result;
use secrecy::ExposeSecret;

use crate::settings::Settings;

/// Market Making Producer Configuration (YAML Config + Environment Variables Only)
#[derive(Debug, Clone)]
pub struct Config {
    pub settings: Settings,
}

impl Config {
    /// Load configuration from YAML files and environment variables
    pub fn load() -> Result<Self> {
        let settings = Settings::get_configuration()
            .map_err(|e| anyhow::anyhow!("Failed to load configuration: {}", e))?;

        Ok(Self { settings })
    }

    /// Get symbols as a vector of uppercase strings
    #[must_use]
    pub fn get_symbols(&self) -> Vec<String> {
        self.settings.trading.get_symbols()
    }

    /// Get startup delay as Duration
    #[must_use]
    pub fn get_startup_delay(&self) -> Duration {
        self.settings.trading.get_startup_delay()
    }

    /// Get snapshot limit
    #[must_use]
    pub fn get_snapshot_limit(&self) -> i32 {
        self.settings.trading.snapshot_limit
    }

    /// Get state stream depth
    #[must_use]
    pub fn get_state_stream_depth(&self) -> i32 {
        self.settings.trading.state_stream_depth
    }

    /// Get update speed
    #[must_use]
    pub fn get_update_speed(&self) -> String {
        self.settings.binance.update_speed.clone()
    }

    /// Get feature flags
    #[must_use]
    pub fn get_enable_streaming(&self) -> bool {
        self.settings.features.enable_streaming
    }

    #[must_use]
    pub fn get_enable_database(&self) -> bool {
        self.settings.features.enable_database
    }

    #[must_use]
    pub fn get_enable_signals(&self) -> bool {
        self.settings.features.enable_signals
    }

    #[must_use]
    pub fn get_enable_state(&self) -> bool {
        self.settings.features.enable_state
    }

    /// Get database URL (reads from infrastructure environment variables)
    #[must_use]
    pub fn get_database_url(&self) -> String {
        let database = Settings::get_database_settings();
        database.connection_string().expose_secret().to_string()
    }

    /// Get Iggy connection string (reads from infrastructure environment variables)
    #[must_use]
    pub fn get_iggy_connection_string(&self) -> String {
        let iggy = Settings::get_iggy_settings();
        iggy.connection_string()
    }

    /// Get Binance REST API URL
    #[must_use]
    pub fn get_binance_rest_url(&self) -> String {
        self.settings.binance.get_rest_url()
    }

    /// Get Binance WebSocket URL
    #[must_use]
    pub fn get_binance_ws_url(&self) -> String {
        self.settings.binance.get_ws_url()
    }

    /// Validate all configuration parameters
    pub fn validate(&self) -> Result<(), String> {
        self.settings.validate()
    }

    /// Get request weight for the configured snapshot limit
    #[must_use]
    pub fn get_snapshot_request_weight(&self) -> i32 {
        self.settings.trading.get_snapshot_request_weight()
    }

    /// Print configuration summary
    pub fn print_summary(&self) {
        self.settings.print_summary();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{BinanceSettings, FeaturesSettings, LoggingSettings, TradingSettings};

    fn create_test_settings() -> Settings {
        Settings {
            binance: BinanceSettings {
                use_testnet: true,
                update_speed: "100ms".to_string(),
                rest_url: None,
                ws_url: None,
            },
            trading: TradingSettings {
                symbols: vec!["BTCUSDT".to_string()],
                snapshot_limit: 500,
                state_stream_depth: 50,
                startup_delay_seconds: 10,
            },
            features: FeaturesSettings {
                enable_streaming: true,
                enable_database: true,
                enable_signals: true,
                enable_state: true,
            },
            logging: LoggingSettings {
                level: "warn".to_string(),
            },
        }
    }

    #[test]
    fn test_config_getters() {
        let config = Config {
            settings: create_test_settings(),
        };

        assert_eq!(config.get_symbols(), vec!["BTCUSDT"]);
        assert_eq!(config.get_snapshot_limit(), 500);
        assert_eq!(config.get_state_stream_depth(), 50);
        assert_eq!(config.get_update_speed(), "100ms");
        assert!(config.get_enable_streaming());
        assert!(config.get_enable_database());
        assert!(config.get_enable_signals());
        assert!(config.get_enable_state());
    }

    #[test]
    fn test_config_validation() {
        let config = Config {
            settings: create_test_settings(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_request_weight_calculation() {
        let mut settings = create_test_settings();

        settings.trading.snapshot_limit = 50;
        let config = Config { settings };
        assert_eq!(config.get_snapshot_request_weight(), 5);

        let mut settings = create_test_settings();
        settings.trading.snapshot_limit = 300;
        let config = Config { settings };
        assert_eq!(config.get_snapshot_request_weight(), 25);

        let mut settings = create_test_settings();
        settings.trading.snapshot_limit = 750;
        let config = Config { settings };
        assert_eq!(config.get_snapshot_request_weight(), 50);

        let mut settings = create_test_settings();
        settings.trading.snapshot_limit = 2000;
        let config = Config { settings };
        assert_eq!(config.get_snapshot_request_weight(), 250);
    }
}

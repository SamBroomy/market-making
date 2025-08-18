use std::time::Duration;

use clap::{ArgAction, Parser};

/// Market Making Producer Configuration
#[derive(Parser, Debug, Clone)]
#[command(name = "producer")]
#[command(about = "Cryptocurrency market making data producer")]
#[command(version = "0.1.0")]
pub struct Config {
    /// Trading symbols to monitor (comma-separated)
    #[arg(short, long, env = "SYMBOLS", value_delimiter = ',', default_values_t = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string(), "ETHBTC".to_string()])]
    pub symbols: Vec<String>,

    /// Order book depth limit for initial snapshots (1-5000)
    /// Higher limits cost more request weight: 1-100=5, 101-500=25, 501-1000=50, 1001-5000=250
    #[arg(long, env = "SNAPSHOT_LIMIT", default_value = "999")]
    pub snapshot_limit: i32,

    /// Number of order book levels to publish to state stream
    #[arg(long, env = "STATE_STREAM_DEPTH", default_value = "50")]
    pub state_stream_depth: i32,

    /// Use Binance testnet instead of production
    #[arg(long, env = "USE_TESTNET", action = ArgAction::SetTrue)]
    pub use_testnet: bool,

    /// WebSocket depth update speed (100ms or 1000ms)
    #[arg(long, env = "UPDATE_SPEED", default_value = "100ms")]
    pub update_speed: String,

    /// Delay in seconds between starting producers for different symbols
    #[arg(long, env = "STARTUP_DELAY_SECONDS", default_value = "10")]
    pub startup_delay_seconds: u64,

    /// Minimum delay between snapshot requests (seconds) to avoid rate limiting
    #[arg(long, env = "SNAPSHOT_DELAY_SECONDS", default_value = "1")]
    pub snapshot_delay_seconds: u64,

    /// Database connection URL
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Iggy message queue connection string
    #[arg(long, env = "IGGY_CONNECTION_STRING")]
    pub iggy_connection_string: Option<String>,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, env = "RUST_LOG", default_value = "warn")]
    pub log_level: String,

    /// Enable publishing to message streams
    #[arg(long, env = "ENABLE_STREAMING", default_value = "true")]
    pub enable_streaming: bool,

    /// Enable database persistence
    #[arg(long, env = "ENABLE_DATABASE", default_value = "true")]
    pub enable_database: bool,

    /// Enable orderbook signals publishing
    #[arg(long, env = "ENABLE_SIGNALS", default_value = "true")]
    pub enable_signals: bool,

    /// Enable orderbook state publishing
    #[arg(long, env = "ENABLE_STATE", default_value = "true")]
    pub enable_state: bool,

    /// Custom Binance REST API URL (overrides testnet/production defaults)
    #[arg(long, env = "BINANCE_REST_URL")]
    pub binance_rest_url: Option<String>,

    /// Custom Binance WebSocket URL (overrides testnet/production defaults)
    #[arg(long, env = "BINANCE_WS_URL")]
    pub binance_ws_url: Option<String>,
}

impl Config {
    /// Parse configuration from command line arguments and environment variables
    #[must_use]
    pub fn parse() -> Self {
        Parser::parse()
    }

    /// Get symbols as a vector of uppercase strings
    #[must_use]
    pub fn get_symbols(&self) -> Vec<String> {
        self.symbols
            .iter()
            .map(|s| s.trim().to_uppercase())
            .collect()
    }

    /// Get startup delay as Duration
    #[must_use]
    pub fn get_startup_delay(&self) -> Duration {
        Duration::from_secs(self.startup_delay_seconds)
    }

    /// Get snapshot delay as Duration
    #[must_use]
    pub fn get_snapshot_delay(&self) -> Duration {
        Duration::from_secs(self.snapshot_delay_seconds)
    }

    /// Validate snapshot limit is within Binance API constraints
    pub fn validate_snapshot_limit(&self) -> Result<(), String> {
        if !(1..=5000).contains(&self.snapshot_limit) {
            return Err(format!(
                "Snapshot limit must be between 1 and 5000, got: {}",
                self.snapshot_limit
            ));
        }
        Ok(())
    }

    /// Validate update speed is supported by Binance
    pub fn validate_update_speed(&self) -> Result<(), String> {
        match self.update_speed.as_str() {
            "100ms" | "1000ms" => Ok(()),
            _ => Err(format!(
                "Update speed must be '100ms' or '1000ms', got: '{}'",
                self.update_speed
            )),
        }
    }

    /// Get database URL with intelligent defaults
    #[must_use]
    pub fn get_database_url(&self) -> String {
        self.database_url.clone().unwrap_or_else(|| {
            if is_docker::is_docker() {
                "postgres://postgres:password@timescaledb:5432/market_data".to_string()
            } else {
                "postgres://postgres:password@localhost:5432/market_data".to_string()
            }
        })
    }

    /// Get Iggy connection string with intelligent defaults
    #[must_use]
    pub fn get_iggy_connection_string(&self) -> String {
        self.iggy_connection_string.clone().unwrap_or_else(|| {
            if is_docker::is_docker() {
                "iggy://iggy:Secret123!@iggy:3000".to_string()
            } else {
                "iggy://iggy:Secret123!@localhost:5100".to_string()
            }
        })
    }

    /// Get Binance REST API URL based on configuration
    #[must_use]
    pub fn get_binance_rest_url(&self) -> String {
        self.binance_rest_url.as_ref().map_or_else(
            || {
                if self.use_testnet {
                    "https://testnet.binance.vision".to_string()
                } else {
                    "https://api.binance.com".to_string()
                }
            },
            Clone::clone,
        )
    }

    /// Get Binance WebSocket URL based on configuration
    #[must_use]
    pub fn get_binance_ws_url(&self) -> String {
        self.binance_ws_url.as_ref().map_or_else(
            || {
                if self.use_testnet {
                    "wss://testnet.binance.vision/ws".to_string()
                } else {
                    "wss://stream.binance.com:9443/ws".to_string()
                }
            },
            Clone::clone,
        )
    }

    /// Validate all configuration parameters
    pub fn validate(&self) -> Result<(), String> {
        self.validate_snapshot_limit()?;
        self.validate_update_speed()?;

        if self.state_stream_depth < 1 {
            return Err("State stream depth must be at least 1".to_string());
        }

        if self.get_symbols().is_empty() {
            return Err("At least one symbol must be specified".to_string());
        }

        Ok(())
    }

    /// Get request weight for the configured snapshot limit
    #[must_use]
    pub fn get_snapshot_request_weight(&self) -> i32 {
        match self.snapshot_limit {
            1..=100 => 5,
            101..=500 => 25,
            501..=1000 => 50,
            1001..=5000 => 250,
            _ => 250, // Default to highest weight for safety
        }
    }

    /// Print configuration summary
    pub fn print_summary(&self) {
        println!("Market Making Producer Configuration:");
        println!("  Symbols: {:?}", self.get_symbols());
        println!(
            "  Snapshot limit: {} (weight: {})",
            self.snapshot_limit,
            self.get_snapshot_request_weight()
        );
        println!("  State stream depth: {}", self.state_stream_depth);
        println!(
            "  Environment: {}",
            if self.use_testnet {
                "testnet"
            } else {
                "production"
            }
        );
        println!("  Update speed: {}", self.update_speed);
        println!("  Startup delay: {}s", self.startup_delay_seconds);
        println!("  Snapshot delay: {}s", self.snapshot_delay_seconds);
        println!(
            "  Features: streaming={}, database={}, signals={}, state={}",
            self.enable_streaming, self.enable_database, self.enable_signals, self.enable_state
        );
        println!("  Binance REST: {}", self.get_binance_rest_url());
        println!("  Binance WS: {}", self.get_binance_ws_url());
        println!("  Database: {}", self.get_database_url());
        println!("  Iggy: {}", self.get_iggy_connection_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_limit_validation() {
        let mut config = Config::parse();

        config.snapshot_limit = 500;
        assert!(config.validate_snapshot_limit().is_ok());

        config.snapshot_limit = 0;
        assert!(config.validate_snapshot_limit().is_err());

        config.snapshot_limit = 6000;
        assert!(config.validate_snapshot_limit().is_err());
    }

    #[test]
    fn test_update_speed_validation() {
        let mut config = Config::parse();

        config.update_speed = "100ms".to_string();
        assert!(config.validate_update_speed().is_ok());

        config.update_speed = "1000ms".to_string();
        assert!(config.validate_update_speed().is_ok());

        config.update_speed = "500ms".to_string();
        assert!(config.validate_update_speed().is_err());
    }

    #[test]
    fn test_request_weight_calculation() {
        let mut config = Config::parse();

        config.snapshot_limit = 50;
        assert_eq!(config.get_snapshot_request_weight(), 5);

        config.snapshot_limit = 300;
        assert_eq!(config.get_snapshot_request_weight(), 25);

        config.snapshot_limit = 750;
        assert_eq!(config.get_snapshot_request_weight(), 50);

        config.snapshot_limit = 2000;
        assert_eq!(config.get_snapshot_request_weight(), 250);
    }
}

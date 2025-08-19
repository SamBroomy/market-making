use std::time::Duration;

use config::{Config, ConfigError, Environment, File};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_aux::field_attributes::deserialize_number_from_string;

/// Environment for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnvironment {
    Development,
    Production,
}

impl AppEnvironment {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

impl TryFrom<String> for AppEnvironment {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "production" | "prod" => Ok(Self::Production),
            other => Err(format!(
                "{other} is not a supported environment. Use either `local`, `development`, or `production`."
            )),
        }
    }
}

/// Database connection settings
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: SecretString,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    pub host: String,
    pub database_name: String,
    pub require_ssl: bool,
}

impl DatabaseSettings {
    /// Get database connection string
    #[must_use]
    pub fn connection_string(&self) -> SecretString {
        SecretString::new(
            format!(
                "postgres://{}:{}@{}:{}/{}",
                self.username,
                self.password.expose_secret(),
                self.host,
                self.port,
                self.database_name,
            )
            .into(),
        )
    }

    /// Get database connection string without credentials for logging
    #[must_use]
    pub fn connection_string_without_db(&self) -> String {
        format!(
            "postgres://{}:***@{}:{}/{}",
            self.username, self.host, self.port, self.database_name
        )
    }
}

/// Iggy message queue settings
#[derive(Debug, Clone, Deserialize)]
pub struct IggySettings {
    pub username: String,
    pub password: SecretString,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub port: u16,
    pub host: String,
}

impl IggySettings {
    /// Get Iggy connection string
    #[must_use]
    pub fn connection_string(&self) -> String {
        format!(
            "iggy://{}:{}@{}:{}",
            self.username,
            self.password.expose_secret(),
            self.host,
            self.port
        )
    }

    /// Get Iggy connection string without credentials for logging
    #[must_use]
    pub fn connection_string_without_credentials(&self) -> String {
        format!("iggy://{}:***@{}:{}", self.username, self.host, self.port)
    }
}

/// Binance API settings
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceSettings {
    pub use_testnet: bool,
    pub update_speed: String,
}

impl BinanceSettings {
    /// Get Binance REST API URL
    pub fn get_rest_url(&self) -> String {
        if self.use_testnet {
            binance_sdk::constants::SPOT_REST_API_TESTNET_URL.to_string()
        } else {
            binance_sdk::constants::SPOT_REST_API_PROD_URL.to_string()
        }
    }

    /// Get Binance WebSocket URL
    pub fn get_ws_url(&self) -> String {
        if self.use_testnet {
            binance_sdk::constants::SPOT_WS_STREAMS_TESTNET_URL.to_string()
        } else {
            binance_sdk::constants::SPOT_WS_STREAMS_PROD_URL.to_string()
        }
    }

    /// Validate update speed
    pub fn validate_update_speed(&self) -> Result<(), String> {
        match self.update_speed.as_str() {
            "100ms" | "1000ms" => Ok(()),
            _ => Err(format!(
                "Update speed must be '100ms' or '1000ms', got: '{}'",
                self.update_speed
            )),
        }
    }
}

/// Trading configuration
#[derive(Debug, Clone, Deserialize)]
pub struct TradingSettings {
    pub symbols: Vec<String>,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub snapshot_limit: i32,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub state_stream_depth: i32,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub startup_delay_seconds: u64,
}

impl TradingSettings {
    /// Get symbols as uppercase strings
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

    /// Validate snapshot limit
    pub fn validate_snapshot_limit(&self) -> Result<(), String> {
        if !(1..=5000).contains(&self.snapshot_limit) {
            return Err(format!(
                "Snapshot limit must be between 1 and 5000, got: {}",
                self.snapshot_limit
            ));
        }
        Ok(())
    }

    /// Get request weight for the snapshot limit
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
}

/// Feature toggles
#[derive(Debug, Clone, Deserialize)]
pub struct FeaturesSettings {
    pub enable_streaming: bool,
    pub enable_database: bool,
    pub enable_signals: bool,
    pub enable_state: bool,
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingSettings {
    pub level: String,
}

/// Application settings (business logic only)
/// Infrastructure settings (database, iggy) are read separately from environment variables
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub binance: BinanceSettings,
    pub trading: TradingSettings,
    pub features: FeaturesSettings,
    pub logging: LoggingSettings,
}

impl Settings {
    /// Load settings from configuration files and environment variables
    pub fn get_configuration() -> Result<Self, ConfigError> {
        let base_path = std::env::current_dir().expect("Failed to determine the current directory");
        let configuration_directory = base_path.join("configuration");

        // Detect the running environment
        let environment: AppEnvironment = std::env::var("MARKET_ENVIRONMENT")
            .unwrap_or_else(|_| "development".into())
            .try_into()
            .expect("Failed to parse MARKET_ENVIRONMENT");

        let environment_filename = format!("{}.yaml", environment.as_str());
        let settings = Config::builder()
            // Load base configuration
            .add_source(File::from(configuration_directory.join("base.yaml")))
            // Layer on environment-specific configuration
            .add_source(
                File::from(configuration_directory.join(environment_filename)).required(false),
            )
            // Add in settings from environment variables (with a prefix of MARKET and '__' as separator)
            // E.g. `MARKET_TRADING__SYMBOLS=BTCUSDT,ETHUSDT` would set `Settings.trading.symbols`
            .add_source(
                Environment::with_prefix("MARKET")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()?;

        settings.try_deserialize::<Self>()
    }

    /// Read database settings from infrastructure environment variables
    #[must_use]
    pub fn get_database_settings() -> DatabaseSettings {
        let is_docker = is_docker::is_docker();

        DatabaseSettings {
            username: std::env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string()),
            password: SecretString::new(
                std::env::var("POSTGRES_PASSWORD")
                    .unwrap_or_else(|_| "password".to_string())
                    .into(),
            ),
            port: std::env::var("POSTGRES_PORT")
                .unwrap_or_else(|_| "5432".to_string())
                .parse()
                .unwrap_or(5432),
            host: if is_docker {
                "timescaledb".to_string()
            } else {
                "localhost".to_string()
            },
            database_name: std::env::var("POSTGRES_DB")
                .unwrap_or_else(|_| "market_data".to_string()),
            require_ssl: false, // Always false for internal connections
        }
    }

    /// Read Iggy settings from infrastructure environment variables
    #[must_use]
    pub fn get_iggy_settings() -> IggySettings {
        let is_docker = is_docker::is_docker();

        IggySettings {
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
        }
    }

    /// Validate all settings
    pub fn validate(&self) -> Result<(), String> {
        self.trading.validate_snapshot_limit()?;
        self.binance.validate_update_speed()?;

        if self.trading.state_stream_depth < 1 {
            return Err("State stream depth must be at least 1".to_string());
        }

        if self.trading.get_symbols().is_empty() {
            return Err("At least one symbol must be specified".to_string());
        }

        Ok(())
    }

    /// Print configuration summary (without secrets)
    pub fn print_summary(&self) {
        println!("Market Making Producer Configuration:");
        println!(
            "  Environment: {}",
            std::env::var("MARKET_ENVIRONMENT").unwrap_or_else(|_| "local".to_string())
        );
        println!("  Symbols: {:?}", self.trading.get_symbols());
        println!(
            "  Snapshot limit: {} (weight: {})",
            self.trading.snapshot_limit,
            self.trading.get_snapshot_request_weight()
        );
        println!("  State stream depth: {}", self.trading.state_stream_depth);
        println!(
            "  Environment: {}",
            if self.binance.use_testnet {
                "testnet"
            } else {
                "production"
            }
        );
        println!("  Update speed: {}", self.binance.update_speed);
        println!("  Startup delay: {}s", self.trading.startup_delay_seconds);
        println!(
            "  Features: streaming={}, database={}, signals={}, state={}",
            self.features.enable_streaming,
            self.features.enable_database,
            self.features.enable_signals,
            self.features.enable_state
        );
        println!("  Binance REST: {}", self.binance.get_rest_url());
        println!("  Binance WS: {}", self.binance.get_ws_url());

        // Read infrastructure settings from environment variables
        let database = Self::get_database_settings();
        let iggy = Self::get_iggy_settings();

        println!("  Database: {}", database.connection_string_without_db());
        println!("  Iggy: {}", iggy.connection_string_without_credentials());
        println!("  Config source: YAML + Infrastructure env vars");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_environment_parsing() {
        assert_eq!(
            AppEnvironment::try_from("development".to_string()).unwrap(),
            AppEnvironment::Development
        );
        assert_eq!(
            AppEnvironment::try_from("production".to_string()).unwrap(),
            AppEnvironment::Production
        );
        assert!(AppEnvironment::try_from("invalid".to_string()).is_err());
    }

    #[test]
    fn test_database_connection_string() {
        let db_settings = DatabaseSettings {
            username: "test_user".to_string(),
            password: SecretString::new("test_pass".to_string().into()),
            port: 5432,
            host: "localhost".to_string(),
            database_name: "test_db".to_string(),
            require_ssl: false,
        };

        let conn_str = db_settings.connection_string();
        assert_eq!(
            conn_str.expose_secret(),
            "postgres://test_user:test_pass@localhost:5432/test_db"
        );
    }

    #[test]
    fn test_binance_url_generation() {
        let binance_settings = BinanceSettings {
            use_testnet: true,
            update_speed: "100ms".to_string(),
        };

        let rest_url = binance_settings.get_rest_url();
        assert_eq!(rest_url, binance_sdk::constants::SPOT_REST_API_TESTNET_URL);

        let ws_url = binance_settings.get_ws_url();
        assert_eq!(ws_url, binance_sdk::constants::SPOT_WS_STREAMS_TESTNET_URL);
    }
}

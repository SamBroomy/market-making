use core::fmt;
use std::{collections::HashMap, sync::OnceLock};

use binance_sdk::spot::websocket_streams::RollingWindowTickerWindowSizeEnum;
use config::{Config, ConfigError, Environment, File};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, de::IntoDeserializer};
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
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BinanceSettings {
    pub use_testnet: bool,
    startup_delay_seconds: u64,
}
impl Default for BinanceSettings {
    fn default() -> Self {
        Self {
            use_testnet: false,
            startup_delay_seconds: 5,
        }
    }
}

impl BinanceSettings {
    /// Get Binance REST API URL
    #[must_use]
    pub fn get_rest_url(&self) -> String {
        if self.use_testnet {
            binance_sdk::constants::SPOT_REST_API_TESTNET_URL.to_string()
        } else {
            binance_sdk::constants::SPOT_REST_API_PROD_URL.to_string()
        }
    }

    /// Get Binance WebSocket URL
    #[must_use]
    pub fn get_ws_url(&self) -> String {
        if self.use_testnet {
            binance_sdk::constants::SPOT_WS_STREAMS_TESTNET_URL.to_string()
        } else {
            binance_sdk::constants::SPOT_WS_STREAMS_PROD_URL.to_string()
        }
    }

    #[must_use]
    pub fn get_startup_delay(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.startup_delay_seconds)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub enum OrderBookUpdateSpeed {
    #[default]
    #[serde(rename = "100ms")]
    HundredMs,
    #[serde(rename = "1000ms")]
    ThousandMs,
}

impl std::str::FromStr for OrderBookUpdateSpeed {
    type Err = String;

    fn from_str(speed: &str) -> Result<Self, Self::Err> {
        match speed {
            "100ms" => Ok(Self::HundredMs),
            "1000ms" => Ok(Self::ThousandMs),
            _ => Err(format!("Invalid update speed: {speed}")),
        }
    }
}
impl fmt::Display for OrderBookUpdateSpeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HundredMs => write!(f, "100ms"),
            Self::ThousandMs => write!(f, "1000ms"),
        }
    }
}

/// Stream configuration for orderbook/depth data
#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookStreamConfig {
    pub snapshot_limit: Option<i32>,
    pub state_stream_depth: Option<i32>,
    pub update_speed: Option<OrderBookUpdateSpeed>,
}

/// Stream configuration for ticker data
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TickerStreamConfig;

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub enum RollingWindowSize {
    #[default]
    #[serde(rename = "1d")]
    Size1d,
    #[serde(rename = "4h")]
    Size4h,
    #[serde(rename = "1h")]
    Size1h,
}

impl std::str::FromStr for RollingWindowSize {
    type Err = String;

    fn from_str(speed: &str) -> Result<Self, Self::Err> {
        match speed {
            "1d" => Ok(Self::Size1d),
            "4h" => Ok(Self::Size4h),
            "1h" => Ok(Self::Size1h),
            _ => Err(format!("Invalid rolling window size: {speed}")),
        }
    }
}
impl fmt::Display for RollingWindowSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size1d => write!(f, "1d"),
            Self::Size4h => write!(f, "4h"),
            Self::Size1h => write!(f, "1h"),
        }
    }
}
impl From<RollingWindowSize> for RollingWindowTickerWindowSizeEnum {
    fn from(val: RollingWindowSize) -> Self {
        match val {
            RollingWindowSize::Size1d => Self::WindowSize1d,
            RollingWindowSize::Size4h => Self::WindowSize4h,
            RollingWindowSize::Size1h => Self::WindowSize1h,
        }
    }
}
/// Stream configuration for rolling window ticker data
#[derive(Debug, Clone, Deserialize)]
pub struct WindowStreamConfig {
    pub rolling_window_size: Option<RollingWindowSize>,
}

/// Stream configuration for aggregate trade data
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AggTradeStreamConfig;

fn deserialize_optional_unit<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    use std::fmt;

    use serde::de::{Error, Visitor};

    struct OptionalUnitVisitor<T>(std::marker::PhantomData<T>);

    impl<'de, T> Visitor<'de> for OptionalUnitVisitor<T>
    where
        T: Default + Deserialize<'de>,
    {
        type Value = Option<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("null, a valid value, or nothing")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            T::deserialize(deserializer).map(Some)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(Some(T::default()))
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            if v.is_empty() {
                Ok(Some(T::default()))
            } else {
                Err(E::custom(format!(
                    "Expected empty string or null, got: {}",
                    v
                )))
            }
        }
    }

    deserializer.deserialize_option(OptionalUnitVisitor(std::marker::PhantomData))
}

/// Container for all stream configurations
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct StreamConfigs {
    pub orderbook: Option<OrderbookStreamConfig>,
    pub ticker: TickerStreamConfig,
    pub window: Option<WindowStreamConfig>,
    pub agg_trade: AggTradeStreamConfig,
}

/// Helper function for default true values
fn default_true() -> bool {
    true
}

/// Configuration for a single trading symbol
#[derive(Debug, Clone, Deserialize)]
pub struct SymbolTradingConfig {
    pub streams: Option<StreamConfigs>,
    #[serde(default = "default_true")]
    pub persist: bool,
    #[serde(default = "default_true")]
    pub message_queue: bool,
}
impl Default for SymbolTradingConfig {
    fn default() -> Self {
        Self {
            streams: None,
            persist: true,
            message_queue: true,
        }
    }
}

fn default_snapshot_limit() -> i32 {
    100 // Default snapshot limit
}
fn default_state_stream_depth() -> i32 {
    25 // Default state stream depth
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamDefaults {
    #[serde(default = "default_snapshot_limit")]
    pub snapshot_limit: i32,
    #[serde(default = "default_state_stream_depth")]
    pub state_stream_depth: i32,
    pub update_speed: OrderBookUpdateSpeed,
    pub rolling_window_size: RollingWindowSize,
}

impl Default for StreamDefaults {
    fn default() -> Self {
        Self {
            snapshot_limit: default_snapshot_limit(),
            state_stream_depth: default_state_stream_depth(),
            update_speed: OrderBookUpdateSpeed::HundredMs,
            rolling_window_size: RollingWindowSize::Size1d,
        }
    }
}

/// Trading configuration
#[derive(Debug, Clone, Deserialize)]
pub struct TradingSettings {
    pub symbols: HashMap<String, Option<SymbolTradingConfig>>,
    // Global defaults for when symbol configs don't specify values
    #[serde(default)]
    pub defaults: StreamDefaults,
}
/// Resolved orderbook configuration with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedOrderbookConfig {
    pub snapshot_limit: i32,
    pub state_stream_depth: i32,
    pub update_speed: OrderBookUpdateSpeed,
}
impl ResolvedOrderbookConfig {
    /// Create a new resolved orderbook config with defaults applied
    #[must_use]
    pub fn new(
        snapshot_limit: i32,
        state_stream_depth: i32,
        update_speed: OrderBookUpdateSpeed,
    ) -> Self {
        let snapshot_limit = Self::validate_snapshot_limit(snapshot_limit);
        let state_stream_depth = Self::validate_state_stream_depth(state_stream_depth);
        Self {
            snapshot_limit,
            state_stream_depth,
            update_speed,
        }
    }

    /// Get request weight for a specific limit value
    fn validate_snapshot_limit(limit: i32) -> i32 {
        match limit {
            1..=100 => 5,
            101..=500 => 25,
            501..=1000 => 50,
            1001..=5000 => 250,
            _ => 250, // Default to highest weight for safety
        }
    }

    fn validate_state_stream_depth(depth: i32) -> i32 {
        match depth {
            1..=100 => depth,
            101..=500 => 100, // Cap at 100 for performance
            _ => 50,          // Default to 50 for safety
        }
    }
}
/// Resolved ticker configuration with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedTickerConfig;
/// Resolved window configuration with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedWindowConfig {
    pub rolling_window_size: RollingWindowSize,
}
impl ResolvedWindowConfig {
    /// Create a new resolved window config with defaults applied
    #[must_use]
    pub fn new(rolling_window_size: RollingWindowSize) -> Self {
        Self {
            rolling_window_size,
        }
    }
}
/// Resolved aggregate trade configuration with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedAggTradeConfig;
/// Resolved stream configurations with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub orderbook: Option<ResolvedOrderbookConfig>,
    pub ticker: Option<ResolvedTickerConfig>,
    pub window: Option<ResolvedWindowConfig>,
    pub agg_trade: Option<ResolvedAggTradeConfig>,
}

/// Resolved configuration for a symbol with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedSymbolConfig {
    pub symbol: String,
    pub streams: ResolvedStream,
    pub persist: bool,
    pub message_queue: bool,
}

impl TradingSettings {
    /// Validate all symbol configurations
    #[must_use]
    pub fn get_symbol_configs(&self) -> Vec<ResolvedSymbolConfig> {
        let Self { symbols, defaults } = self;
        let StreamDefaults {
            snapshot_limit,
            state_stream_depth,
            update_speed,
            rolling_window_size,
        } = *defaults;

        symbols
            .iter()
            .map(|(symbol_name, config)| {
                let default_config = SymbolTradingConfig::default();
                let config = config.as_ref().unwrap_or(&default_config);
                let symbol_name = symbol_name.trim().to_uppercase();
                let persist = config.persist;
                let message_queue = config.message_queue;
                // is stream none and if not are all the stream configs also none cuz then we use defaults
                let streams = if config.streams.as_ref().is_none_or(|stream| {
                    stream.orderbook.is_none()
                        //&& stream.ticker.is_none()
                        && stream.window.is_none()
                    //&& stream.agg_trade.is_none()
                }) {
                    ResolvedStream {
                        orderbook: Some(ResolvedOrderbookConfig::new(
                            snapshot_limit,
                            state_stream_depth,
                            update_speed,
                        )),
                        ticker: Some(ResolvedTickerConfig {}),
                        window: Some(ResolvedWindowConfig::new(rolling_window_size)),
                        agg_trade: Some(ResolvedAggTradeConfig {}),
                    }
                } else {
                    // create a stream config, using values from the config falling back to defaults if not specified
                    ResolvedStream {
                        orderbook: config.streams.as_ref().and_then(|sc| {
                            sc.orderbook.as_ref().map(|ob| {
                                ResolvedOrderbookConfig::new(
                                    ob.snapshot_limit.unwrap_or(snapshot_limit),
                                    ob.state_stream_depth.unwrap_or(state_stream_depth),
                                    ob.update_speed.unwrap_or(update_speed),
                                )
                            })
                        }),
                        ticker: config
                            .streams
                            .as_ref()
                            .and_then(|sc| sc.ticker.as_ref().map(|_| ResolvedTickerConfig {})),
                        window: config.streams.as_ref().and_then(|sc| {
                            sc.window.as_ref().map(|w| {
                                ResolvedWindowConfig::new(
                                    w.rolling_window_size.unwrap_or(rolling_window_size),
                                )
                            })
                        }),
                        agg_trade: config.streams.as_ref().and_then(|sc| {
                            sc.agg_trade.as_ref().map(|_| ResolvedAggTradeConfig {})
                        }),
                    }
                };

                ResolvedSymbolConfig {
                    symbol: symbol_name,
                    streams,
                    persist,
                    message_queue,
                }
            })
            .collect::<Vec<_>>()
    }
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
    #[serde(default)]
    pub binance: BinanceSettings,
    pub trading: TradingSettings,
    pub logging: LoggingSettings,
}

impl Settings {
    fn validate(self) -> Result<Self, ConfigError> {
        // Validate that at least one symbol is configured
        if self.trading.symbols.is_empty() {
            return Err(ConfigError::Message(
                "At least one trading symbol must be configured".to_string(),
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

        let environment_filename = format!("{}.yaml", environment.as_str());
        let settings = Config::builder()
            // Load base configuration
            .add_source(File::from(configuration_directory.join("base.yaml")))
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

    /// Read database settings from infrastructure environment variables
    #[must_use]
    pub fn get_database_settings() -> DatabaseSettings {
        static CACHED_RESULT: OnceLock<DatabaseSettings> = OnceLock::new();

        CACHED_RESULT
            .get_or_init(|| {
                let is_docker = is_docker::is_docker();

                DatabaseSettings {
                    username: std::env::var("POSTGRES_USER")
                        .unwrap_or_else(|_| "postgres".to_string()),
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
            })
            .clone()
    }

    /// Read Iggy settings from infrastructure environment variables
    #[must_use]
    pub fn get_iggy_settings() -> IggySettings {
        static CACHED_RESULT: OnceLock<IggySettings> = OnceLock::new();

        CACHED_RESULT
            .get_or_init(|| {
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
            })
            .clone()
    }

    /// Print configuration summary (without secrets)
    pub fn print_summary(&self) {
        println!("Market Making Producer Configuration Summary");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Environment info
        let environment = Self::app_environment();
        println!("Environment: {}", environment.as_str());
        println!("Log Level: {}", self.logging.level);

        // Trading configuration
        let resolved_symbols = self.trading.get_symbol_configs();
        println!("Trading Symbols: {} configured", resolved_symbols.len());

        for config in &resolved_symbols {
            println!("   • {} →", config.symbol);

            // Stream info
            let mut streams = Vec::new();
            if config.streams.orderbook.is_some() {
                let ob = config.streams.orderbook.as_ref().unwrap();
                streams.push(format!(
                    "orderbook({}@{:?})",
                    ob.snapshot_limit, ob.update_speed
                ));
            }
            if config.streams.ticker.is_some() {
                streams.push("ticker".to_string());
            }
            if config.streams.window.is_some() {
                let w = config.streams.window.as_ref().unwrap();
                streams.push(format!("window({:?})", w.rolling_window_size));
            }
            if config.streams.agg_trade.is_some() {
                streams.push("agg_trade".to_string());
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

        // Infrastructure info
        println!("  Infrastructure:");
        println!(
            "   • Database: {}",
            Self::get_database_settings().connection_string_without_db()
        );
        println!(
            "   • Message Queue: {}",
            Self::get_iggy_settings().connection_string_without_credentials()
        );

        // Binance API info
        let api_env = if self.binance.use_testnet {
            " testnet"
        } else {
            " production"
        };
        println!("  Binance API: {api_env}");
        println!("   • REST: {}", self.binance.get_rest_url());
        println!("   • WebSocket: {}", self.binance.get_ws_url());
        println!(
            "   • Startup delay: {}s",
            self.binance.startup_delay_seconds
        );

        // Defaults info
        println!("  Stream Defaults:");
        println!(
            "   • Snapshot limit: {} (weight: {})",
            self.trading.defaults.snapshot_limit,
            ResolvedOrderbookConfig::validate_snapshot_limit(self.trading.defaults.snapshot_limit)
        );
        println!(
            "   • State stream depth: {}",
            self.trading.defaults.state_stream_depth
        );
        println!(
            "   • Update speed: {:?}",
            self.trading.defaults.update_speed
        );
        println!(
            "   • Rolling window: {:?}",
            self.trading.defaults.rolling_window_size
        );

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }
}

use core::fmt;
use std::{collections::HashMap, sync::OnceLock};

use binance_sdk::spot::websocket_streams::RollingWindowTickerWindowSizeEnum;
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

macro_rules! impl_stream_config {
    ($config_enum:ident, $settings_struct:ident) => {
        #[derive(Debug, Clone, Deserialize)]
        #[serde(untagged)]
        enum $config_enum {
            Config($settings_struct),
            Enabled(bool),
        }
    };
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
struct OrderbookStreamSettingsRaw {
    pub snapshot_limit: i32,
    pub state_stream_depth: i32,
    pub update_speed: OrderBookUpdateSpeed,
}

impl Default for OrderbookStreamSettingsRaw {
    fn default() -> Self {
        Self {
            snapshot_limit: 999,
            state_stream_depth: 100,
            update_speed: OrderBookUpdateSpeed::default(),
        }
    }
}

/// Stream configuration for ticker data
#[derive(Debug, Clone, Deserialize, Default)]
struct TickerStreamSettingsRaw;

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
#[derive(Debug, Clone, Deserialize, Default)]
struct WindowStreamSettingsRaw {
    pub rolling_window_size: RollingWindowSize,
}

/// Stream configuration for aggregate trade data
#[derive(Debug, Clone, Deserialize)]
struct AggTradeStreamSettingsRaw {
    pub window_duration_seconds: u64,
    pub publish_interval_seconds: u64,
}

impl Default for AggTradeStreamSettingsRaw {
    fn default() -> Self {
        Self {
            window_duration_seconds: 60,
            publish_interval_seconds: 10,
        }
    }
}

// Now use the macro to generate the enums and impls
impl_stream_config!(OrderbookStreamConfig, OrderbookStreamSettingsRaw);
impl_stream_config!(TickerStreamConfig, TickerStreamSettingsRaw);
impl_stream_config!(WindowStreamConfig, WindowStreamSettingsRaw);
impl_stream_config!(AggTradeStreamConfig, AggTradeStreamSettingsRaw);

/// Helper function for default true values
fn default_true() -> bool {
    true
}

/// Configuration for a single trading pair
#[derive(Debug, Clone, Deserialize)]
struct TradingPairConfig {
    #[serde(default = "default_true")]
    pub persist: bool,
    #[serde(default = "default_true")]
    pub message_queue: bool,

    pub orderbook: Option<OrderbookStreamConfig>,
    pub ticker: Option<TickerStreamConfig>,
    pub window: Option<WindowStreamConfig>,
    pub agg_trade: Option<AggTradeStreamConfig>,
}

/// Trading configuration
#[derive(Debug, Clone, Deserialize)]
pub struct TradingSettings {
    market: HashMap<String, TradingPairConfig>,
}

#[derive(Debug, Clone)]
pub struct OrderbookStreamSettings {
    pub snapshot_limit: i32,
    pub state_stream_depth: i32,
    pub update_speed: OrderBookUpdateSpeed,
}

impl OrderbookStreamSettings {
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
            1..=5000 => limit,
            _ => 999, // Default to 999 for safety
        }
    }

    fn validate_state_stream_depth(depth: i32) -> i32 {
        match depth {
            1..=200 => depth,
            _ => 50, // Default to 50 for safety
        }
    }
}
impl Default for OrderbookStreamSettings {
    fn default() -> Self {
        Self::new(999, 100, OrderBookUpdateSpeed::default())
    }
}
impl From<OrderbookStreamSettingsRaw> for OrderbookStreamSettings {
    fn from(
        OrderbookStreamSettingsRaw {
            snapshot_limit,
            state_stream_depth,
            update_speed,
        }: OrderbookStreamSettingsRaw,
    ) -> Self {
        Self::new(snapshot_limit, state_stream_depth, update_speed)
    }
}

impl From<OrderbookStreamConfig> for Option<OrderbookStreamSettings> {
    fn from(val: OrderbookStreamConfig) -> Self {
        match val {
            OrderbookStreamConfig::Config(settings) => Some(settings.into()),
            OrderbookStreamConfig::Enabled(true) => Some(OrderbookStreamSettings::default()),
            OrderbookStreamConfig::Enabled(false) => None,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct TickerStreamSettings;
impl From<TickerStreamSettingsRaw> for TickerStreamSettings {
    fn from(_: TickerStreamSettingsRaw) -> Self {
        Self {}
    }
}
impl From<TickerStreamConfig> for Option<TickerStreamSettings> {
    fn from(val: TickerStreamConfig) -> Self {
        match val {
            TickerStreamConfig::Config(settings) => Some(settings.into()),
            TickerStreamConfig::Enabled(true) => Some(TickerStreamSettings),
            TickerStreamConfig::Enabled(false) => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WindowStreamSettings {
    pub rolling_window_size: RollingWindowSize,
}
impl WindowStreamSettings {
    #[must_use]
    pub fn new(rolling_window_size: RollingWindowSize) -> Self {
        Self {
            rolling_window_size,
        }
    }
}

impl From<WindowStreamSettingsRaw> for WindowStreamSettings {
    fn from(
        WindowStreamSettingsRaw {
            rolling_window_size,
        }: WindowStreamSettingsRaw,
    ) -> Self {
        Self::new(rolling_window_size)
    }
}

impl From<WindowStreamConfig> for Option<WindowStreamSettings> {
    fn from(val: WindowStreamConfig) -> Self {
        match val {
            WindowStreamConfig::Config(settings) => Some(settings.into()),
            WindowStreamConfig::Enabled(true) => Some(WindowStreamSettings::default()),
            WindowStreamConfig::Enabled(false) => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AggTradeStreamSettings {
    pub window_duration_seconds: u64,
    pub publish_interval_seconds: u64,
}

impl From<AggTradeStreamSettingsRaw> for AggTradeStreamSettings {
    fn from(
        AggTradeStreamSettingsRaw {
            window_duration_seconds,
            publish_interval_seconds,
        }: AggTradeStreamSettingsRaw,
    ) -> Self {
        Self {
            window_duration_seconds,
            publish_interval_seconds,
        }
    }
}

impl From<AggTradeStreamConfig> for Option<AggTradeStreamSettings> {
    fn from(value: AggTradeStreamConfig) -> Self {
        match value {
            AggTradeStreamConfig::Config(settings) => Some(settings.into()),
            AggTradeStreamConfig::Enabled(true) => Some(AggTradeStreamSettings::default()),
            AggTradeStreamConfig::Enabled(false) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Streams {
    pub orderbook: Option<OrderbookStreamSettings>,
    pub ticker: Option<TickerStreamSettings>,
    pub window: Option<WindowStreamSettings>,
    pub agg_trade: Option<AggTradeStreamSettings>,
}

impl Default for Streams {
    fn default() -> Self {
        Self {
            orderbook: Some(OrderbookStreamSettings::default()),
            ticker: Some(TickerStreamSettings),
            window: Some(WindowStreamSettings::default()),
            agg_trade: Some(AggTradeStreamSettings::default()),
        }
    }
}

/// Resolved configuration for a pair with all defaults applied
#[derive(Debug, Clone)]
pub struct ResolvedPairConfig {
    pub symbol: String,
    pub streams: Streams,
    pub persist: bool,
    pub message_queue: bool,
}

impl TradingSettings {
    /// Validate all pairs configurations
    #[must_use]
    pub fn get_pair_configs(&self) -> Vec<ResolvedPairConfig> {
        let Self { market } = self;

        market
            .iter()
            .map(|(pair, config)| {
                let symbol = pair.trim().to_uppercase();
                let persist = config.persist;
                let message_queue = config.message_queue;
                let streams = if config.orderbook.is_none()
                    && config.ticker.is_none()
                    && config.window.is_none()
                    && config.agg_trade.is_none()
                {
                    Streams::default()
                } else {
                    Streams {
                        orderbook: config.orderbook.as_ref().and_then(|ob| ob.clone().into()),
                        ticker: config.ticker.as_ref().and_then(|t| t.clone().into()),
                        window: config.window.as_ref().and_then(|w| w.clone().into()),
                        agg_trade: config.agg_trade.as_ref().and_then(|at| at.clone().into()),
                    }
                };
                ResolvedPairConfig {
                    symbol,
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
        // Validate that at least one market pair is configured
        if self.trading.market.is_empty() {
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
                    at.window_duration_seconds, at.publish_interval_seconds
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
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }
}

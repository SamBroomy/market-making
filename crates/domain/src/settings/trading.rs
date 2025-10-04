use core::fmt;
use std::{collections::HashMap, time::Duration};

use serde::Deserialize;

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
    /// Interval at which to publish orderbook snapshots (e.g., every 100ms) if none then every update received is published
    #[serde(with = "humantime_serde", default)]
    pub publish_interval: Option<Duration>,
}

impl Default for OrderbookStreamSettingsRaw {
    fn default() -> Self {
        Self {
            snapshot_limit: 999,
            state_stream_depth: 100,
            update_speed: OrderBookUpdateSpeed::default(),
            publish_interval: None,
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

/// Stream configuration for rolling window ticker data
#[derive(Debug, Clone, Deserialize, Default)]
struct WindowStreamSettingsRaw {
    pub rolling_window_size: RollingWindowSize,
}

/// Stream configuration for aggregate trade data
#[derive(Debug, Clone, Deserialize)]
struct AggTradeStreamSettingsRaw {
    /// Duration of the rolling window for aggregating trades (e.g., 1 minute)
    #[serde(with = "humantime_serde")]
    pub window_duration: Duration,
    /// Interval at which to publish aggregated trade data (e.g., every 10 seconds) if none then every aggregated trade received a summary is published
    #[serde(with = "humantime_serde", default)]
    pub publish_interval: Option<Duration>,
}

impl Default for AggTradeStreamSettingsRaw {
    fn default() -> Self {
        Self {
            window_duration: Duration::from_secs(60),
            publish_interval: None,
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
impl TradingSettings {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.market.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct OrderbookStreamSettings {
    pub snapshot_limit: i32,
    pub state_stream_depth: i32,
    pub update_speed: OrderBookUpdateSpeed,
    pub publish_interval: Option<Duration>,
}

impl OrderbookStreamSettings {
    /// Create a new resolved orderbook config with defaults applied
    #[must_use]
    pub fn new(
        snapshot_limit: i32,
        state_stream_depth: i32,
        update_speed: OrderBookUpdateSpeed,
        publish_interval: Option<Duration>,
    ) -> Self {
        let snapshot_limit = Self::validate_snapshot_limit(snapshot_limit);
        let state_stream_depth = Self::validate_state_stream_depth(state_stream_depth);
        Self {
            snapshot_limit,
            state_stream_depth,
            update_speed,
            publish_interval,
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
        Self::new(999, 100, OrderBookUpdateSpeed::default(), None)
    }
}
impl From<OrderbookStreamSettingsRaw> for OrderbookStreamSettings {
    fn from(
        OrderbookStreamSettingsRaw {
            snapshot_limit,
            state_stream_depth,
            update_speed,
            publish_interval,
        }: OrderbookStreamSettingsRaw,
    ) -> Self {
        Self::new(
            snapshot_limit,
            state_stream_depth,
            update_speed,
            publish_interval,
        )
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

#[derive(Debug, Clone)]
pub struct AggTradeStreamSettings {
    pub window_duration: Duration,
    pub publish_interval: Option<Duration>,
}

impl Default for AggTradeStreamSettings {
    fn default() -> Self {
        Self {
            window_duration: Duration::from_secs(60),
            publish_interval: None,
        }
    }
}

impl From<AggTradeStreamSettingsRaw> for AggTradeStreamSettings {
    fn from(
        AggTradeStreamSettingsRaw {
            window_duration,
            publish_interval,
        }: AggTradeStreamSettingsRaw,
    ) -> Self {
        Self {
            window_duration,
            publish_interval,
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

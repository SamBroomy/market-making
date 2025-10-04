use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use binance_sdk::{
    common::{
        utils::{replace_websocket_streams_placeholders, send_request},
        websocket::{self, WebsocketBase, create_stream_handler},
    },
    config::{ConfigurationRestApi, ConfigurationWebsocketStreams},
};
use domain::{
    models::market_data::{
        AggregateTrade, DepthSnapshot, DepthUpdate, TickerData, WindowTickerData,
    },
    services::exchange::{ExchangeConfig, ExchangeDataProvider},
};
use reqwest;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tracing::{error, info, warn};

use crate::data_providers::binance::config::BinanceSettings;

/// The reason we do this in a bit of a custom way half using the SDK and half not is because if we do it this way we can deserialize
/// directly into our own types without needing to create a whole bunch of new types that just mirror the SDK types.
/// The SDK types allocate vecs for example which we would then need to convert to our own types with extra allocations.
/// This way we can deserialize directly into our own types and avoid the extra allocations and conversions.
#[derive(Clone)]
pub struct BinanceClient {
    configuration: ConfigurationRestApi,
    ws_streams: Arc<websocket::WebsocketStreams>,
}

impl BinanceClient {
    #[must_use]
    async fn new(settings: &BinanceSettings) -> Self {
        const HAS_TIME_UNIT: bool = true;

        let mut cfg = ConfigurationWebsocketStreams::builder()
            .build()
            .expect("Failed to build WebSocket configuration");
        cfg.ws_url = Some(settings.ws_url().to_string());
        if !HAS_TIME_UNIT {
            cfg.time_unit = None;
        }

        let ws_streams = websocket::WebsocketStreams::new(cfg, vec![]);

        match ws_streams.clone().connect(vec![]).await {
            Ok(()) => {
                info!("WebSocket connection established");
            }
            Err(e) => {
                error!(error = %e, "Failed to connect WebSocket streams");
                panic!("Failed to connect WebSocket streams: {e}");
            }
        }

        let configuration = ConfigurationRestApi::builder()
            //   .api_key("YOUR_API_KEY")
            //   .api_secret("YOUR_SECRET_KEY")
            .base_path(settings.rest_url().to_string())
            .build()
            .expect("Failed to build REST API configuration");

        Self {
            configuration,
            ws_streams,
        }
    }
}

#[async_trait]
impl ExchangeDataProvider for BinanceClient {
    type Config = BinanceSettings;

    async fn from_config(cfg: Self::Config) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(Self::new(&cfg).await)
    }

    async fn depth_snapshot(&self, symbol: String, limit: Option<i32>) -> Result<DepthSnapshot> {
        let mut query_params = BTreeMap::new();

        query_params.insert("symbol".to_string(), json!(symbol));

        if let Some(rw) = limit {
            query_params.insert("limit".to_string(), json!(rw));
        }

        Ok(send_request::<DepthSnapshot>(
            &self.configuration,
            "/api/v3/depth",
            reqwest::Method::GET,
            query_params,
            self.configuration.time_unit,
            false,
        )
        .await?
        .data()
        .await?)
    }

    async fn agg_trade(&self, symbol: String) -> Result<UnboundedReceiver<AggregateTrade>> {
        let pairs: &[(&str, Option<String>); 2] = &[("symbol", Some(symbol.clone())), ("id", None)];

        let vars: HashMap<_, _> = pairs
            .iter()
            .filter_map(|&(k, ref v)| v.clone().map(|v| (k, v)))
            .collect();

        let id_opt: Option<String> = vars.get("id").map(ToString::to_string);

        let stream = replace_websocket_streams_placeholders("/<symbol>@aggTrade", &vars);

        let agg_trade_ws = create_stream_handler::<AggregateTrade>(
            WebsocketBase::WebsocketStreams(Arc::clone(&self.ws_streams)),
            stream,
            id_opt,
        )
        .await;

        let (agg_trade_tx, agg_trade_rx) = unbounded_channel();
        agg_trade_ws.on_message(move |data| {
            if let Err(e) = agg_trade_tx.send(data) {
                error!("Failed to send agg trade to channel: {}", e);
            }
        });
        Ok(agg_trade_rx)
    }

    async fn diff_book_depth(
        &self,
        symbol: String,
        update_speed: Option<String>,
    ) -> Result<UnboundedReceiver<DepthUpdate>> {
        let pairs: &[(&str, Option<String>); 3] = &[
            ("symbol", Some(symbol.clone())),
            ("id", None),
            ("updateSpeed", update_speed.clone()),
        ];

        let vars: HashMap<_, _> = pairs
            .iter()
            .filter_map(|&(k, ref v)| v.clone().map(|v| (k, v)))
            .collect();

        let id_opt: Option<String> = vars.get("id").map(ToString::to_string);

        let stream = replace_websocket_streams_placeholders("/<symbol>@depth@<updateSpeed>", &vars);

        let diff_book_ws = create_stream_handler::<DepthUpdate>(
            WebsocketBase::WebsocketStreams(Arc::clone(&self.ws_streams)),
            stream,
            id_opt,
        )
        .await;

        let (depth_tx, depth_rx) = unbounded_channel();
        diff_book_ws.on_message(move |data| {
            if let Err(e) = depth_tx.send(data) {
                error!("Failed to send depth update to channel: {}", e);
            }
        });

        Ok(depth_rx)
    }

    async fn ticker(&self, symbol: String) -> Result<UnboundedReceiver<TickerData>> {
        let pairs: &[(&str, Option<String>); 2] = &[("symbol", Some(symbol.clone())), ("id", None)];

        let vars: HashMap<_, _> = pairs
            .iter()
            .filter_map(|&(k, ref v)| v.clone().map(|v| (k, v)))
            .collect();

        let id_opt: Option<String> = vars.get("id").map(ToString::to_string);

        let stream = replace_websocket_streams_placeholders("/<symbol>@ticker", &vars);

        let ticker_ws = create_stream_handler::<TickerData>(
            WebsocketBase::WebsocketStreams(Arc::clone(&self.ws_streams)),
            stream,
            id_opt,
        )
        .await;

        let (ticker_tx, ticker_rx) = unbounded_channel();
        ticker_ws.on_message(move |data| {
            if let Err(e) = ticker_tx.send(data) {
                error!("Failed to send ticker to channel: {}", e);
            }
        });
        Ok(ticker_rx)
    }

    async fn rolling_window_ticker(
        &self,
        symbol: String,
        window_size: Option<String>,
    ) -> Result<UnboundedReceiver<WindowTickerData>> {
        let pairs: &[(&str, Option<String>); 3] = &[
            ("symbol", Some(symbol.clone())),
            ("windowSize", window_size.clone()),
            ("id", None),
        ];

        let vars: HashMap<_, _> = pairs
            .iter()
            .filter_map(|&(k, ref v)| v.clone().map(|v| (k, v)))
            .collect();

        let id_opt: Option<String> = vars.get("id").map(ToString::to_string);

        let stream = replace_websocket_streams_placeholders("/<symbol>@ticker_<windowSize>", &vars);

        let rolling_window_ticker_ws = create_stream_handler::<WindowTickerData>(
            WebsocketBase::WebsocketStreams(Arc::clone(&self.ws_streams)),
            stream,
            id_opt,
        )
        .await;

        let (rolling_window_ticker_tx, rolling_window_ticker_rx) = unbounded_channel();
        rolling_window_ticker_ws.on_message(move |data| {
            if let Err(e) = rolling_window_ticker_tx.send(data) {
                error!("Failed to send rolling window ticker to channel: {}", e);
            }
        });
        Ok(rolling_window_ticker_rx)
    }

    async fn disconnect(&self) -> Result<()> {
        info!("Disconnecting WebSocket streams");

        // Add timeout to prevent hanging on stubborn connections
        let disconnect_timeout = std::time::Duration::from_secs(1);

        match tokio::time::timeout(disconnect_timeout, self.ws_streams.disconnect()).await {
            Ok(Ok(())) => {
                info!("WebSocket streams disconnected successfully");
                Ok(())
            }
            Ok(Err(e)) => {
                warn!(
                    error = %e,
                    "WebSocket disconnect error (continuing shutdown)"
                );
                // Don't fail shutdown for WebSocket errors
                Ok(())
            }
            Err(_) => {
                warn!(
                    timeout_secs = disconnect_timeout.as_secs(),
                    "WebSocket disconnect timed out (continuing shutdown)"
                );
                // Don't fail shutdown for timeout
                Ok(())
            }
        }
    }
}

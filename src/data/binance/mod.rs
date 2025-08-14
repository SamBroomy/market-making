pub mod models;
pub mod protocol;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use binance_sdk::{
    common::{
        utils::{replace_websocket_streams_placeholders, send_request},
        websocket::{WebsocketBase, WebsocketStreams, create_stream_handler},
    },
    config::{ConfigurationRestApi, ConfigurationWebsocketStreams},
    constants::{SPOT_REST_API_PROD_URL, SPOT_WS_STREAMS_PROD_URL},
    models::RestApiResponse,
    spot::{
        rest_api::DepthParams,
        websocket_streams::{
            AggTradeParams, DiffBookDepthParams, RollingWindowTickerParams, TickerParams,
        },
    },
};
use models::DepthSnapshot;
use reqwest;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver as Receiver, unbounded_channel};

use crate::data::binance::models::{DepthUpdate, TickerData, WindowTickerData};

#[derive(Clone)]
pub struct BinanceClient {
    configuration: ConfigurationRestApi,
    ws_streams: Arc<WebsocketStreams>,
}

impl BinanceClient {
    #[must_use]
    pub async fn new() -> Self {
        const HAS_TIME_UNIT: bool = true;
        // let client = spot::SpotWsStreams::production(configuration);
        // let connection = client.connect().await?;

        let mut cfg = ConfigurationWebsocketStreams::builder()
            .build()
            .expect("Failed to build WebSocket configuration");
        cfg.ws_url = Some(SPOT_WS_STREAMS_PROD_URL.to_string());
        if !HAS_TIME_UNIT {
            cfg.time_unit = None;
        }

        // let websocket_streams_base = WebsocketStreamsBase::new(cfg, vec![]);
        // websocket_streams_base.clone().connect(streams).await?;
        // let client = spot::SpotWsStreams::production(configuration);

        let ws_streams = WebsocketStreams::new(cfg, vec![]);
        ws_streams
            .clone()
            .connect(vec![])
            .await
            .expect("Failed to connect WebSocket streams");
        let configuration = ConfigurationRestApi::builder()
            //   .api_key("YOUR_API_KEY")
            //   .api_secret("YOUR_SECRET_KEY")
            .base_path(SPOT_REST_API_PROD_URL.to_string())
            .build()
            .expect("Failed to build REST API configuration");

        Self {
            configuration,
            ws_streams,
        }
    }

    pub async fn depth_snapshot(
        &self,
        params: DepthParams,
    ) -> anyhow::Result<RestApiResponse<DepthSnapshot>> {
        let DepthParams { symbol, limit } = params;

        let mut query_params = BTreeMap::new();

        query_params.insert("symbol".to_string(), json!(symbol));

        if let Some(rw) = limit {
            query_params.insert("limit".to_string(), json!(rw));
        }

        send_request::<DepthSnapshot>(
            &self.configuration,
            "/api/v3/depth",
            reqwest::Method::GET,
            query_params,
            self.configuration.time_unit,
            false,
        )
        .await
    }

    pub async fn agg_trade(
        &self,
        params: AggTradeParams,
    ) -> anyhow::Result<Receiver<models::AggregateTrade>> {
        let AggTradeParams { symbol, id } = params;

        let pairs: &[(&str, Option<String>)] =
            &[("symbol", Some(symbol.clone())), ("id", id.clone())];

        let vars: HashMap<_, _> = pairs
            .iter()
            .filter_map(|&(k, ref v)| v.clone().map(|v| (k, v)))
            .collect();

        let id_opt: Option<String> = vars.get("id").map(ToString::to_string);

        let stream = replace_websocket_streams_placeholders("/<symbol>@aggTrade", &vars);

        let agg_trade_ws = create_stream_handler::<models::AggregateTrade>(
            WebsocketBase::WebsocketStreams(Arc::clone(&self.ws_streams)),
            stream,
            id_opt,
        )
        .await;

        let (agg_trade_tx, agg_trade_rx) = unbounded_channel();
        agg_trade_ws.on_message(move |data| {
            agg_trade_tx
                .send(data)
                .expect("Failed to send aggregate trade");
        });
        Ok(agg_trade_rx)
    }

    

    pub async fn diff_book_depth(
        &self,
        params: DiffBookDepthParams,
    ) -> anyhow::Result<Receiver<DepthUpdate>> {
        let DiffBookDepthParams {
            symbol,
            id,
            update_speed,
        } = params;

        let pairs: &[(&str, Option<String>)] = &[
            ("symbol", Some(symbol.clone())),
            ("id", id.clone()),
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
            depth_tx.send(data).expect("Failed to send depth update");
        });

        Ok(depth_rx)
    }

    pub async fn ticker(&self, params: TickerParams) -> anyhow::Result<Receiver<TickerData>> {
        let TickerParams { symbol, id } = params;

        let pairs: &[(&str, Option<String>)] =
            &[("symbol", Some(symbol.clone())), ("id", id.clone())];

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
            ticker_tx.send(data).expect("Failed to send ticker data");
        });
        Ok(ticker_rx)
    }

    pub async fn rolling_window_ticker(
        &self,
        params: RollingWindowTickerParams,
    ) -> anyhow::Result<Receiver<WindowTickerData>> {
        let RollingWindowTickerParams {
            symbol,
            window_size,
            id,
        } = params;

        let pairs: &[(&str, Option<String>)] = &[
            ("symbol", Some(symbol.clone())),
            ("windowSize", Some(window_size.as_str().to_string())),
            ("id", id.clone()),
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
            rolling_window_ticker_tx
                .send(data)
                .expect("Failed to send rolling window ticker data");
        });
        Ok(rolling_window_ticker_rx)
    }
}

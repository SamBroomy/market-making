use domain::{services::exchange::ExchangeDataProvider, settings::ExchangeSettings};

use crate::data_providers::binance::{config::BinanceSettings, connection::BinanceClient};

pub mod binance;

#[must_use]
pub async fn get_provider(cfg: ExchangeSettings) -> impl ExchangeDataProvider {
    let ExchangeSettings {
        name,
        use_testnet,
        startup_delay,
    } = cfg;

    let name = name.to_lowercase();
    match name.as_str() {
        "binance" => {
            let bs = BinanceSettings {
                use_testnet,
                startup_delay,
            };
            let config = BinanceSettings {
                use_testnet,
                startup_delay,
            };
            BinanceClient::from_config(config)
                .await
                .expect("Failed to create Binance data provider")
        }
        _ => panic!("Unsupported exchange: {name}"),
    }
}

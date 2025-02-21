use binance_spot_connector_rust::{hyper::BinanceHttpClient, market};
use marketmakerlib::binance::data::DepthSnapshot;

#[tokio::main]
async fn main() {
    let symbol = "BTCUSDT";
    let client = BinanceHttpClient::default();

    let data = client
        .send(market::depth(symbol).limit(5_000))
        .await
        .expect("Failed to get depth")
        .into_body_str()
        .await
        .expect("Failed to read response body");

    let snapshot =
        serde_json::from_str::<DepthSnapshot>(&data).expect("Failed to parse depth snapshot");
    println!("Snapshot: {:?}", snapshot);
}

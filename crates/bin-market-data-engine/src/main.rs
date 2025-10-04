use std::time::Duration;

use anyhow::Result;
use application::market_data_engine::run_multi_market_producer;
use domain::settings::Settings;
use support::shutdown::ShutdownCoordinator;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration from toml files and environment variables
    let settings = match Settings::get_configuration() {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    // Initialize tracing with configured log level
    tracing_subscriber::fmt::fmt()
        .with_env_filter(&settings.logging.level)
        .init();

    // Print configuration summary
    settings.print_summary();

    let shutdown_timeout = Duration::from_secs(10);
    let shutdown_coordinator = ShutdownCoordinator::new(shutdown_timeout);

    // Run the producer with graceful shutdown
    let producer_result = run_multi_market_producer(settings, shutdown_coordinator).await;

    // All cleanup completed - no need to wait further
    info!("Graceful shutdown completed successfully");

    // Check producer result
    match producer_result {
        Ok(()) => {
            info!("Market data producer completed successfully");
            Ok(())
        }
        Err(e) => {
            error!("Producer failed: {}", e);
            Err(e)
        }
    }
}

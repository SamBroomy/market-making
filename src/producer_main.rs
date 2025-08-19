use std::time::Duration;

use anyhow::Result;
use market_making::{
    config::Config,
    producer::{run_multi_symbol_producer, shutdown_global_resources},
    shutdown::{ShutdownCoordinator, setup_signal_handlers},
};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration from YAML files and environment variables
    let config = match Config::load() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Configuration validation failed: {}", e);
        std::process::exit(1);
    }

    // Initialize tracing with configured log level
    tracing_subscriber::fmt::fmt()
        .with_env_filter(&config.settings.logging.level)
        .init();

    // Print configuration summary
    config.print_summary();

    // Create shutdown coordinator with reduced timeout (most cleanup happens in <5s)
    let shutdown_timeout = Duration::from_secs(10); // 10 seconds for graceful shutdown
    let shutdown_coordinator = ShutdownCoordinator::new(shutdown_timeout);

    // Setup signal handlers for graceful shutdown
    setup_signal_handlers(shutdown_coordinator.clone());

    // Get symbols from configuration
    let symbols = config.get_symbols();

    info!("Starting market data producer for symbols: {:?}", symbols);
    info!(
        "Request weight for snapshots: {} (limit: {})",
        config.get_snapshot_request_weight(),
        config.get_snapshot_limit()
    );

    // Run the producer with graceful shutdown
    let producer_result = run_multi_symbol_producer(symbols, &config, &shutdown_coordinator).await;

    // Shutdown global resources (Iggy client, database pool, etc.)
    shutdown_global_resources().await;

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

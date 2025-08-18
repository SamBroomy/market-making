use std::time::Duration;

use anyhow::Result;
use market_making::{
    config::Config,
    producer::run_multi_symbol_producer,
    shutdown::{ShutdownCoordinator, setup_signal_handlers},
};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Parse configuration from CLI args and environment variables
    let config = Config::parse();

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Configuration validation failed: {}", e);
        std::process::exit(1);
    }

    // Initialize tracing with configured log level
    tracing_subscriber::fmt::fmt()
        .with_env_filter(&config.log_level)
        .init();

    // Print configuration summary
    config.print_summary();

    // Create shutdown coordinator with configurable timeout
    let shutdown_timeout = Duration::from_secs(30); // 30 seconds for graceful shutdown
    let shutdown_coordinator = ShutdownCoordinator::new(shutdown_timeout);

    // Setup signal handlers for graceful shutdown
    setup_signal_handlers(shutdown_coordinator.clone());

    // Get symbols from configuration
    let symbols = config.get_symbols();

    info!("Starting market data producer for symbols: {:?}", symbols);
    info!(
        "Request weight for snapshots: {} (limit: {})",
        config.get_snapshot_request_weight(),
        config.snapshot_limit
    );

    // Run the producer with graceful shutdown
    let producer_result = run_multi_symbol_producer(symbols, &config, &shutdown_coordinator).await;

    // Wait for graceful shutdown to complete
    info!("Waiting for graceful shutdown to complete...");
    let graceful_shutdown = shutdown_coordinator.wait_for_shutdown_with_timeout().await;

    if !graceful_shutdown {
        error!("Graceful shutdown timeout exceeded, forcing exit");
        std::process::exit(1);
    }

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

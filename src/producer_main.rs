use std::time::Duration;

use anyhow::Result;
use market_making::{
    producer::{run_multi_symbol_producer, shutdown_global_resources},
    settings::Settings,
    shutdown::{ShutdownCoordinator, setup_signal_handlers},
};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration from YAML files and environment variables
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

    // Create shutdown coordinator with reduced timeout (most cleanup happens in <5s)
    let shutdown_timeout = Duration::from_secs(10); // 10 seconds for graceful shutdown
    let shutdown_coordinator = ShutdownCoordinator::new(shutdown_timeout);

    // Setup signal handlers for graceful shutdown
    setup_signal_handlers(shutdown_coordinator.clone());

    // Run the producer with graceful shutdown
    let producer_result = run_multi_symbol_producer(settings, shutdown_coordinator).await;

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

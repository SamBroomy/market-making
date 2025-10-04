use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{signal, sync::broadcast, time::timeout};
use tracing::{error, info, warn};

/// Global shutdown coordinator
#[derive(Debug, Clone)]
pub struct ShutdownCoordinator {
    /// Atomic flag indicating if shutdown has been initiated
    flag: Arc<AtomicBool>,
    /// Broadcast channel for notifying all tasks of shutdown
    tx: broadcast::Sender<()>,
    /// Timeout for graceful shutdown before force exit
    timeout: Duration,
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator and will setup signal handlers
    #[must_use]
    pub fn new(shutdown_timeout: Duration) -> Self {
        let (shutdown_tx, _) = broadcast::channel(16);

        Self {
            flag: Arc::new(AtomicBool::new(false)),
            tx: shutdown_tx,
            timeout: shutdown_timeout,
        }
        .setup_signal_handlers()
    }

    /// Check if shutdown has been initiated
    #[must_use]
    pub fn is_shutting_down(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Get a receiver for shutdown notifications
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    /// Initiate graceful shutdown
    pub fn initiate_shutdown(&self) {
        if self.flag.swap(true, Ordering::Relaxed) {
            // Already shutting down
            return;
        }

        info!("Graceful shutdown initiated");

        // Notify all subscribers
        if let Err(e) = self.tx.send(()) {
            warn!("Failed to send shutdown signal: {}", e);
        }
    }

    /// Wait for shutdown with timeout
    pub async fn wait_for_shutdown_with_timeout(&self) -> bool {
        if let Ok(()) = timeout(self.timeout, self.wait_for_shutdown()).await {
            info!("Graceful shutdown completed within timeout");
            true
        } else {
            error!(
                "Shutdown timeout exceeded ({:?}), forcing exit",
                self.timeout
            );
            false
        }
    }

    /// Wait for shutdown signal indefinitely
    async fn wait_for_shutdown(&self) {
        let mut shutdown_rx = self.subscribe();
        let _ = shutdown_rx.recv().await;
    }

    /// Setup signal handlers for graceful shutdown
    fn setup_signal_handlers(self) -> Self {
        let coordinator = self.clone();
        tokio::spawn(async move {
            let coordinator_clone = coordinator.clone();

            // Handle SIGINT (Ctrl+C)
            tokio::spawn(async move {
                match signal::ctrl_c().await {
                    Ok(()) => {
                        info!("Received SIGINT (Ctrl+C)");
                        coordinator_clone.initiate_shutdown();
                    }
                    Err(err) => {
                        error!("Failed to listen for SIGINT: {}", err);
                    }
                }
            });

            // Handle SIGTERM (Docker/systemd shutdown)
            #[cfg(unix)]
            {
                use signal::unix::{SignalKind, signal};

                let coordinator_clone = coordinator.clone();
                tokio::spawn(async move {
                    match signal(SignalKind::terminate()) {
                        Ok(mut sigterm) => {
                            sigterm.recv().await;
                            info!("Received SIGTERM (termination signal)");
                            coordinator_clone.initiate_shutdown();
                        }
                        Err(err) => {
                            error!("Failed to listen for SIGTERM: {}", err);
                        }
                    }
                });
            }

            // Handle SIGHUP
            #[cfg(unix)]
            {
                use signal::unix::{SignalKind, signal};

                let coordinator_clone = coordinator.clone();
                tokio::spawn(async move {
                    match signal(SignalKind::hangup()) {
                        Ok(mut sighup) => {
                            while sighup.recv().await.is_some() {
                                info!("Received SIGHUP (hangup signal)");
                                // Could implement config reload here in the future?
                                coordinator_clone.initiate_shutdown();
                            }
                        }
                        Err(err) => {
                            error!("Failed to listen for SIGHUP: {}", err);
                        }
                    }
                });
            }
        });
        self
    }
}

/// Graceful task runner that respects shutdown signals
pub async fn run_with_shutdown<F, Fut>(
    task_name: &str,
    shutdown_coordinator: &ShutdownCoordinator,
    task: F,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: FnOnce(broadcast::Receiver<()>) -> Fut,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
{
    info!("Starting task: {}", task_name);

    let shutdown_rx = shutdown_coordinator.subscribe();

    tokio::select! {
        result = task(shutdown_rx) => {
            match &result {
                Ok(()) => info!("Task '{}' completed successfully", task_name),
                Err(e) => error!("Task '{}' failed: {}", task_name, e),
            }
            result
        }
        () = shutdown_coordinator.wait_for_shutdown() => {
            info!("Task '{}' interrupted by shutdown signal", task_name);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    async fn test_shutdown_coordinator() {
        let coordinator = ShutdownCoordinator::new(Duration::from_secs(1));

        assert!(!coordinator.is_shutting_down());

        coordinator.initiate_shutdown();

        assert!(coordinator.is_shutting_down());

        // Should complete quickly since shutdown was already initiated
        let completed = coordinator.wait_for_shutdown_with_timeout().await;
        assert!(completed);
    }

    #[tokio::test]
    async fn test_shutdown_timeout() {
        let coordinator = ShutdownCoordinator::new(Duration::from_millis(100));

        // Don't initiate shutdown, so timeout should trigger
        let completed = coordinator.wait_for_shutdown_with_timeout().await;
        assert!(!completed);
    }

    #[tokio::test]
    async fn test_run_with_shutdown() {
        let coordinator = ShutdownCoordinator::new(Duration::from_secs(1));

        // Start a task that would run forever
        let task_handle = tokio::spawn({
            let coordinator_clone = coordinator.clone();
            async move {
                run_with_shutdown(
                    "test_task",
                    &coordinator_clone,
                    |mut shutdown_rx| async move {
                        loop {
                            tokio::select! {
                                () = sleep(Duration::from_millis(10)) => {
                                    // Keep running
                                }
                                _ = shutdown_rx.recv() => {
                                    info!("Task received shutdown signal");
                                    break;
                                }
                            }
                        }
                        Ok(())
                    },
                )
                .await
            }
        });

        // Let the task start
        sleep(Duration::from_millis(50)).await;

        // Initiate shutdown
        coordinator.initiate_shutdown();

        // Task should complete quickly
        let result = task_handle.await.unwrap();
        assert!(result.is_ok());
    }
}

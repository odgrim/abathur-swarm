use crate::infrastructure::mcp::{
    error::McpError, error::Result, server_manager::McpServerManager,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Health monitor for MCP servers with auto-restart capability
///
/// Monitors server health every 10 seconds using ping requests.
/// Tracks consecutive failures and automatically restarts servers
/// after 3 failed health checks.
///
/// # Example
///
/// ```rust,no_run
/// use abathur::infrastructure::mcp::{McpServerManager, HealthMonitor};
/// use std::sync::Arc;
/// use tokio::sync::broadcast;
///
/// # async fn example() -> anyhow::Result<()> {
/// let manager = Arc::new(McpServerManager::new());
/// let (shutdown_tx, _) = broadcast::channel(1);
///
/// let monitor = HealthMonitor::new(manager);
/// let handle = monitor.start_monitoring("github-mcp".to_string(), shutdown_tx.subscribe());
///
/// // Later: trigger graceful shutdown
/// shutdown_tx.send(()).unwrap();
/// handle.await.unwrap();
/// # Ok(())
/// # }
/// ```
pub struct HealthMonitor {
    manager: Arc<McpServerManager>,
    check_interval: Duration,
    max_failures: u32,
    health_check_timeout: Duration,
}

impl HealthMonitor {
    /// Create a new health monitor
    ///
    /// Uses default configuration:
    /// - Check interval: 10 seconds
    /// - Max failures: 3
    /// - Health check timeout: 5 seconds
    pub fn new(manager: Arc<McpServerManager>) -> Self {
        Self {
            manager,
            check_interval: Duration::from_secs(10),
            max_failures: 3,
            health_check_timeout: Duration::from_secs(5),
        }
    }

    /// Create a health monitor with custom configuration
    pub fn with_config(
        manager: Arc<McpServerManager>,
        check_interval: Duration,
        max_failures: u32,
        health_check_timeout: Duration,
    ) -> Self {
        Self {
            manager,
            check_interval,
            max_failures,
            health_check_timeout,
        }
    }

    /// Start health monitoring background task for a specific server
    ///
    /// Spawns a tokio task that:
    /// 1. Periodically checks server health (10s interval)
    /// 2. Tracks consecutive failures
    /// 3. Auto-restarts after max failures (default: 3)
    /// 4. Listens for shutdown signals via broadcast channel
    ///
    /// Returns a `JoinHandle` that can be awaited for graceful shutdown.
    ///
    /// # Arguments
    ///
    /// * `server_name` - Name of the MCP server to monitor
    /// * `shutdown_rx` - Broadcast receiver for shutdown signals
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use tokio::sync::broadcast;
    ///
    /// # async fn example(monitor: &abathur::infrastructure::mcp::HealthMonitor) -> anyhow::Result<()> {
    /// let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    /// let handle = monitor.start_monitoring("github-mcp".to_string(), shutdown_rx);
    ///
    /// // Trigger shutdown
    /// shutdown_tx.send(()).unwrap();
    /// handle.await.unwrap();
    /// # Ok(())
    /// # }
    /// ```
    pub fn start_monitoring(
        &self,
        server_name: String,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> JoinHandle<()> {
        let manager = self.manager.clone();
        let check_interval = self.check_interval;
        let max_failures = self.max_failures;
        let health_check_timeout = self.health_check_timeout;

        tokio::spawn(async move {
            let mut consecutive_failures = 0;
            let mut interval = tokio::time::interval(check_interval);

            // Skip first tick (fires immediately)
            interval.tick().await;

            tracing::info!(
                server_name = %server_name,
                check_interval_secs = check_interval.as_secs(),
                max_failures = max_failures,
                "Started health monitoring for MCP server"
            );

            loop {
                tokio::select! {
                    // Periodic health check
                    _ = interval.tick() => {
                        match Self::health_check(&manager, &server_name, health_check_timeout).await {
                            Ok(true) => {
                                if consecutive_failures > 0 {
                                    tracing::info!(
                                        server_name = %server_name,
                                        "Health check recovered after {} failures",
                                        consecutive_failures
                                    );
                                }
                                consecutive_failures = 0;
                            }
                            Ok(false) | Err(_) => {
                                consecutive_failures += 1;
                                tracing::warn!(
                                    server_name = %server_name,
                                    consecutive_failures = consecutive_failures,
                                    max_failures = max_failures,
                                    "Health check failed for MCP server"
                                );

                                if consecutive_failures >= max_failures {
                                    tracing::error!(
                                        server_name = %server_name,
                                        consecutive_failures = consecutive_failures,
                                        "Max health check failures reached. Attempting restart."
                                    );

                                    if let Err(e) = manager.restart_server(&server_name).await {
                                        tracing::error!(
                                            server_name = %server_name,
                                            error = %e,
                                            "Failed to restart MCP server"
                                        );
                                    } else {
                                        tracing::info!(
                                            server_name = %server_name,
                                            "Successfully restarted MCP server"
                                        );
                                        consecutive_failures = 0;
                                    }
                                }
                            }
                        }
                    }

                    // Shutdown signal
                    _ = shutdown_rx.recv() => {
                        tracing::info!(
                            server_name = %server_name,
                            "Received shutdown signal, stopping health monitoring"
                        );
                        break;
                    }
                }
            }

            tracing::info!(
                server_name = %server_name,
                "Health monitoring stopped"
            );
        })
    }

    /// Perform health check on an MCP server
    ///
    /// Sends a ping request to the server and awaits response.
    /// Returns `Ok(true)` if server responds successfully within timeout,
    /// `Ok(false)` if server is unresponsive, or `Err` on errors.
    ///
    /// # Arguments
    ///
    /// * `manager` - Reference to MCP server manager
    /// * `server_name` - Name of the server to check
    /// * `timeout` - Maximum time to wait for response
    ///
    /// # Errors
    ///
    /// Returns `McpError` if:
    /// - Server not found
    /// - Transport communication fails
    /// - Health check times out
    async fn health_check(
        manager: &McpServerManager,
        server_name: &str,
        timeout: Duration,
    ) -> Result<bool> {
        // Get transport for the server
        let transport = manager.get_transport(server_name).await.map_err(|e| {
            tracing::debug!(
                server_name = %server_name,
                error = %e,
                "Failed to get transport for health check"
            );
            e
        })?;

        let mut transport = transport.lock().await;

        // Construct ping request
        let ping_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": format!("health_check_{}", server_name),
            "method": "ping",
            "params": {}
        });

        tracing::debug!(
            server_name = %server_name,
            "Sending health check ping"
        );

        // Send ping with timeout
        match tokio::time::timeout(timeout, transport.request(&ping_request)).await {
            Ok(Ok(_response)) => {
                tracing::debug!(
                    server_name = %server_name,
                    "Health check ping successful"
                );
                Ok(true)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    server_name = %server_name,
                    error = %e,
                    "Health check request failed"
                );
                Ok(false)
            }
            Err(_) => {
                tracing::warn!(
                    server_name = %server_name,
                    timeout_secs = timeout.as_secs(),
                    "Health check timed out"
                );
                Err(McpError::HealthCheckTimeout(server_name.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_monitor_creation() {
        let manager = Arc::new(McpServerManager {});
        let monitor = HealthMonitor::new(manager);

        assert_eq!(monitor.check_interval, Duration::from_secs(10));
        assert_eq!(monitor.max_failures, 3);
        assert_eq!(monitor.health_check_timeout, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_health_monitor_custom_config() {
        let manager = Arc::new(McpServerManager {});
        let monitor =
            HealthMonitor::with_config(manager, Duration::from_secs(5), 5, Duration::from_secs(3));

        assert_eq!(monitor.check_interval, Duration::from_secs(5));
        assert_eq!(monitor.max_failures, 5);
        assert_eq!(monitor.health_check_timeout, Duration::from_secs(3));
    }

    #[tokio::test]
    async fn test_health_monitor_graceful_shutdown() {
        let manager = Arc::new(McpServerManager {});
        let monitor = HealthMonitor::new(manager);

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // Start monitoring
        let handle = monitor.start_monitoring("test-server".to_string(), shutdown_rx);

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Trigger shutdown
        shutdown_tx.send(()).unwrap();

        // Wait for graceful shutdown
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(result.is_ok(), "Health monitor should shutdown gracefully");
    }
}

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::time::interval;
use tracing::{debug, info};

/// Resource limits configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory usage in MB (for tracking only, no enforced throttling)
    pub max_memory_mb: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: 4096,
        }
    }
}

/// Current resource usage status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatus {
    /// Current CPU usage percentage (0.0-100.0)
    pub cpu_percent: f32,

    /// Current memory usage in MB
    pub memory_mb: u64,

    /// Timestamp of the measurement
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Resource monitor events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceEvent {
    /// Resource status update
    StatusUpdate(ResourceStatus),

    /// Monitor shutdown
    Shutdown,
}

/// Background resource monitor with async concurrency
///
/// Monitors system CPU and memory usage at configurable intervals,
/// tracks resource limits, and provides adaptive throttling signals.
///
/// Uses tokio primitives for concurrent monitoring:
/// - RwLock for shared state (read-heavy access pattern)
/// - broadcast channel for event notifications (one-to-many)
/// - interval timer for periodic monitoring (tokio::time)
/// - select! for graceful shutdown handling
///
/// # Examples
///
/// ```
/// use abathur::application::ResourceMonitor;
/// use abathur::application::resource_monitor::ResourceLimits;
///
/// # async fn example() -> anyhow::Result<()> {
/// let limits = ResourceLimits::default();
/// let monitor = ResourceMonitor::new(limits);
///
/// // Start background monitoring
/// let handle = monitor.start(Duration::from_secs(5)).await?;
///
/// // Subscribe to events
/// let mut events = monitor.subscribe();
///
/// // Check if throttling needed
/// if monitor.should_throttle().await {
///     // Reduce concurrent operations
/// }
///
/// // Shutdown gracefully
/// monitor.shutdown().await?;
/// handle.await??;
/// # Ok(())
/// # }
/// ```
pub struct ResourceMonitor {
    /// Shared system info (uses RwLock for read-heavy access)
    system: Arc<RwLock<System>>,

    /// Resource limits configuration
    limits: ResourceLimits,

    /// Current resource status (cached for quick access)
    current_status: Arc<RwLock<Option<ResourceStatus>>>,

    /// Event broadcaster (one-to-many notification)
    event_tx: broadcast::Sender<ResourceEvent>,

    /// Shutdown signal broadcaster
    shutdown_tx: broadcast::Sender<()>,
}

impl ResourceMonitor {
    /// Create a new resource monitor with specified limits
    pub fn new(limits: ResourceLimits) -> Self {
        // Create system with minimal refresh kind for efficiency
        let refresh_kind = RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything());

        let system = System::new_with_specifics(refresh_kind);

        let (event_tx, _) = broadcast::channel(100); // Buffer 100 events
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            system: Arc::new(RwLock::new(system)),
            limits,
            current_status: Arc::new(RwLock::new(None)),
            event_tx,
            shutdown_tx,
        }
    }

    /// Start background monitoring task
    ///
    /// Spawns a tokio task that monitors resources at the specified interval.
    /// Returns a JoinHandle that completes when the monitor shuts down.
    ///
    /// # Arguments
    ///
    /// * `interval_duration` - How frequently to check resources (e.g., 5s)
    pub async fn start(
        &self,
        interval_duration: Duration,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let system = Arc::clone(&self.system);
        let current_status = Arc::clone(&self.current_status);
        let event_tx = self.event_tx.clone();
        let limits = self.limits.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            let mut check_interval = interval(interval_duration);

            info!(
                interval_secs = interval_duration.as_secs(),
                memory_limit_mb = limits.max_memory_mb,
                "Resource monitor started"
            );

            loop {
                tokio::select! {
                    // Periodic resource check
                    _ = check_interval.tick() => {
                        // Refresh system info
                        let status = {
                            let mut sys = system.write().await;
                            sys.refresh_cpu_all();
                            sys.refresh_memory();

                            // Calculate current usage
                            let cpu_percent = sys.global_cpu_usage();
                            let memory_mb = sys.used_memory() / 1024 / 1024;

                            ResourceStatus {
                                cpu_percent,
                                memory_mb,
                                timestamp: chrono::Utc::now(),
                            }
                        };

                        // Update cached status
                        {
                            let mut current = current_status.write().await;
                            *current = Some(status.clone());
                        }

                        // Broadcast status update
                        let _ = event_tx.send(ResourceEvent::StatusUpdate(status.clone()));

                        debug!(
                            cpu_percent = status.cpu_percent,
                            memory_mb = status.memory_mb,
                            "Resource check completed"
                        );
                    }

                    // Shutdown signal
                    _ = shutdown_rx.recv() => {
                        info!("Resource monitor shutting down");
                        let _ = event_tx.send(ResourceEvent::Shutdown);
                        break;
                    }
                }
            }

            info!("Resource monitor stopped");
            Ok(())
        });

        Ok(handle)
    }

    /// Get current resource status
    ///
    /// Returns the most recent cached status, or None if monitoring hasn't started.
    pub async fn get_status(&self) -> Option<ResourceStatus> {
        let status = self.current_status.read().await;
        status.clone()
    }


    /// Subscribe to resource events
    ///
    /// Returns a broadcast receiver that yields ResourceEvent messages.
    /// Multiple subscribers can listen simultaneously (one-to-many).
    pub fn subscribe(&self) -> broadcast::Receiver<ResourceEvent> {
        self.event_tx.subscribe()
    }

    /// Manually trigger a resource check
    ///
    /// Useful for on-demand status updates outside the periodic interval.
    pub async fn check_resources(&self) -> Result<ResourceStatus> {
        let mut sys = self.system.write().await;
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let cpu_percent = sys.global_cpu_usage();
        let memory_mb = sys.used_memory() / 1024 / 1024;

        let status = ResourceStatus {
            cpu_percent,
            memory_mb,
            timestamp: chrono::Utc::now(),
        };

        // Update cached status
        {
            let mut current = self.current_status.write().await;
            *current = Some(status.clone());
        }

        Ok(status)
    }

    /// Shutdown the background monitoring task
    ///
    /// Broadcasts shutdown signal to all monitoring tasks.
    /// Use the JoinHandle from start() to wait for completion.
    pub async fn shutdown(&self) -> Result<()> {
        info!("Initiating resource monitor shutdown");
        self.shutdown_tx
            .send(())
            .context("Failed to send shutdown signal")?;
        Ok(())
    }

    /// Get resource limits configuration
    pub fn get_limits(&self) -> &ResourceLimits {
        &self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resource_monitor_creation() {
        let limits = ResourceLimits::default();
        let monitor = ResourceMonitor::new(limits.clone());

        assert_eq!(monitor.get_limits().max_memory_mb, limits.max_memory_mb);
    }

    #[tokio::test]
    async fn test_manual_resource_check() {
        let limits = ResourceLimits::default();
        let monitor = ResourceMonitor::new(limits);

        let status = monitor.check_resources().await.unwrap();

        assert!(status.cpu_percent >= 0.0);
        assert!(status.memory_mb > 0);
        assert!(status.timestamp <= chrono::Utc::now());
    }

    #[tokio::test]
    async fn test_status_caching() {
        let limits = ResourceLimits::default();
        let monitor = ResourceMonitor::new(limits);

        // Initially no status
        assert!(monitor.get_status().await.is_none());

        // After check, status is cached
        let _ = monitor.check_resources().await.unwrap();
        let cached_status = monitor.get_status().await;
        assert!(cached_status.is_some());
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let limits = ResourceLimits::default();
        let monitor = ResourceMonitor::new(limits);

        let mut events = monitor.subscribe();

        // Start monitoring with very short interval
        let handle = monitor.start(Duration::from_millis(100)).await.unwrap();

        // Wait for at least one status update
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("Timeout waiting for event")
            .expect("Event channel closed");

        match event {
            ResourceEvent::StatusUpdate(status) => {
                assert!(status.cpu_percent >= 0.0);
                assert!(status.memory_mb > 0);
            }
            _ => panic!("Expected StatusUpdate event"),
        }

        // Shutdown
        monitor.shutdown().await.unwrap();
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let limits = ResourceLimits::default();
        let monitor = ResourceMonitor::new(limits);

        let mut events = monitor.subscribe();

        // Start monitoring
        let handle = monitor.start(Duration::from_secs(1)).await.unwrap();

        // Trigger shutdown
        monitor.shutdown().await.unwrap();

        // Should receive shutdown event
        let shutdown_received = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.recv().await {
                    Ok(ResourceEvent::Shutdown) => return true,
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        })
        .await
        .unwrap_or(false);

        assert!(shutdown_received, "Should receive shutdown event");

        // Handle should complete
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("Timeout waiting for monitor to shutdown")
            .expect("Monitor task panicked")
            .expect("Monitor returned error");
    }

}

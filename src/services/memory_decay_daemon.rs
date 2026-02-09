//! Memory decay background daemon.
//!
//! Runs scheduled maintenance tasks for the memory system:
//! - Pruning expired memories
//! - Pruning decayed memories below threshold
//! - Promoting memories based on access patterns

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Instant};

use crate::domain::errors::DomainResult;
use crate::domain::ports::MemoryRepository;
use crate::services::event_bus::EventBus;
use crate::services::memory_service::{MaintenanceReport, MemoryService};

/// Configuration for the memory decay daemon.
#[derive(Debug, Clone)]
pub struct DecayDaemonConfig {
    /// Interval between maintenance runs.
    pub maintenance_interval: Duration,
    /// Whether to run on startup.
    pub run_on_startup: bool,
    /// Maximum consecutive failures before stopping.
    pub max_consecutive_failures: u32,
    /// Enable verbose logging.
    pub verbose: bool,
}

impl Default for DecayDaemonConfig {
    fn default() -> Self {
        Self {
            maintenance_interval: Duration::from_secs(300), // 5 minutes
            run_on_startup: true,
            max_consecutive_failures: 5,
            verbose: false,
        }
    }
}

impl DecayDaemonConfig {
    /// Create config with custom interval.
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            maintenance_interval: interval,
            ..Default::default()
        }
    }

    /// Create config for frequent maintenance (testing).
    pub fn frequent() -> Self {
        Self {
            maintenance_interval: Duration::from_secs(10),
            run_on_startup: true,
            max_consecutive_failures: 3,
            verbose: true,
        }
    }
}

/// Event emitted by the decay daemon.
#[derive(Debug, Clone)]
pub enum DecayDaemonEvent {
    /// Daemon started.
    Started,
    /// Maintenance run started.
    MaintenanceStarted { run_number: u64 },
    /// Maintenance run completed.
    MaintenanceCompleted {
        run_number: u64,
        report: MaintenanceReport,
        duration_ms: u64,
    },
    /// Maintenance run failed.
    MaintenanceFailed {
        run_number: u64,
        error: String,
    },
    /// Daemon stopped.
    Stopped { reason: StopReason },
}

/// Reason the daemon stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Requested to stop.
    Requested,
    /// Too many consecutive failures.
    TooManyFailures,
    /// Channel closed.
    ChannelClosed,
}

/// Status of the decay daemon.
#[derive(Debug, Clone)]
pub struct DaemonStatus {
    /// Whether the daemon is running.
    pub running: bool,
    /// Total maintenance runs.
    pub total_runs: u64,
    /// Successful runs.
    pub successful_runs: u64,
    /// Failed runs.
    pub failed_runs: u64,
    /// Last run time.
    pub last_run: Option<Instant>,
    /// Total memories pruned.
    pub total_pruned: u64,
    /// Total memories promoted.
    pub total_promoted: u64,
}

impl Default for DaemonStatus {
    fn default() -> Self {
        Self {
            running: false,
            total_runs: 0,
            successful_runs: 0,
            failed_runs: 0,
            last_run: None,
            total_pruned: 0,
            total_promoted: 0,
        }
    }
}

/// Handle to control the decay daemon.
pub struct DaemonHandle {
    stop_flag: Arc<AtomicBool>,
    status: Arc<RwLock<DaemonStatus>>,
}

impl DaemonHandle {
    /// Request the daemon to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
    }

    /// Check if stop was requested.
    pub fn is_stop_requested(&self) -> bool {
        self.stop_flag.load(Ordering::Acquire)
    }

    /// Get current daemon status.
    pub async fn status(&self) -> DaemonStatus {
        self.status.read().await.clone()
    }
}

/// Memory decay background daemon.
pub struct MemoryDecayDaemon<R>
where
    R: MemoryRepository + Send + Sync + 'static,
{
    memory_service: Arc<MemoryService<R>>,
    config: DecayDaemonConfig,
    status: Arc<RwLock<DaemonStatus>>,
    stop_flag: Arc<AtomicBool>,
    event_bus: Option<Arc<EventBus>>,
}

impl<R> MemoryDecayDaemon<R>
where
    R: MemoryRepository + Send + Sync + 'static,
{
    /// Create a new decay daemon.
    pub fn new(memory_service: Arc<MemoryService<R>>, config: DecayDaemonConfig) -> Self {
        Self {
            memory_service,
            config,
            status: Arc::new(RwLock::new(DaemonStatus::default())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            event_bus: None,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults(memory_service: Arc<MemoryService<R>>) -> Self {
        Self::new(memory_service, DecayDaemonConfig::default())
    }

    /// Set the event bus for publishing maintenance events.
    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Get a handle to control the daemon.
    pub fn handle(&self) -> DaemonHandle {
        DaemonHandle {
            stop_flag: self.stop_flag.clone(),
            status: self.status.clone(),
        }
    }

    /// Run the daemon, returning a channel for events.
    pub async fn run(self) -> mpsc::Receiver<DecayDaemonEvent> {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            self.run_loop(tx).await;
        });

        rx
    }

    /// Run the daemon with an existing sender.
    pub async fn run_with_sender(self, tx: mpsc::Sender<DecayDaemonEvent>) {
        self.run_loop(tx).await;
    }

    /// Main daemon loop.
    async fn run_loop(self, tx: mpsc::Sender<DecayDaemonEvent>) {
        // Mark as running
        {
            let mut status = self.status.write().await;
            status.running = true;
        }

        let _ = tx.send(DecayDaemonEvent::Started).await;

        let mut consecutive_failures = 0u32;
        let mut interval_timer = interval(self.config.maintenance_interval);

        // Run on startup if configured
        if self.config.run_on_startup {
            self.run_maintenance_cycle(&tx, &mut consecutive_failures).await;
        }

        loop {
            // Wait for next interval or stop signal
            tokio::select! {
                _ = interval_timer.tick() => {
                    if self.stop_flag.load(Ordering::Acquire) {
                        break;
                    }

                    self.run_maintenance_cycle(&tx, &mut consecutive_failures).await;

                    // Check for too many failures
                    if consecutive_failures >= self.config.max_consecutive_failures {
                        let _ = tx.send(DecayDaemonEvent::Stopped {
                            reason: StopReason::TooManyFailures,
                        }).await;
                        break;
                    }
                }
            }

            // Check stop flag
            if self.stop_flag.load(Ordering::Acquire) {
                break;
            }
        }

        // Mark as stopped
        {
            let mut status = self.status.write().await;
            status.running = false;
        }

        if !self.stop_flag.load(Ordering::Acquire) {
            let _ = tx.send(DecayDaemonEvent::Stopped {
                reason: StopReason::TooManyFailures,
            }).await;
        } else {
            let _ = tx.send(DecayDaemonEvent::Stopped {
                reason: StopReason::Requested,
            }).await;
        }
    }

    /// Run a single maintenance cycle.
    async fn run_maintenance_cycle(
        &self,
        tx: &mpsc::Sender<DecayDaemonEvent>,
        consecutive_failures: &mut u32,
    ) {
        let run_number = {
            let mut status = self.status.write().await;
            status.total_runs += 1;
            status.total_runs
        };

        let _ = tx.send(DecayDaemonEvent::MaintenanceStarted { run_number }).await;

        let start = Instant::now();
        let result = self.memory_service.run_maintenance().await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok((report, events)) => {
                *consecutive_failures = 0;

                // Publish memory service events via EventBus
                if let Some(ref bus) = self.event_bus {
                    for event in events {
                        bus.publish(event).await;
                    }
                }

                {
                    let mut status = self.status.write().await;
                    status.successful_runs += 1;
                    status.last_run = Some(Instant::now());
                    status.total_pruned += report.expired_pruned + report.decayed_pruned;
                    status.total_promoted += report.promoted;
                }

                let _ = tx.send(DecayDaemonEvent::MaintenanceCompleted {
                    run_number,
                    report,
                    duration_ms,
                }).await;
            }
            Err(e) => {
                *consecutive_failures += 1;

                {
                    let mut status = self.status.write().await;
                    status.failed_runs += 1;
                }

                let _ = tx.send(DecayDaemonEvent::MaintenanceFailed {
                    run_number,
                    error: e.to_string(),
                }).await;
            }
        }
    }

    /// Run maintenance once (for testing or manual invocation).
    pub async fn run_once(&self) -> DomainResult<MaintenanceReport> {
        let (report, events) = self.memory_service.run_maintenance().await?;

        // Publish memory service events via EventBus
        if let Some(ref bus) = self.event_bus {
            for event in events {
                bus.publish(event).await;
            }
        }

        Ok(report)
    }

    /// Get current status.
    pub async fn status(&self) -> DaemonStatus {
        self.status.read().await.clone()
    }

    /// Get configuration.
    pub fn config(&self) -> &DecayDaemonConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = DecayDaemonConfig::default();
        assert_eq!(config.maintenance_interval, Duration::from_secs(300));
        assert!(config.run_on_startup);
        assert_eq!(config.max_consecutive_failures, 5);
    }

    #[test]
    fn test_config_with_interval() {
        let config = DecayDaemonConfig::with_interval(Duration::from_secs(60));
        assert_eq!(config.maintenance_interval, Duration::from_secs(60));
    }

    #[test]
    fn test_config_frequent() {
        let config = DecayDaemonConfig::frequent();
        assert_eq!(config.maintenance_interval, Duration::from_secs(10));
        assert!(config.verbose);
    }

    #[test]
    fn test_daemon_status_default() {
        let status = DaemonStatus::default();
        assert!(!status.running);
        assert_eq!(status.total_runs, 0);
        assert_eq!(status.successful_runs, 0);
        assert_eq!(status.failed_runs, 0);
        assert!(status.last_run.is_none());
    }

    #[test]
    fn test_stop_reason_equality() {
        assert_eq!(StopReason::Requested, StopReason::Requested);
        assert_ne!(StopReason::Requested, StopReason::TooManyFailures);
    }
}

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
use crate::services::event_bus::{EventBus, EventCategory, EventPayload, EventSeverity};
use crate::services::event_factory;
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
    /// Number of consecutive failures that triggers a degraded warning.
    /// Must be less than `max_consecutive_failures`.
    pub warning_threshold: u32,
    /// Enable verbose logging.
    pub verbose: bool,
}

impl Default for DecayDaemonConfig {
    fn default() -> Self {
        Self {
            maintenance_interval: Duration::from_secs(300), // 5 minutes
            run_on_startup: true,
            max_consecutive_failures: 5,
            warning_threshold: 3,
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
            warning_threshold: 2,
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
        consecutive_failures: u32,
        max_consecutive_failures: u32,
    },
    /// Warning: consecutive failures have reached the warning threshold.
    /// Emitted once when `consecutive_failures == warning_threshold`.
    FailureThresholdWarning {
        consecutive_failures: u32,
        max_consecutive_failures: u32,
        latest_error: String,
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
#[derive(Default)]
pub struct DaemonStatus {
    /// Whether the daemon is running.
    pub running: bool,
    /// Total maintenance runs.
    pub total_runs: u64,
    /// Successful runs.
    pub successful_runs: u64,
    /// Failed runs.
    pub failed_runs: u64,
    /// Current consecutive failure count.
    pub consecutive_failures: u32,
    /// Last run time.
    pub last_run: Option<Instant>,
    /// Total memories pruned.
    pub total_pruned: u64,
    /// Total memories promoted.
    pub total_promoted: u64,
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

        let stopped_reason = loop {
            // Wait for next interval or stop signal
            tokio::select! {
                _ = interval_timer.tick() => {
                    if self.stop_flag.load(Ordering::Acquire) {
                        break StopReason::Requested;
                    }

                    self.run_maintenance_cycle(&tx, &mut consecutive_failures).await;

                    // Check for too many failures
                    if consecutive_failures >= self.config.max_consecutive_failures {
                        break StopReason::TooManyFailures;
                    }
                }
            }

            // Check stop flag
            if self.stop_flag.load(Ordering::Acquire) {
                break StopReason::Requested;
            }
        };

        // Mark as stopped
        {
            let mut status = self.status.write().await;
            status.running = false;
        }

        // Emit exactly one Stopped event.
        let _ = tx.send(DecayDaemonEvent::Stopped { reason: stopped_reason }).await;
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
                    status.consecutive_failures = 0;
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
                let error_str = e.to_string();

                {
                    let mut status = self.status.write().await;
                    status.failed_runs += 1;
                    status.consecutive_failures = *consecutive_failures;
                }

                let _ = tx.send(DecayDaemonEvent::MaintenanceFailed {
                    run_number,
                    error: error_str.clone(),
                    consecutive_failures: *consecutive_failures,
                    max_consecutive_failures: self.config.max_consecutive_failures,
                }).await;

                // Publish failure event to EventBus for system-wide observability
                if let Some(ref bus) = self.event_bus {
                    bus.publish(event_factory::make_event(
                        EventSeverity::Warning,
                        EventCategory::Memory,
                        None,
                        None,
                        EventPayload::MemoryMaintenanceFailed {
                            run_number,
                            error: error_str.clone(),
                            consecutive_failures: *consecutive_failures,
                            max_consecutive_failures: self.config.max_consecutive_failures,
                        },
                    )).await;
                }

                // Emit a warning when we hit the warning threshold (exactly once).
                if *consecutive_failures == self.config.warning_threshold
                    && self.config.warning_threshold > 0
                    && self.config.warning_threshold < self.config.max_consecutive_failures
                {
                    tracing::warn!(
                        consecutive_failures = *consecutive_failures,
                        max = self.config.max_consecutive_failures,
                        "Memory decay daemon approaching failure limit"
                    );
                    let _ = tx.send(DecayDaemonEvent::FailureThresholdWarning {
                        consecutive_failures: *consecutive_failures,
                        max_consecutive_failures: self.config.max_consecutive_failures,
                        latest_error: error_str.clone(),
                    }).await;

                    // Publish degraded warning to EventBus for system-wide observability
                    if let Some(ref bus) = self.event_bus {
                        bus.publish(event_factory::make_event(
                            EventSeverity::Error,
                            EventCategory::Memory,
                            None,
                            None,
                            EventPayload::MemoryDaemonDegraded {
                                consecutive_failures: *consecutive_failures,
                                max_consecutive_failures: self.config.max_consecutive_failures,
                                latest_error: error_str,
                            },
                        )).await;
                    }
                }
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
    use std::sync::atomic::AtomicBool;

    use async_trait::async_trait;
    use uuid::Uuid;

    use crate::domain::errors::{DomainError, DomainResult};
    use crate::domain::models::{Memory, MemoryQuery, MemoryTier};
    use crate::domain::ports::MemoryRepository;

    /// A mock MemoryRepository that can be configured to fail on `prune_expired()`.
    struct FailingMemoryRepository {
        should_fail: AtomicBool,
    }

    impl FailingMemoryRepository {
        fn new(should_fail: bool) -> Self {
            Self {
                should_fail: AtomicBool::new(should_fail),
            }
        }

        fn set_should_fail(&self, fail: bool) {
            self.should_fail.store(fail, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl MemoryRepository for FailingMemoryRepository {
        async fn store(&self, _memory: &Memory) -> DomainResult<()> { Ok(()) }
        async fn get(&self, _id: Uuid) -> DomainResult<Option<Memory>> { Ok(None) }
        async fn get_by_key(&self, _key: &str, _namespace: &str) -> DomainResult<Option<Memory>> { Ok(None) }
        async fn update(&self, _memory: &Memory) -> DomainResult<()> { Ok(()) }
        async fn delete(&self, _id: Uuid) -> DomainResult<()> { Ok(()) }
        async fn query(&self, _query: MemoryQuery) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn search(&self, _query: &str, _namespace: Option<&str>, _limit: usize) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn list_by_tier(&self, _tier: MemoryTier) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn list_by_namespace(&self, _namespace: &str) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn get_expired(&self) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn prune_expired(&self) -> DomainResult<u64> {
            if self.should_fail.load(Ordering::SeqCst) {
                Err(DomainError::DatabaseError("simulated prune failure".to_string()))
            } else {
                Ok(0)
            }
        }
        async fn get_decayed(&self, _threshold: f32) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn get_for_task(&self, _task_id: Uuid) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn get_for_goal(&self, _goal_id: Uuid) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn count_by_tier(&self) -> DomainResult<std::collections::HashMap<MemoryTier, u64>> { Ok(std::collections::HashMap::new()) }
    }

    /// Helper to create a daemon with the FailingMemoryRepository.
    fn make_daemon(
        should_fail: bool,
        config: DecayDaemonConfig,
    ) -> (MemoryDecayDaemon<FailingMemoryRepository>, Arc<FailingMemoryRepository>) {
        let repo = Arc::new(FailingMemoryRepository::new(should_fail));
        let service = Arc::new(MemoryService::new(repo.clone()));
        let daemon = MemoryDecayDaemon::new(service, config);
        (daemon, repo)
    }

    #[tokio::test]
    async fn test_run_once_basic_execution() {
        let (daemon, _repo) = make_daemon(false, DecayDaemonConfig::default());
        let report = daemon.run_once().await.expect("run_once should succeed");
        assert_eq!(report.expired_pruned, 0);
        assert_eq!(report.decayed_pruned, 0);
        assert_eq!(report.promoted, 0);
        assert_eq!(report.conflicts_resolved, 0);
    }

    #[tokio::test]
    async fn test_consecutive_failure_counting_and_warning() {
        // Config: warning_threshold=2, max_consecutive_failures=5
        let config = DecayDaemonConfig {
            maintenance_interval: Duration::from_millis(50),
            run_on_startup: false,
            max_consecutive_failures: 5,
            warning_threshold: 2,
            verbose: false,
        };
        let (daemon, _repo) = make_daemon(true, config);

        let handle = daemon.handle();
        let mut rx = daemon.run().await;

        // Collect events until we see FailureThresholdWarning, then stop.
        let mut saw_warning = false;
        let mut failure_events = Vec::new();

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                match &event {
                    DecayDaemonEvent::MaintenanceFailed { consecutive_failures, .. } => {
                        failure_events.push(*consecutive_failures);
                    }
                    DecayDaemonEvent::FailureThresholdWarning {
                        consecutive_failures,
                        max_consecutive_failures,
                        ..
                    } => {
                        assert_eq!(*consecutive_failures, 2);
                        assert_eq!(*max_consecutive_failures, 5);
                        saw_warning = true;
                        handle.stop();
                    }
                    DecayDaemonEvent::Stopped { .. } => break,
                    _ => {}
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test timed out waiting for events");
        assert!(saw_warning, "Should have received FailureThresholdWarning");
        assert!(
            failure_events.contains(&1) && failure_events.contains(&2),
            "Should have seen consecutive failure counts 1 and 2, got: {:?}",
            failure_events,
        );
    }

    #[tokio::test]
    async fn test_max_consecutive_failures_stops_daemon() {
        // Config: max=3 so daemon stops after 3 consecutive failures
        let config = DecayDaemonConfig {
            maintenance_interval: Duration::from_millis(50),
            run_on_startup: false,
            max_consecutive_failures: 3,
            warning_threshold: 2,
            verbose: false,
        };
        let (daemon, _repo) = make_daemon(true, config);

        let handle = daemon.handle();
        let mut rx = daemon.run().await;

        let mut stop_reason = None;

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                if let DecayDaemonEvent::Stopped { reason } = event {
                    stop_reason = Some(reason);
                    break;
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test timed out waiting for daemon to stop");
        assert_eq!(
            stop_reason,
            Some(StopReason::TooManyFailures),
            "Daemon should stop due to TooManyFailures"
        );

        let status = handle.status().await;
        assert!(!status.running);
        assert_eq!(status.consecutive_failures, 3);
        assert_eq!(status.failed_runs, 3);
    }

    #[tokio::test]
    async fn test_warning_threshold_equal_to_max_failures_skips_warning() {
        // When warning_threshold == max_consecutive_failures, the daemon should
        // stop with TooManyFailures WITHOUT ever emitting FailureThresholdWarning.
        // The warning fires at `consecutive_failures == warning_threshold` but the
        // stop check (`>= max_consecutive_failures`) is evaluated first in the loop,
        // so the daemon exits before reaching the warning branch.
        let config = DecayDaemonConfig {
            maintenance_interval: Duration::from_millis(50),
            run_on_startup: false,
            max_consecutive_failures: 3,
            warning_threshold: 3, // equal to max
            verbose: false,
        };
        let (daemon, _repo) = make_daemon(true, config);

        let handle = daemon.handle();
        let mut rx = daemon.run().await;

        let mut saw_warning = false;
        let mut stop_reason = None;

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                match &event {
                    DecayDaemonEvent::FailureThresholdWarning { .. } => {
                        saw_warning = true;
                    }
                    DecayDaemonEvent::Stopped { reason } => {
                        stop_reason = Some(reason.clone());
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test timed out waiting for daemon to stop");
        assert!(
            !saw_warning,
            "FailureThresholdWarning should NOT fire when warning_threshold == max_consecutive_failures"
        );
        assert_eq!(
            stop_reason,
            Some(StopReason::TooManyFailures),
            "Daemon should still stop with TooManyFailures"
        );

        let status = handle.status().await;
        assert!(!status.running);
        assert_eq!(status.consecutive_failures, 3);
    }

    #[tokio::test]
    async fn test_successful_run_resets_failure_counter() {
        // Config: warning_threshold=2, max=5
        let config = DecayDaemonConfig {
            maintenance_interval: Duration::from_millis(50),
            run_on_startup: false,
            max_consecutive_failures: 5,
            warning_threshold: 2,
            verbose: false,
        };
        let (daemon, repo) = make_daemon(true, config);

        let handle = daemon.handle();
        let mut rx = daemon.run().await;

        // Wait for first failure, then flip repo to succeed, then observe reset.
        let mut saw_success_after_failure = false;
        let mut max_consecutive_seen = 0u32;

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                match &event {
                    DecayDaemonEvent::MaintenanceFailed { consecutive_failures, .. } => {
                        if *consecutive_failures > max_consecutive_seen {
                            max_consecutive_seen = *consecutive_failures;
                        }
                        // After first failure, switch to success mode
                        if *consecutive_failures == 1 {
                            repo.set_should_fail(false);
                        }
                    }
                    DecayDaemonEvent::MaintenanceCompleted { .. } => {
                        if max_consecutive_seen > 0 {
                            saw_success_after_failure = true;
                            handle.stop();
                        }
                    }
                    DecayDaemonEvent::Stopped { .. } => break,
                    _ => {}
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test timed out waiting for events");
        assert!(
            saw_success_after_failure,
            "Should have seen a successful run after a failure"
        );
        assert!(max_consecutive_seen >= 1, "Should have seen at least one failure");

        let status = handle.status().await;
        assert_eq!(
            status.consecutive_failures, 0,
            "Consecutive failures should reset to 0 after success"
        );
        assert!(status.successful_runs >= 1);
    }

    #[test]
    fn test_config_default() {
        let config = DecayDaemonConfig::default();
        assert_eq!(config.maintenance_interval, Duration::from_secs(300));
        assert!(config.run_on_startup);
        assert_eq!(config.max_consecutive_failures, 5);
        assert_eq!(config.warning_threshold, 3);
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
        assert_eq!(status.consecutive_failures, 0);
        assert!(status.last_run.is_none());
    }

    #[test]
    fn test_stop_reason_equality() {
        assert_eq!(StopReason::Requested, StopReason::Requested);
        assert_ne!(StopReason::Requested, StopReason::TooManyFailures);
    }

    #[test]
    fn test_warning_threshold_less_than_max_failures() {
        let config = DecayDaemonConfig::default();
        assert!(
            config.warning_threshold < config.max_consecutive_failures,
            "warning_threshold ({}) must be < max_consecutive_failures ({})",
            config.warning_threshold,
            config.max_consecutive_failures,
        );
    }

    #[test]
    fn test_frequent_config_warning_threshold() {
        let config = DecayDaemonConfig::frequent();
        assert_eq!(config.warning_threshold, 2);
        assert_eq!(config.max_consecutive_failures, 3);
        assert!(config.warning_threshold < config.max_consecutive_failures);
    }

    #[test]
    fn test_maintenance_failed_event_carries_consecutive_counts() {
        let event = DecayDaemonEvent::MaintenanceFailed {
            run_number: 5,
            error: "connection timeout".to_string(),
            consecutive_failures: 3,
            max_consecutive_failures: 5,
        };

        match event {
            DecayDaemonEvent::MaintenanceFailed {
                consecutive_failures,
                max_consecutive_failures,
                ..
            } => {
                assert_eq!(consecutive_failures, 3);
                assert_eq!(max_consecutive_failures, 5);
            }
            _ => panic!("Expected MaintenanceFailed"),
        }
    }

    #[test]
    fn test_failure_threshold_warning_event_structure() {
        let event = DecayDaemonEvent::FailureThresholdWarning {
            consecutive_failures: 3,
            max_consecutive_failures: 5,
            latest_error: "disk full".to_string(),
        };

        match event {
            DecayDaemonEvent::FailureThresholdWarning {
                consecutive_failures,
                max_consecutive_failures,
                latest_error,
            } => {
                assert_eq!(consecutive_failures, 3);
                assert_eq!(max_consecutive_failures, 5);
                assert_eq!(latest_error, "disk full");
            }
            _ => panic!("Expected FailureThresholdWarning"),
        }
    }

    #[test]
    fn test_daemon_status_tracks_consecutive_failures() {
        let mut status = DaemonStatus::default();
        assert_eq!(status.consecutive_failures, 0);

        status.consecutive_failures = 3;
        status.failed_runs = 3;
        assert_eq!(status.consecutive_failures, 3);

        // Reset on success
        status.consecutive_failures = 0;
        status.successful_runs = 1;
        assert_eq!(status.consecutive_failures, 0);
    }

    #[test]
    fn test_stopped_event_carries_reason() {
        let event = DecayDaemonEvent::Stopped {
            reason: StopReason::TooManyFailures,
        };
        match event {
            DecayDaemonEvent::Stopped { reason } => {
                assert_eq!(reason, StopReason::TooManyFailures);
            }
            _ => panic!("Expected Stopped"),
        }

        let event2 = DecayDaemonEvent::Stopped {
            reason: StopReason::Requested,
        };
        match event2 {
            DecayDaemonEvent::Stopped { reason } => {
                assert_eq!(reason, StopReason::Requested);
            }
            _ => panic!("Expected Stopped"),
        }
    }

    /// A mock MemoryRepository that returns Ok(3) from prune_expired()
    /// to generate MemoryPruned events via the MemoryService.
    struct EventProducingMemoryRepository;

    #[async_trait]
    impl MemoryRepository for EventProducingMemoryRepository {
        async fn store(&self, _memory: &Memory) -> DomainResult<()> { Ok(()) }
        async fn get(&self, _id: Uuid) -> DomainResult<Option<Memory>> { Ok(None) }
        async fn get_by_key(&self, _key: &str, _namespace: &str) -> DomainResult<Option<Memory>> { Ok(None) }
        async fn update(&self, _memory: &Memory) -> DomainResult<()> { Ok(()) }
        async fn delete(&self, _id: Uuid) -> DomainResult<()> { Ok(()) }
        async fn query(&self, _query: MemoryQuery) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn search(&self, _query: &str, _namespace: Option<&str>, _limit: usize) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn list_by_tier(&self, _tier: MemoryTier) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn list_by_namespace(&self, _namespace: &str) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn get_expired(&self) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn prune_expired(&self) -> DomainResult<u64> { Ok(3) }
        async fn get_decayed(&self, _threshold: f32) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn get_for_task(&self, _task_id: Uuid) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn get_for_goal(&self, _goal_id: Uuid) -> DomainResult<Vec<Memory>> { Ok(Vec::new()) }
        async fn count_by_tier(&self) -> DomainResult<std::collections::HashMap<MemoryTier, u64>> { Ok(std::collections::HashMap::new()) }
    }

    /// Helper to create a daemon backed by EventProducingMemoryRepository.
    fn make_event_producing_daemon(
        config: DecayDaemonConfig,
    ) -> MemoryDecayDaemon<EventProducingMemoryRepository> {
        let repo = Arc::new(EventProducingMemoryRepository);
        let service = Arc::new(MemoryService::new(repo));
        MemoryDecayDaemon::new(service, config)
    }

    #[tokio::test]
    async fn test_run_once_publishes_events_to_event_bus() {
        use crate::services::event_bus::{EventBus, EventBusConfig, EventPayload};

        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let mut subscriber = bus.subscribe();

        let daemon = make_event_producing_daemon(DecayDaemonConfig::default())
            .with_event_bus(bus);

        let report = daemon.run_once().await.expect("run_once should succeed");
        assert_eq!(report.expired_pruned, 3);

        // Collect events published to the bus (non-blocking drain)
        let mut pruned_events = Vec::new();
        while let Ok(event) = subscriber.try_recv() {
            if let EventPayload::MemoryPruned { count, reason } = event.payload {
                pruned_events.push((count, reason));
            }
        }

        assert!(
            !pruned_events.is_empty(),
            "Should have received at least one MemoryPruned event on the EventBus"
        );
        // The expired prune produces a MemoryPruned event with count=3 reason="expired"
        assert!(
            pruned_events.iter().any(|(c, r)| *c == 3 && r == "expired"),
            "Expected MemoryPruned {{ count: 3, reason: \"expired\" }}, got: {:?}",
            pruned_events,
        );
    }

    #[tokio::test]
    async fn test_run_once_without_event_bus_succeeds() {
        // Daemon without an EventBus should still succeed when events are generated.
        let daemon = make_event_producing_daemon(DecayDaemonConfig::default());

        let report = daemon.run_once().await.expect("run_once should succeed without EventBus");
        assert_eq!(report.expired_pruned, 3);
        assert_eq!(report.decayed_pruned, 0);
        assert_eq!(report.promoted, 0);
    }

    #[tokio::test]
    async fn test_failure_events_published_to_event_bus() {
        use crate::services::event_bus::{EventBus, EventBusConfig, EventPayload};

        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let mut subscriber = bus.subscribe();

        // Config: warning_threshold=2, max_consecutive_failures=5
        let config = DecayDaemonConfig {
            maintenance_interval: Duration::from_millis(50),
            run_on_startup: false,
            max_consecutive_failures: 5,
            warning_threshold: 2,
            verbose: false,
        };
        let (daemon, _repo) = make_daemon(true, config);
        let daemon = daemon.with_event_bus(bus);

        let handle = daemon.handle();
        let mut rx = daemon.run().await;

        // Collect events until we see FailureThresholdWarning on the mpsc channel, then stop.
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                if matches!(&event, DecayDaemonEvent::FailureThresholdWarning { .. }) {
                    handle.stop();
                    break;
                }
            }
        })
        .await;

        assert!(result.is_ok(), "Test timed out waiting for events");

        // Drain the EventBus subscriber and verify we received both event types
        let mut saw_maintenance_failed = false;
        let mut saw_daemon_degraded = false;

        while let Ok(event) = subscriber.try_recv() {
            match event.payload {
                EventPayload::MemoryMaintenanceFailed { .. } => {
                    saw_maintenance_failed = true;
                }
                EventPayload::MemoryDaemonDegraded { .. } => {
                    saw_daemon_degraded = true;
                }
                _ => {}
            }
        }

        assert!(
            saw_maintenance_failed,
            "Should have received MemoryMaintenanceFailed event on EventBus"
        );
        assert!(
            saw_daemon_degraded,
            "Should have received MemoryDaemonDegraded event on EventBus"
        );
    }
}

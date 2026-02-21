//! Safety guardrails for the swarm system.
//!
//! Enforces resource limits, safety constraints, and monitors
//! for dangerous operations.
//!
//! ## Atomicity guarantees
//!
//! Token check-and-record uses a CAS (compare-and-swap) loop on an `AtomicU64`
//! to avoid the TOCTOU race between checking the hourly limit and recording
//! usage. Task and agent check-and-register hold the write lock for the full
//! duration of the operation, preventing concurrent registrations from
//! exceeding the configured limits.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Configuration for guardrails.
#[derive(Debug, Clone)]
pub struct GuardrailsConfig {
    /// Maximum total tokens per hour.
    pub max_tokens_per_hour: u64,
    /// Maximum concurrent tasks.
    pub max_concurrent_tasks: usize,
    /// Maximum concurrent agents.
    pub max_concurrent_agents: usize,
    /// Maximum depth for goal decomposition.
    pub max_decomposition_depth: usize,
    /// Maximum retries per task.
    pub max_task_retries: u32,
    /// Maximum turns per agent invocation.
    pub max_turns_per_invocation: u32,
    /// Blocked tool patterns.
    pub blocked_tools: Vec<String>,
    /// Blocked file patterns.
    pub blocked_files: Vec<String>,
    /// Whether to enforce budget limits.
    pub enforce_budget: bool,
    /// Budget limit in cents.
    pub budget_limit_cents: f64,
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_hour: 1_000_000,
            max_concurrent_tasks: 10,
            max_concurrent_agents: 4,
            max_decomposition_depth: 3,
            max_task_retries: 3,
            max_turns_per_invocation: 50,
            blocked_tools: vec![],
            blocked_files: vec![
                ".env".to_string(),
                "*.key".to_string(),
                "*.pem".to_string(),
                "**/secrets/**".to_string(),
            ],
            enforce_budget: false,
            budget_limit_cents: 10000.0, // $100
        }
    }
}

/// Result of a guardrail check.
#[derive(Debug, Clone)]
pub enum GuardrailResult {
    /// Action is allowed.
    Allowed,
    /// Action is blocked with reason.
    Blocked(String),
    /// Action is allowed but with a warning.
    Warning(String),
}

impl GuardrailResult {
    pub fn is_allowed(&self) -> bool {
        !matches!(self, Self::Blocked(_))
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }
}

/// Runtime metrics for monitoring.
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    tokens_used_this_hour: AtomicU64,
    total_tokens_used: AtomicU64,
    tasks_started: AtomicU64,
    tasks_completed: AtomicU64,
    tasks_failed: AtomicU64,
    agents_spawned: AtomicU64,
    cost_cents: AtomicU64, // Store as integer cents * 100
}

impl RuntimeMetrics {
    pub fn record_tokens(&self, tokens: u64) {
        self.tokens_used_this_hour
            .fetch_add(tokens, Ordering::Relaxed);
        self.total_tokens_used.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Atomically check whether `tokens` can be added without exceeding `limit`,
    /// and if so, add them. Returns the new total on success, or the current
    /// total on failure (i.e. when the addition would exceed the limit).
    pub fn check_and_record_tokens(&self, tokens: u64, limit: u64) -> Result<u64, u64> {
        loop {
            let current = self.tokens_used_this_hour.load(Ordering::Relaxed);
            let new_total = current.saturating_add(tokens);
            if new_total > limit {
                return Err(current);
            }
            match self.tokens_used_this_hour.compare_exchange_weak(
                current,
                new_total,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Also bump the lifetime counter (no limit check needed).
                    self.total_tokens_used.fetch_add(tokens, Ordering::Relaxed);
                    return Ok(new_total);
                }
                Err(_) => {
                    // Another thread changed the value; retry the CAS loop.
                    continue;
                }
            }
        }
    }

    pub fn record_task_started(&self) {
        self.tasks_started.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_task_completed(&self) {
        self.tasks_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_task_failed(&self) {
        self.tasks_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_agent_spawned(&self) {
        self.agents_spawned.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cost(&self, cents: f64) {
        let int_cents = (cents * 100.0) as u64;
        self.cost_cents.fetch_add(int_cents, Ordering::Relaxed);
    }

    pub fn get_tokens_this_hour(&self) -> u64 {
        self.tokens_used_this_hour.load(Ordering::Relaxed)
    }

    pub fn get_total_tokens(&self) -> u64 {
        self.total_tokens_used.load(Ordering::Relaxed)
    }

    pub fn get_cost_cents(&self) -> f64 {
        self.cost_cents.load(Ordering::Relaxed) as f64 / 100.0
    }

    pub fn reset_hourly(&self) {
        self.tokens_used_this_hour.store(0, Ordering::Relaxed);
    }
}

/// Guardrails service for safety enforcement.
pub struct Guardrails {
    config: GuardrailsConfig,
    metrics: Arc<RuntimeMetrics>,
    current_tasks: Arc<RwLock<HashSet<uuid::Uuid>>>,
    current_agents: Arc<RwLock<HashSet<String>>>,
}

impl Guardrails {
    pub fn new(config: GuardrailsConfig) -> Self {
        Self {
            config,
            metrics: Arc::new(RuntimeMetrics::default()),
            current_tasks: Arc::new(RwLock::new(HashSet::new())),
            current_agents: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Create guardrails with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(GuardrailsConfig::default())
    }

    /// Check if we can start a new task.
    ///
    /// **Deprecated**: Use [`check_and_register_task`] instead to avoid a
    /// TOCTOU race between checking and registering.
    #[deprecated(note = "Use check_and_register_task instead to avoid TOCTOU race")]
    pub async fn check_task_start(&self, task_id: uuid::Uuid) -> GuardrailResult {
        let tasks = self.current_tasks.read().await;

        if tasks.len() >= self.config.max_concurrent_tasks {
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent tasks ({}) reached",
                self.config.max_concurrent_tasks
            ));
        }

        if tasks.contains(&task_id) {
            return GuardrailResult::Blocked("Task already running".to_string());
        }

        GuardrailResult::Allowed
    }

    /// Register a task as started.
    ///
    /// **Deprecated**: Use [`check_and_register_task`] instead to avoid a
    /// TOCTOU race between checking and registering.
    #[deprecated(note = "Use check_and_register_task instead to avoid TOCTOU race")]
    pub async fn register_task_start(&self, task_id: uuid::Uuid) {
        let mut tasks = self.current_tasks.write().await;
        tasks.insert(task_id);
        self.metrics.record_task_started();
    }

    /// Atomically check whether a task can start and register it in one step.
    ///
    /// Holds the write lock for the entire check+insert, eliminating the
    /// TOCTOU window present in separate `check_task_start` / `register_task_start`.
    pub async fn check_and_register_task(&self, task_id: uuid::Uuid) -> GuardrailResult {
        let mut tasks = self.current_tasks.write().await;

        if tasks.contains(&task_id) {
            return GuardrailResult::Blocked("Task already running".to_string());
        }

        if tasks.len() >= self.config.max_concurrent_tasks {
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent tasks ({}) reached",
                self.config.max_concurrent_tasks
            ));
        }

        tasks.insert(task_id);
        self.metrics.record_task_started();
        GuardrailResult::Allowed
    }

    /// Register a task as finished.
    pub async fn register_task_end(&self, task_id: uuid::Uuid, success: bool) {
        let mut tasks = self.current_tasks.write().await;
        tasks.remove(&task_id);

        if success {
            self.metrics.record_task_completed();
        } else {
            self.metrics.record_task_failed();
        }
    }

    /// Check if we can spawn a new agent.
    ///
    /// **Deprecated**: Use [`check_and_register_agent`] instead to avoid a
    /// TOCTOU race between checking and registering.
    #[deprecated(note = "Use check_and_register_agent instead to avoid TOCTOU race")]
    pub async fn check_agent_spawn(&self, _agent_id: &str) -> GuardrailResult {
        let agents = self.current_agents.read().await;

        if agents.len() >= self.config.max_concurrent_agents {
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent agents ({}) reached",
                self.config.max_concurrent_agents
            ));
        }

        GuardrailResult::Allowed
    }

    /// Register an agent as spawned.
    ///
    /// **Deprecated**: Use [`check_and_register_agent`] instead to avoid a
    /// TOCTOU race between checking and registering.
    #[deprecated(note = "Use check_and_register_agent instead to avoid TOCTOU race")]
    pub async fn register_agent_spawn(&self, agent_id: &str) {
        let mut agents = self.current_agents.write().await;
        agents.insert(agent_id.to_string());
        self.metrics.record_agent_spawned();
    }

    /// Atomically check whether an agent can spawn and register it in one step.
    ///
    /// Holds the write lock for the entire check+insert, eliminating the
    /// TOCTOU window present in separate `check_agent_spawn` / `register_agent_spawn`.
    pub async fn check_and_register_agent(&self, agent_id: &str) -> GuardrailResult {
        let mut agents = self.current_agents.write().await;

        if agents.contains(agent_id) {
            return GuardrailResult::Blocked(format!("Agent '{}' already running", agent_id));
        }

        if agents.len() >= self.config.max_concurrent_agents {
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent agents ({}) reached",
                self.config.max_concurrent_agents
            ));
        }

        agents.insert(agent_id.to_string());
        self.metrics.record_agent_spawned();
        GuardrailResult::Allowed
    }

    /// Register an agent as finished.
    pub async fn register_agent_end(&self, agent_id: &str) {
        let mut agents = self.current_agents.write().await;
        agents.remove(agent_id);
    }

    /// Check if a tool is allowed.
    pub fn check_tool(&self, tool_name: &str) -> GuardrailResult {
        for blocked in &self.config.blocked_tools {
            if tool_name.eq_ignore_ascii_case(blocked) {
                return GuardrailResult::Blocked(format!("Tool '{}' is blocked", tool_name));
            }
        }
        GuardrailResult::Allowed
    }

    /// Check if a file path is allowed.
    pub fn check_file_path(&self, path: &str) -> GuardrailResult {
        for pattern in &self.config.blocked_files {
            if Self::matches_pattern(path, pattern) {
                return GuardrailResult::Blocked(format!(
                    "Access to '{}' is blocked by pattern '{}'",
                    path, pattern
                ));
            }
        }
        GuardrailResult::Allowed
    }

    /// Check token usage without recording.
    ///
    /// **Deprecated**: Use [`check_and_record_tokens`] instead to avoid a
    /// TOCTOU race between checking and recording.
    #[deprecated(note = "Use check_and_record_tokens instead to avoid TOCTOU race")]
    pub fn check_tokens(&self, requested: u64) -> GuardrailResult {
        let current = self.metrics.get_tokens_this_hour();
        if current + requested > self.config.max_tokens_per_hour {
            return GuardrailResult::Blocked(format!(
                "Token limit ({}/hour) would be exceeded",
                self.config.max_tokens_per_hour
            ));
        }

        // Warn at 80%
        if current + requested > (self.config.max_tokens_per_hour * 80) / 100 {
            return GuardrailResult::Warning(format!(
                "Approaching token limit: {}/{} used",
                current + requested,
                self.config.max_tokens_per_hour
            ));
        }

        GuardrailResult::Allowed
    }

    /// Atomically check and record token usage.
    ///
    /// Uses a CAS loop internally so that the hourly limit cannot be exceeded
    /// even under concurrent access. Returns `Blocked` if the addition would
    /// exceed the limit, `Warning` if the new total is above 80% of the limit,
    /// or `Allowed` otherwise.
    pub fn check_and_record_tokens(&self, requested: u64) -> GuardrailResult {
        let limit = self.config.max_tokens_per_hour;
        match self.metrics.check_and_record_tokens(requested, limit) {
            Ok(new_total) => {
                // Warn at 80%
                let threshold = (limit * 80) / 100;
                if new_total > threshold {
                    GuardrailResult::Warning(format!(
                        "Approaching token limit: {}/{} used",
                        new_total, limit
                    ))
                } else {
                    GuardrailResult::Allowed
                }
            }
            Err(_current) => GuardrailResult::Blocked(format!(
                "Token limit ({}/hour) would be exceeded",
                limit
            )),
        }
    }

    /// Check budget.
    pub fn check_budget(&self, additional_cents: f64) -> GuardrailResult {
        if !self.config.enforce_budget {
            return GuardrailResult::Allowed;
        }

        let current = self.metrics.get_cost_cents();
        if current + additional_cents > self.config.budget_limit_cents {
            return GuardrailResult::Blocked(format!(
                "Budget limit (${:.2}) would be exceeded",
                self.config.budget_limit_cents / 100.0
            ));
        }

        GuardrailResult::Allowed
    }

    /// Check decomposition depth.
    pub fn check_decomposition_depth(&self, current_depth: usize) -> GuardrailResult {
        if current_depth >= self.config.max_decomposition_depth {
            return GuardrailResult::Blocked(format!(
                "Maximum decomposition depth ({}) reached",
                self.config.max_decomposition_depth
            ));
        }
        GuardrailResult::Allowed
    }

    /// Record token usage.
    ///
    /// **Deprecated**: Use [`check_and_record_tokens`] instead for atomic
    /// check+record.
    #[deprecated(note = "Use check_and_record_tokens instead for atomic check+record")]
    pub fn record_tokens(&self, tokens: u64) {
        self.metrics.record_tokens(tokens);
    }

    /// Record cost.
    pub fn record_cost(&self, cents: f64) {
        self.metrics.record_cost(cents);
    }

    /// Get current metrics.
    pub fn get_metrics(&self) -> &RuntimeMetrics {
        &self.metrics
    }

    /// Get a reference to the shared metrics `Arc`, e.g. for the reset daemon.
    pub fn metrics_arc(&self) -> Arc<RuntimeMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Simple pattern matching for file paths.
    fn matches_pattern(path: &str, pattern: &str) -> bool {
        if let Some(suffix) = pattern.strip_prefix("**/") {
            // Match anywhere in path
            path.contains(suffix.trim_start_matches('*'))
        } else if pattern.starts_with("*.") {
            // Extension match
            path.ends_with(&pattern[1..])
        } else {
            // Exact match or contains
            path == pattern || path.ends_with(pattern)
        }
    }
}

// ---------------------------------------------------------------------------
// Hourly token reset daemon
// ---------------------------------------------------------------------------

/// Configuration for the hourly token reset daemon.
#[derive(Debug, Clone)]
pub struct HourlyResetConfig {
    /// How often to reset the hourly counter. Defaults to 1 hour.
    pub reset_interval: Duration,
}

impl Default for HourlyResetConfig {
    fn default() -> Self {
        Self {
            reset_interval: Duration::from_secs(3600),
        }
    }
}

impl HourlyResetConfig {
    /// Create a config with a custom interval (useful for tests).
    pub fn with_interval(interval: Duration) -> Self {
        Self {
            reset_interval: interval,
        }
    }
}

/// Handle to control the hourly reset daemon.
pub struct HourlyResetHandle {
    stop_flag: Arc<AtomicBool>,
}

impl HourlyResetHandle {
    /// Signal the daemon to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
    }

    /// Check whether a stop has been requested.
    pub fn is_stop_requested(&self) -> bool {
        self.stop_flag.load(Ordering::Acquire)
    }
}

/// Background daemon that periodically resets the hourly token counter.
///
/// Modelled after [`MemoryDecayDaemon`](super::memory_decay_daemon::MemoryDecayDaemon):
/// it uses a `tokio::time::interval` tick and an `AtomicBool` stop flag.
pub struct HourlyResetDaemon {
    metrics: Arc<RuntimeMetrics>,
    config: HourlyResetConfig,
    stop_flag: Arc<AtomicBool>,
}

impl HourlyResetDaemon {
    /// Create a new daemon that will reset the given metrics.
    pub fn new(metrics: Arc<RuntimeMetrics>, config: HourlyResetConfig) -> Self {
        Self {
            metrics,
            config,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get a handle to stop the daemon.
    pub fn handle(&self) -> HourlyResetHandle {
        HourlyResetHandle {
            stop_flag: Arc::clone(&self.stop_flag),
        }
    }

    /// Spawn the daemon onto the Tokio runtime. Returns a [`HourlyResetHandle`]
    /// that can be used to stop it.
    pub fn spawn(self) -> HourlyResetHandle {
        let handle = self.handle();
        tokio::spawn(self.run_loop());
        handle
    }

    /// Main loop: tick at the configured interval and reset the counter.
    async fn run_loop(self) {
        let mut interval_timer = tokio::time::interval(self.config.reset_interval);
        // The first tick completes immediately; consume it so we don't reset
        // right away.
        interval_timer.tick().await;

        loop {
            interval_timer.tick().await;

            if self.stop_flag.load(Ordering::Acquire) {
                tracing::info!("Hourly token reset daemon stopping (requested)");
                break;
            }

            let previous = self.metrics.get_tokens_this_hour();
            self.metrics.reset_hourly();
            tracing::debug!(
                previous_tokens = previous,
                "Hourly token counter reset"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(deprecated)]
    async fn test_task_limit() {
        let config = GuardrailsConfig {
            max_concurrent_tasks: 2,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let id3 = uuid::Uuid::new_v4();

        assert!(guardrails.check_task_start(id1).await.is_allowed());
        guardrails.register_task_start(id1).await;

        assert!(guardrails.check_task_start(id2).await.is_allowed());
        guardrails.register_task_start(id2).await;

        // Third task should be blocked
        assert!(guardrails.check_task_start(id3).await.is_blocked());

        // Free up a slot
        guardrails.register_task_end(id1, true).await;
        assert!(guardrails.check_task_start(id3).await.is_allowed());
    }

    #[test]
    fn test_tool_blocking() {
        let config = GuardrailsConfig {
            blocked_tools: vec!["rm".to_string(), "sudo".to_string()],
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_tool("read").is_allowed());
        assert!(guardrails.check_tool("rm").is_blocked());
        assert!(guardrails.check_tool("sudo").is_blocked());
    }

    #[test]
    fn test_file_blocking() {
        let config = GuardrailsConfig::default();
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_file_path("src/main.rs").is_allowed());
        assert!(guardrails.check_file_path(".env").is_blocked());
        assert!(guardrails.check_file_path("config/secrets/api.key").is_blocked());
    }

    #[test]
    #[allow(deprecated)]
    fn test_token_limit() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_tokens(500).is_allowed());
        guardrails.record_tokens(800);

        // Should warn at 80%+
        match guardrails.check_tokens(50) {
            GuardrailResult::Warning(_) => {}
            other => panic!("Expected Warning, got {:?}", other),
        }

        // Should block when exceeding
        assert!(guardrails.check_tokens(300).is_blocked());
    }

    // -----------------------------------------------------------------------
    // New tests for atomic operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_atomic_check_and_record_tokens_allowed() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Well below limit => Allowed
        let result = guardrails.check_and_record_tokens(100);
        assert!(result.is_allowed());
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 100);
        assert_eq!(guardrails.get_metrics().get_total_tokens(), 100);
    }

    #[test]
    fn test_atomic_check_and_record_tokens_warning() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Push past 80% threshold (801 > 800)
        let result = guardrails.check_and_record_tokens(801);
        match result {
            GuardrailResult::Warning(_) => {}
            other => panic!("Expected Warning, got {:?}", other),
        }
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 801);
    }

    #[test]
    #[allow(deprecated)]
    fn test_atomic_check_and_record_tokens_blocked() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Record some tokens first
        guardrails.record_tokens(900);

        // This would exceed the limit => Blocked, counter unchanged
        let result = guardrails.check_and_record_tokens(200);
        assert!(result.is_blocked());
        // Counter stays at 900 — not 1100
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 900);
    }

    #[test]
    fn test_atomic_check_and_record_tokens_exact_limit() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Record exactly up to limit: should be Warning (above 80%)
        let result = guardrails.check_and_record_tokens(1000);
        match result {
            GuardrailResult::Warning(_) => {}
            other => panic!("Expected Warning at exact limit, got {:?}", other),
        }
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 1000);

        // Now any more should be blocked
        let result = guardrails.check_and_record_tokens(1);
        assert!(result.is_blocked());
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 1000);
    }

    #[test]
    fn test_runtime_metrics_check_and_record_cas() {
        let metrics = RuntimeMetrics::default();

        // Successful CAS
        assert_eq!(metrics.check_and_record_tokens(500, 1000), Ok(500));
        assert_eq!(metrics.get_tokens_this_hour(), 500);
        assert_eq!(metrics.get_total_tokens(), 500);

        // Another successful CAS
        assert_eq!(metrics.check_and_record_tokens(400, 1000), Ok(900));
        assert_eq!(metrics.get_tokens_this_hour(), 900);

        // Would exceed limit — Err returns current value
        assert_eq!(metrics.check_and_record_tokens(200, 1000), Err(900));
        // Counter unchanged
        assert_eq!(metrics.get_tokens_this_hour(), 900);
    }

    #[tokio::test]
    async fn test_check_and_register_task_atomic() {
        let config = GuardrailsConfig {
            max_concurrent_tasks: 2,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let id3 = uuid::Uuid::new_v4();

        // First two should succeed
        assert!(guardrails.check_and_register_task(id1).await.is_allowed());
        assert!(guardrails.check_and_register_task(id2).await.is_allowed());

        // Third should be blocked — limit reached
        assert!(guardrails.check_and_register_task(id3).await.is_blocked());

        // Duplicate should be blocked
        assert!(guardrails.check_and_register_task(id1).await.is_blocked());

        // Free a slot
        guardrails.register_task_end(id1, true).await;
        assert!(guardrails.check_and_register_task(id3).await.is_allowed());
    }

    #[tokio::test]
    async fn test_check_and_register_agent_atomic() {
        let config = GuardrailsConfig {
            max_concurrent_agents: 2,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_and_register_agent("a1").await.is_allowed());
        assert!(guardrails.check_and_register_agent("a2").await.is_allowed());

        // Limit reached
        assert!(guardrails.check_and_register_agent("a3").await.is_blocked());

        // Duplicate
        assert!(guardrails.check_and_register_agent("a1").await.is_blocked());

        // Free a slot
        guardrails.register_agent_end("a1").await;
        assert!(guardrails.check_and_register_agent("a3").await.is_allowed());
    }

    #[tokio::test]
    async fn test_hourly_reset_daemon() {
        let metrics = Arc::new(RuntimeMetrics::default());
        metrics.record_tokens(5000);
        assert_eq!(metrics.get_tokens_this_hour(), 5000);

        // Use a tiny interval so the test finishes quickly.
        let config = HourlyResetConfig::with_interval(Duration::from_millis(50));
        let daemon = HourlyResetDaemon::new(Arc::clone(&metrics), config);
        let handle = daemon.spawn();

        // Wait long enough for at least one reset tick.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(metrics.get_tokens_this_hour(), 0);
        // Total tokens are not reset
        assert_eq!(metrics.get_total_tokens(), 5000);

        handle.stop();
        // Give the daemon a moment to exit.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[test]
    fn test_hourly_reset_config_default() {
        let config = HourlyResetConfig::default();
        assert_eq!(config.reset_interval, Duration::from_secs(3600));
    }

    #[test]
    fn test_hourly_reset_config_custom() {
        let config = HourlyResetConfig::with_interval(Duration::from_secs(60));
        assert_eq!(config.reset_interval, Duration::from_secs(60));
    }
}

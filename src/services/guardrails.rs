//! Safety guardrails for the swarm system.
//!
//! Enforces resource limits, safety constraints, and monitors
//! for dangerous operations.
//!
//! ## Cost tracking
//!
//! Costs are stored internally as **hundredths of a cent** (i.e. cents × 100)
//! using an `AtomicU64`.  Public API surfaces continue to accept and return
//! values denominated in *cents* (`f64`), so callers are unaffected.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::supervise_with_handle;

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
    /// Accumulated cost stored as hundredths-of-a-cent (i.e. `cents * 100`).
    cost_hundredths: AtomicU64,
}

impl RuntimeMetrics {
    pub fn record_tokens(&self, tokens: u64) {
        self.tokens_used_this_hour
            .fetch_add(tokens, Ordering::Relaxed);
        self.total_tokens_used.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Atomically check whether `requested` tokens fit within `max_tokens_per_hour`
    /// and, if so, reserve them in a single CAS operation.
    ///
    /// Returns `Ok(previous_value)` when the reservation succeeded, or
    /// `Err(current_value)` when the limit would be exceeded.
    pub fn check_and_record_tokens(
        &self,
        requested: u64,
        max_tokens_per_hour: u64,
    ) -> Result<u64, u64> {
        let result = self.tokens_used_this_hour.fetch_update(
            Ordering::SeqCst,
            Ordering::SeqCst,
            |current| {
                if current.checked_add(requested)? <= max_tokens_per_hour {
                    Some(current + requested)
                } else {
                    None
                }
            },
        );
        if result.is_ok() {
            // Also bump the lifetime counter (best-effort, no CAS needed).
            self.total_tokens_used
                .fetch_add(requested, Ordering::Relaxed);
        }
        result
    }

    /// Atomically check whether `additional_hundredths` fits within
    /// `limit_hundredths` and, if so, reserve it in a single CAS operation.
    ///
    /// Returns `Ok(previous_value)` on success, `Err(current_value)` on
    /// budget exhaustion.
    pub fn check_and_record_cost_hundredths(
        &self,
        additional_hundredths: u64,
        limit_hundredths: u64,
    ) -> Result<u64, u64> {
        self.cost_hundredths
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current.checked_add(additional_hundredths)? <= limit_hundredths {
                    Some(current + additional_hundredths)
                } else {
                    None
                }
            })
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
        debug_assert!(cents >= 0.0, "cost must be non-negative, got {cents}");
        if cents < 0.0 || cents.is_nan() {
            // In release builds, silently ignore bad values rather than
            // corrupting the accumulator.
            return;
        }

        let hundredths = (cents * 100.0).round() as u64;

        // Use a CAS loop to detect overflow instead of wrapping silently.
        loop {
            let current = self.cost_hundredths.load(Ordering::Relaxed);
            match current.checked_add(hundredths) {
                Some(new_val) => {
                    if self
                        .cost_hundredths
                        .compare_exchange_weak(
                            current,
                            new_val,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                    {
                        return;
                    }
                    // CAS failed — another thread updated concurrently; retry.
                }
                None => {
                    // Overflow would occur — saturate at u64::MAX and warn.
                    tracing::warn!(
                        current_hundredths = current,
                        additional_hundredths = hundredths,
                        "cost accumulator overflow detected — saturating at u64::MAX"
                    );
                    self.cost_hundredths.store(u64::MAX, Ordering::Relaxed);
                    return;
                }
            }
        }
    }

    pub fn get_tokens_this_hour(&self) -> u64 {
        self.tokens_used_this_hour.load(Ordering::Relaxed)
    }

    pub fn get_total_tokens(&self) -> u64 {
        self.total_tokens_used.load(Ordering::Relaxed)
    }

    pub fn get_cost_cents(&self) -> f64 {
        self.cost_hundredths.load(Ordering::Relaxed) as f64 / 100.0
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

    /// Check if a new task can be created (pre-flight check at the API boundary).
    ///
    /// Unlike [`check_task_start`], this does not require a task ID because the
    /// task has not been created yet. It only validates that the concurrent-task
    /// limit has not been reached.
    pub async fn check_task_creation(&self) -> GuardrailResult {
        let tasks = self.current_tasks.read().await;

        if tasks.len() >= self.config.max_concurrent_tasks {
            tracing::warn!(
                current_count = tasks.len(),
                max = self.config.max_concurrent_tasks,
                "task creation blocked: max concurrent tasks reached"
            );
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent tasks ({}) reached",
                self.config.max_concurrent_tasks
            ));
        }

        // Warn when approaching the limit (80%+)
        let threshold = (self.config.max_concurrent_tasks * 80) / 100;
        if threshold > 0 && tasks.len() >= threshold {
            return GuardrailResult::Warning(format!(
                "Approaching concurrent task limit: {}/{}",
                tasks.len(),
                self.config.max_concurrent_tasks
            ));
        }

        GuardrailResult::Allowed
    }

    /// Check if we can start a new task.
    pub async fn check_task_start(&self, task_id: uuid::Uuid) -> GuardrailResult {
        let tasks = self.current_tasks.read().await;

        if tasks.len() >= self.config.max_concurrent_tasks {
            tracing::warn!(%task_id, current_count = tasks.len(), max = self.config.max_concurrent_tasks, "task start blocked: max concurrent tasks reached");
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent tasks ({}) reached",
                self.config.max_concurrent_tasks
            ));
        }

        if tasks.contains(&task_id) {
            tracing::warn!(%task_id, "task start blocked: task already running");
            return GuardrailResult::Blocked("Task already running".to_string());
        }

        GuardrailResult::Allowed
    }

    /// Register a task as started.
    pub async fn register_task_start(&self, task_id: uuid::Uuid) {
        let mut tasks = self.current_tasks.write().await;
        tasks.insert(task_id);
        self.metrics.record_task_started();
        tracing::info!(%task_id, current_count = tasks.len(), "task registered as started");
    }

    /// Register a task as finished.
    pub async fn register_task_end(&self, task_id: uuid::Uuid, success: bool) {
        let mut tasks = self.current_tasks.write().await;
        tasks.remove(&task_id);
        let remaining = tasks.len();

        if success {
            self.metrics.record_task_completed();
        } else {
            self.metrics.record_task_failed();
        }
        tracing::info!(%task_id, success, remaining_count = remaining, "task registered as ended");
    }

    /// Check if we can spawn a new agent.
    pub async fn check_agent_spawn(&self, agent_id: &str) -> GuardrailResult {
        let agents = self.current_agents.read().await;

        if agents.contains(agent_id) {
            tracing::warn!(agent_id, "agent spawn blocked: agent already running");
            return GuardrailResult::Blocked(format!("Agent '{}' is already running", agent_id));
        }

        if agents.len() >= self.config.max_concurrent_agents {
            tracing::warn!(
                agent_id,
                current_count = agents.len(),
                max = self.config.max_concurrent_agents,
                "agent spawn blocked: max concurrent agents reached"
            );
            return GuardrailResult::Blocked(format!(
                "Maximum concurrent agents ({}) reached",
                self.config.max_concurrent_agents
            ));
        }

        GuardrailResult::Allowed
    }

    /// Register an agent as spawned.
    pub async fn register_agent_spawn(&self, agent_id: &str) {
        let mut agents = self.current_agents.write().await;
        agents.insert(agent_id.to_string());
        self.metrics.record_agent_spawned();
        tracing::info!(
            agent_id,
            current_count = agents.len(),
            "agent registered as spawned"
        );
    }

    /// Register an agent as finished.
    pub async fn register_agent_end(&self, agent_id: &str) {
        let mut agents = self.current_agents.write().await;
        agents.remove(agent_id);
        tracing::info!(
            agent_id,
            remaining_count = agents.len(),
            "agent registered as ended"
        );
    }

    /// Check if a tool is allowed.
    pub fn check_tool(&self, tool_name: &str) -> GuardrailResult {
        for blocked in &self.config.blocked_tools {
            if tool_name.eq_ignore_ascii_case(blocked) {
                tracing::warn!(tool_name, blocked_pattern = %blocked, "tool usage blocked");
                return GuardrailResult::Blocked(format!("Tool '{}' is blocked", tool_name));
            }
        }
        GuardrailResult::Allowed
    }

    /// Check if a file path is allowed.
    pub fn check_file_path(&self, path: &str) -> GuardrailResult {
        for pattern in &self.config.blocked_files {
            if Self::matches_pattern(path, pattern) {
                tracing::warn!(path, blocked_pattern = %pattern, "file access blocked");
                return GuardrailResult::Blocked(format!(
                    "Access to '{}' is blocked by pattern '{}'",
                    path, pattern
                ));
            }
        }
        GuardrailResult::Allowed
    }

    /// Check token usage (read-only — does NOT reserve tokens).
    ///
    /// Prefer [`check_and_record_tokens`] for callers that intend to
    /// consume the tokens immediately after the check, since that method
    /// performs the check and reservation atomically.
    pub fn check_tokens(&self, requested: u64) -> GuardrailResult {
        let current = self.metrics.get_tokens_this_hour();
        if current + requested > self.config.max_tokens_per_hour {
            tracing::warn!(
                requested,
                current,
                max = self.config.max_tokens_per_hour,
                "token usage blocked: limit would be exceeded"
            );
            return GuardrailResult::Blocked(format!(
                "Token limit ({}/hour) would be exceeded",
                self.config.max_tokens_per_hour
            ));
        }

        // Warn at 80%
        if current + requested > (self.config.max_tokens_per_hour * 80) / 100 {
            tracing::warn!(
                requested,
                current = current + requested,
                max = self.config.max_tokens_per_hour,
                "token usage warning: approaching limit"
            );
            return GuardrailResult::Warning(format!(
                "Approaching token limit: {}/{} used",
                current + requested,
                self.config.max_tokens_per_hour
            ));
        }

        GuardrailResult::Allowed
    }

    /// Atomically check **and** reserve `requested` tokens against the
    /// hourly limit.
    ///
    /// This combines the read + write into a single lock-free CAS loop so
    /// concurrent callers cannot collectively exceed the quota.
    pub fn check_and_record_tokens(&self, requested: u64) -> GuardrailResult {
        match self
            .metrics
            .check_and_record_tokens(requested, self.config.max_tokens_per_hour)
        {
            Ok(prev) => {
                let new_total = prev + requested;
                tracing::debug!(requested, new_total, "atomically reserved tokens");

                // Emit a warning if we crossed the 80 % threshold.
                let warn_threshold = (self.config.max_tokens_per_hour * 80) / 100;
                if new_total > warn_threshold {
                    tracing::warn!(
                        requested,
                        current = new_total,
                        max = self.config.max_tokens_per_hour,
                        "token usage warning: approaching limit"
                    );
                    return GuardrailResult::Warning(format!(
                        "Approaching token limit: {}/{} used",
                        new_total, self.config.max_tokens_per_hour
                    ));
                }

                GuardrailResult::Allowed
            }
            Err(current) => {
                tracing::warn!(
                    requested,
                    current,
                    max = self.config.max_tokens_per_hour,
                    "token usage blocked: limit would be exceeded"
                );
                GuardrailResult::Blocked(format!(
                    "Token limit ({}/hour) would be exceeded",
                    self.config.max_tokens_per_hour
                ))
            }
        }
    }

    /// Check budget (read-only — does NOT reserve spend).
    ///
    /// Prefer [`check_and_record_cost`] for callers that intend to record
    /// the cost immediately after the check.
    pub fn check_budget(&self, additional_cents: f64) -> GuardrailResult {
        if !self.config.enforce_budget {
            return GuardrailResult::Allowed;
        }

        let current = self.metrics.get_cost_cents();
        if current + additional_cents > self.config.budget_limit_cents {
            tracing::warn!(
                additional_cents,
                current_cents = current,
                limit_cents = self.config.budget_limit_cents,
                "budget blocked: limit would be exceeded"
            );
            return GuardrailResult::Blocked(format!(
                "Budget limit (${:.2}) would be exceeded",
                self.config.budget_limit_cents / 100.0
            ));
        }

        GuardrailResult::Allowed
    }

    /// Atomically check **and** reserve `additional_cents` against the
    /// budget limit.
    ///
    /// Returns [`GuardrailResult::Allowed`] when budget is not enforced.
    pub fn check_and_record_cost(&self, additional_cents: f64) -> GuardrailResult {
        if !self.config.enforce_budget {
            // Still record the cost for observability, even though we don't enforce.
            self.metrics.record_cost(additional_cents);
            return GuardrailResult::Allowed;
        }

        if additional_cents < 0.0 || additional_cents.is_nan() {
            return GuardrailResult::Allowed;
        }

        let additional_hundredths = (additional_cents * 100.0).round() as u64;
        let limit_hundredths = (self.config.budget_limit_cents * 100.0).round() as u64;

        match self
            .metrics
            .check_and_record_cost_hundredths(additional_hundredths, limit_hundredths)
        {
            Ok(_prev) => {
                tracing::debug!(additional_cents, "atomically reserved budget");
                GuardrailResult::Allowed
            }
            Err(current_hundredths) => {
                let current_cents = current_hundredths as f64 / 100.0;
                tracing::warn!(
                    additional_cents,
                    current_cents,
                    limit_cents = self.config.budget_limit_cents,
                    "budget blocked: limit would be exceeded"
                );
                GuardrailResult::Blocked(format!(
                    "Budget limit (${:.2}) would be exceeded",
                    self.config.budget_limit_cents / 100.0
                ))
            }
        }
    }

    /// Check decomposition depth.
    pub fn check_decomposition_depth(&self, current_depth: usize) -> GuardrailResult {
        if current_depth >= self.config.max_decomposition_depth {
            tracing::warn!(
                current_depth,
                max_depth = self.config.max_decomposition_depth,
                "decomposition blocked: max depth reached"
            );
            return GuardrailResult::Blocked(format!(
                "Maximum decomposition depth ({}) reached",
                self.config.max_decomposition_depth
            ));
        }
        GuardrailResult::Allowed
    }

    /// Record token usage.
    pub fn record_tokens(&self, tokens: u64) {
        self.metrics.record_tokens(tokens);
        tracing::debug!(tokens, "recorded token usage");
    }

    /// Record cost.
    pub fn record_cost(&self, cents: f64) {
        self.metrics.record_cost(cents);
        tracing::debug!(cents, "recorded cost");
    }

    /// Get current metrics.
    pub fn get_metrics(&self) -> &RuntimeMetrics {
        &self.metrics
    }

    /// Get a clone of the shared metrics Arc.
    pub fn metrics_arc(&self) -> Arc<RuntimeMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Spawn a background task that resets the hourly token counter every 60 minutes.
    ///
    /// The task runs until the provided `cancel` token is cancelled, enabling
    /// graceful shutdown.  Returns a `JoinHandle` so the caller can await
    /// completion if desired.
    pub fn spawn_hourly_reset(&self, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        let metrics = Arc::clone(&self.metrics);
        supervise_with_handle("guardrails_hourly_reset", async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            // The first tick completes immediately — consume it so we don't
            // reset at startup.
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!("hourly token reset task shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        metrics.reset_hourly();
                        tracing::info!("hourly token counter reset");
                    }
                }
            }
        })
    }

    /// Simple glob-style pattern matching for file paths.
    ///
    /// Supported patterns:
    /// - `**/dir/**` — matches if `dir` appears as a path segment anywhere
    /// - `**/name`   — matches if the last path segment equals `name`
    /// - `*.ext`     — matches if the file ends with `.ext`
    /// - `literal`   — exact match on the filename component (last segment)
    fn matches_pattern(path: &str, pattern: &str) -> bool {
        if let Some(suffix) = pattern.strip_prefix("**/") {
            if let Some(inner) = suffix.strip_suffix("/**") {
                // Pattern: **/dir/** — match if `dir` appears as a complete path segment
                path.split('/').any(|seg| seg == inner)
            } else if suffix.contains('/') {
                // Pattern: **/a/b — match if path ends with /a/b or equals a/b
                path == suffix || path.ends_with(&format!("/{suffix}"))
            } else {
                // Pattern: **/name — match if the filename equals `name`
                Self::filename(path) == suffix
            }
        } else if let Some(ext) = pattern.strip_prefix("*.") {
            // Extension match: *.key matches any file ending in .key
            path.ends_with(&format!(".{ext}"))
        } else {
            // Exact filename match: ".env" matches only ".env" as the filename
            // component, not "production.env"
            Self::filename(path) == pattern
        }
    }

    /// Extract the filename component (last path segment) from a path.
    fn filename(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reset_hourly_clears_token_counter() {
        let metrics = RuntimeMetrics::default();
        metrics.record_tokens(5000);
        assert_eq!(metrics.get_tokens_this_hour(), 5000);

        metrics.reset_hourly();
        assert_eq!(metrics.get_tokens_this_hour(), 0);

        // Total tokens should remain unchanged
        assert_eq!(metrics.get_total_tokens(), 5000);
    }

    #[tokio::test]
    async fn test_spawn_hourly_reset_responds_to_cancellation() {
        let guardrails = Guardrails::new(GuardrailsConfig::default());
        let cancel = CancellationToken::new();
        let handle = guardrails.spawn_hourly_reset(cancel.clone());

        // The task should be running
        assert!(!handle.is_finished());

        // Cancel and verify graceful shutdown
        cancel.cancel();
        // The task should finish promptly after cancellation
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("hourly reset task did not shut down in time")
            .expect("hourly reset task panicked");
    }

    #[tokio::test]
    async fn test_spawn_hourly_reset_resets_counter() {
        // Use a short interval by directly testing the metrics reset mechanism
        // (we can't easily wait 60 minutes in a test)
        let guardrails = Guardrails::new(GuardrailsConfig {
            max_tokens_per_hour: 10_000,
            ..Default::default()
        });

        // Record some tokens
        guardrails.record_tokens(5000);
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 5000);

        // Simulate what the background task does
        guardrails.get_metrics().reset_hourly();
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 0);

        // Token check should now pass again
        assert!(guardrails.check_tokens(5000).is_allowed());
    }

    #[tokio::test]
    async fn test_metrics_arc_returns_shared_reference() {
        let guardrails = Guardrails::new(GuardrailsConfig::default());
        let metrics = guardrails.metrics_arc();

        guardrails.record_tokens(1000);
        assert_eq!(metrics.get_tokens_this_hour(), 1000);

        metrics.reset_hourly();
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 0);
    }

    #[tokio::test]
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
        assert!(
            guardrails
                .check_file_path("config/secrets/api.key")
                .is_blocked()
        );
    }

    #[test]
    fn test_double_star_secrets_pattern() {
        // **/secrets/** should match paths containing "secrets" as a directory segment
        let config = GuardrailsConfig {
            blocked_files: vec!["**/secrets/**".to_string()],
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Should block
        assert!(
            guardrails
                .check_file_path("config/secrets/api.key")
                .is_blocked()
        );
        assert!(
            guardrails
                .check_file_path("secrets/password.txt")
                .is_blocked()
        );
        assert!(
            guardrails
                .check_file_path("a/b/secrets/c/d.txt")
                .is_blocked()
        );

        // Should allow — "secrets" is not a standalone segment
        assert!(
            guardrails
                .check_file_path("my-secrets-file.txt")
                .is_allowed()
        );
        assert!(
            guardrails
                .check_file_path("nosecrets/file.txt")
                .is_allowed()
        );
    }

    #[test]
    fn test_env_exact_match() {
        // ".env" should match only the exact filename, not partial matches
        let config = GuardrailsConfig {
            blocked_files: vec![".env".to_string()],
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Should block
        assert!(guardrails.check_file_path(".env").is_blocked());
        assert!(guardrails.check_file_path("config/.env").is_blocked());

        // Should allow — these are different filenames
        assert!(guardrails.check_file_path("production.env").is_allowed());
        assert!(guardrails.check_file_path("some.env").is_allowed());
        assert!(guardrails.check_file_path(".env.local").is_allowed());
    }

    #[test]
    fn test_extension_pattern() {
        let config = GuardrailsConfig {
            blocked_files: vec!["*.key".to_string(), "*.pem".to_string()],
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_file_path("server.key").is_blocked());
        assert!(guardrails.check_file_path("path/to/cert.pem").is_blocked());
        assert!(guardrails.check_file_path("not-a-key.txt").is_allowed());
    }

    #[test]
    fn test_double_star_filename_pattern() {
        // **/name should match the filename in any directory
        let config = GuardrailsConfig {
            blocked_files: vec!["**/.gitignore".to_string()],
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_file_path(".gitignore").is_blocked());
        assert!(
            guardrails
                .check_file_path("sub/dir/.gitignore")
                .is_blocked()
        );
        assert!(guardrails.check_file_path("not-gitignore").is_allowed());
    }

    #[test]
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

    #[tokio::test]
    async fn test_agent_limit() {
        let config = GuardrailsConfig {
            max_concurrent_agents: 2,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_agent_spawn("agent-1").await.is_allowed());
        guardrails.register_agent_spawn("agent-1").await;

        assert!(guardrails.check_agent_spawn("agent-2").await.is_allowed());
        guardrails.register_agent_spawn("agent-2").await;

        // Third agent should be blocked — at capacity
        assert!(guardrails.check_agent_spawn("agent-3").await.is_blocked());

        // Free up a slot
        guardrails.register_agent_end("agent-1").await;
        assert!(guardrails.check_agent_spawn("agent-3").await.is_allowed());
    }

    #[tokio::test]
    async fn test_agent_tracking_with_duplicate_template_names() {
        // THE KEY BUG FIX: Two agents using the same template name ("implementer")
        // must be tracked independently by unique task IDs, not template names.
        let config = GuardrailsConfig {
            max_concurrent_agents: 3,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Simulate two tasks both routed to the "implementer" template,
        // but tracked by unique task_id strings (as spawn_task_agent does).
        let task_id_a = uuid::Uuid::new_v4().to_string();
        let task_id_b = uuid::Uuid::new_v4().to_string();

        assert!(guardrails.check_agent_spawn(&task_id_a).await.is_allowed());
        guardrails.register_agent_spawn(&task_id_a).await;

        // Second agent with a DIFFERENT unique ID must also be allowed
        assert!(guardrails.check_agent_spawn(&task_id_b).await.is_allowed());
        guardrails.register_agent_spawn(&task_id_b).await;

        // Both agents tracked independently — count is 2
        {
            let agents = guardrails.current_agents.read().await;
            assert_eq!(agents.len(), 2);
        }

        // Ending one agent doesn't affect the other
        guardrails.register_agent_end(&task_id_a).await;
        {
            let agents = guardrails.current_agents.read().await;
            assert_eq!(agents.len(), 1);
            assert!(agents.contains(&task_id_b));
        }
    }

    #[tokio::test]
    async fn test_agent_duplicate_registration_blocked() {
        let config = GuardrailsConfig {
            max_concurrent_agents: 4,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        let agent_id = "task-abc-123";

        assert!(guardrails.check_agent_spawn(agent_id).await.is_allowed());
        guardrails.register_agent_spawn(agent_id).await;

        // Same ID should be blocked even though we haven't hit the limit
        let result = guardrails.check_agent_spawn(agent_id).await;
        assert!(result.is_blocked());
        match result {
            GuardrailResult::Blocked(msg) => {
                assert!(
                    msg.contains("already running"),
                    "Expected 'already running' message, got: {}",
                    msg
                );
            }
            _ => panic!("Expected Blocked result"),
        }

        // After ending, the same ID can be re-used
        guardrails.register_agent_end(agent_id).await;
        assert!(guardrails.check_agent_spawn(agent_id).await.is_allowed());
    }

    #[tokio::test]
    async fn test_check_task_creation_blocks_at_limit() {
        let config = GuardrailsConfig {
            max_concurrent_tasks: 2,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // No tasks yet — creation should be allowed
        assert!(guardrails.check_task_creation().await.is_allowed());

        // Register two tasks to fill the limit
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        guardrails.register_task_start(id1).await;
        guardrails.register_task_start(id2).await;

        // At the limit — creation should be blocked
        let result = guardrails.check_task_creation().await;
        assert!(result.is_blocked());
        match result {
            GuardrailResult::Blocked(msg) => {
                assert!(
                    msg.contains("Maximum concurrent tasks"),
                    "Unexpected message: {}",
                    msg
                );
            }
            _ => panic!("Expected Blocked result"),
        }

        // Free a slot — creation should be allowed again
        guardrails.register_task_end(id1, true).await;
        assert!(guardrails.check_task_creation().await.is_allowed());
    }

    #[tokio::test]
    async fn test_check_task_creation_warns_near_limit() {
        let config = GuardrailsConfig {
            max_concurrent_tasks: 10,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Register 8 tasks (80% of 10)
        for _ in 0..8 {
            guardrails.register_task_start(uuid::Uuid::new_v4()).await;
        }

        // Should get a warning at 80%
        match guardrails.check_task_creation().await {
            GuardrailResult::Warning(msg) => {
                assert!(msg.contains("Approaching"), "Unexpected warning: {}", msg);
            }
            other => panic!("Expected Warning, got {:?}", other),
        }
    }

    // ------- Atomic check-and-record tests -------

    #[test]
    fn test_check_and_record_tokens_basic() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Should succeed and actually reserve
        assert!(guardrails.check_and_record_tokens(500).is_allowed());
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 500);
        assert_eq!(guardrails.get_metrics().get_total_tokens(), 500);

        // Should succeed again (total 900)
        assert!(guardrails.check_and_record_tokens(400).is_allowed());
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 900);

        // Should block (would exceed 1000)
        assert!(guardrails.check_and_record_tokens(200).is_blocked());
        // Counter should not have changed
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 900);
    }

    #[test]
    fn test_check_and_record_tokens_warns_at_80_percent() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // 850 tokens = 85% — should warn
        match guardrails.check_and_record_tokens(850) {
            GuardrailResult::Warning(msg) => {
                assert!(msg.contains("Approaching"), "Unexpected message: {msg}");
            }
            other => panic!("Expected Warning, got {:?}", other),
        }
        // Tokens should still be reserved despite the warning
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 850);
    }

    #[test]
    fn test_check_and_record_tokens_concurrent_never_exceeds_limit() {
        use std::sync::Arc;
        use std::thread;

        let max_tokens: u64 = 10_000;
        let config = GuardrailsConfig {
            max_tokens_per_hour: max_tokens,
            ..Default::default()
        };
        let guardrails = Arc::new(Guardrails::new(config));

        let num_threads = 20;
        let tokens_per_request: u64 = 100;
        let requests_per_thread = 50;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let g = Arc::clone(&guardrails);
                thread::spawn(move || {
                    let mut allowed = 0u64;
                    for _ in 0..requests_per_thread {
                        if g.check_and_record_tokens(tokens_per_request).is_allowed()
                            || matches!(
                                g.check_and_record_tokens(0),
                                GuardrailResult::Warning(_) | GuardrailResult::Allowed
                            )
                        {
                            // Count only the first call's result
                        }
                        // We just care that the counter never exceeds max.
                        if g.check_and_record_tokens(tokens_per_request).is_allowed() {
                            allowed += tokens_per_request;
                        }
                    }
                    allowed
                })
            })
            .collect();

        let _total_allowed: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        let final_tokens = guardrails.get_metrics().get_tokens_this_hour();

        // The critical invariant: the counter must never exceed the limit.
        assert!(
            final_tokens <= max_tokens,
            "Token counter ({final_tokens}) exceeded limit ({max_tokens})!"
        );
    }

    #[test]
    fn test_check_and_record_cost_basic() {
        let config = GuardrailsConfig {
            enforce_budget: true,
            budget_limit_cents: 100.0, // $1.00
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        assert!(guardrails.check_and_record_cost(50.0).is_allowed());
        assert!((guardrails.get_metrics().get_cost_cents() - 50.0).abs() < 0.1);

        assert!(guardrails.check_and_record_cost(40.0).is_allowed());
        assert!((guardrails.get_metrics().get_cost_cents() - 90.0).abs() < 0.1);

        // Should block (would exceed 100)
        assert!(guardrails.check_and_record_cost(20.0).is_blocked());
        // Counter should not have changed
        assert!((guardrails.get_metrics().get_cost_cents() - 90.0).abs() < 0.1);
    }

    #[test]
    fn test_check_and_record_cost_skips_when_not_enforced() {
        let config = GuardrailsConfig {
            enforce_budget: false,
            budget_limit_cents: 100.0,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Should always allow and still record
        assert!(guardrails.check_and_record_cost(200.0).is_allowed());
        assert!(guardrails.get_metrics().get_cost_cents() > 100.0);
    }

    #[test]
    fn test_check_and_record_cost_concurrent_never_exceeds_limit() {
        use std::sync::Arc;
        use std::thread;

        let limit_cents: f64 = 100.0;
        let config = GuardrailsConfig {
            enforce_budget: true,
            budget_limit_cents: limit_cents,
            ..Default::default()
        };
        let guardrails = Arc::new(Guardrails::new(config));

        let num_threads = 20;
        let cost_per_request: f64 = 1.0;
        let requests_per_thread = 50;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let g = Arc::clone(&guardrails);
                thread::spawn(move || {
                    for _ in 0..requests_per_thread {
                        let _ = g.check_and_record_cost(cost_per_request);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let final_cost = guardrails.get_metrics().get_cost_cents();
        assert!(
            final_cost <= limit_cents + 0.1, // small float tolerance
            "Cost ({final_cost}) exceeded limit ({limit_cents})!"
        );
    }

    #[test]
    fn test_atomic_metrics_check_and_record_tokens() {
        let metrics = RuntimeMetrics::default();

        // Basic success
        assert!(metrics.check_and_record_tokens(500, 1000).is_ok());
        assert_eq!(metrics.get_tokens_this_hour(), 500);
        assert_eq!(metrics.get_total_tokens(), 500);

        // Exactly at limit
        assert!(metrics.check_and_record_tokens(500, 1000).is_ok());
        assert_eq!(metrics.get_tokens_this_hour(), 1000);

        // Over limit
        assert!(metrics.check_and_record_tokens(1, 1000).is_err());
        assert_eq!(metrics.get_tokens_this_hour(), 1000); // unchanged
    }

    #[test]
    fn test_atomic_metrics_check_and_record_cost_hundredths() {
        let metrics = RuntimeMetrics::default();

        assert!(
            metrics
                .check_and_record_cost_hundredths(5000, 10000)
                .is_ok()
        );
        assert_eq!(metrics.cost_hundredths.load(Ordering::Relaxed), 5000);

        assert!(
            metrics
                .check_and_record_cost_hundredths(5000, 10000)
                .is_ok()
        );
        assert_eq!(metrics.cost_hundredths.load(Ordering::Relaxed), 10000);

        // Over limit
        assert!(metrics.check_and_record_cost_hundredths(1, 10000).is_err());
        assert_eq!(metrics.cost_hundredths.load(Ordering::Relaxed), 10000);
    }
}

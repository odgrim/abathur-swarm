//! Safety guardrails for the swarm system.
//!
//! Enforces resource limits, safety constraints, and monitors
//! for dangerous operations.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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
        self.tokens_used_this_hour.fetch_add(tokens, Ordering::Relaxed);
        self.total_tokens_used.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Atomically check if `requested` tokens fit within `max`, and record them if so.
    ///
    /// Uses `fetch_update` (CAS loop) to avoid TOCTOU races between callers.
    /// Returns `Ok(new_total)` on success (tokens have been recorded),
    /// or `Err(current)` if adding `requested` would exceed `max`.
    pub fn check_and_record_tokens(&self, requested: u64, max: u64) -> Result<u64, u64> {
        self.tokens_used_this_hour
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current + requested <= max {
                    Some(current + requested)
                } else {
                    None
                }
            })
            .map(|prev| {
                // Also bump the all-time counter (no limit semantics, Relaxed is fine)
                self.total_tokens_used.fetch_add(requested, Ordering::Relaxed);
                prev + requested
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
    pub async fn register_task_start(&self, task_id: uuid::Uuid) {
        let mut tasks = self.current_tasks.write().await;
        tasks.insert(task_id);
        self.metrics.record_task_started();
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
    pub async fn register_agent_spawn(&self, agent_id: &str) {
        let mut agents = self.current_agents.write().await;
        agents.insert(agent_id.to_string());
        self.metrics.record_agent_spawned();
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

    /// Check token usage.
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
    pub fn record_tokens(&self, tokens: u64) {
        self.metrics.record_tokens(tokens);
    }

    /// Atomically check token budget and record usage if within limits.
    ///
    /// This is the race-free alternative to calling `check_tokens` then `record_tokens`
    /// separately. The token increment is guaranteed to only happen if the limit is
    /// not exceeded.
    ///
    /// Returns:
    /// - `Allowed`          — tokens recorded, usage below 80% of max
    /// - `Warning(message)` — tokens recorded, usage at or above 80% of max
    /// - `Blocked(message)` — tokens NOT recorded, limit would be exceeded
    pub fn check_and_record_tokens(&self, requested: u64) -> GuardrailResult {
        let max = self.config.max_tokens_per_hour;
        match self.metrics.check_and_record_tokens(requested, max) {
            Ok(new_total) => {
                if new_total > (max * 80) / 100 {
                    GuardrailResult::Warning(format!(
                        "Approaching token limit: {}/{} used",
                        new_total, max
                    ))
                } else {
                    GuardrailResult::Allowed
                }
            }
            Err(_current) => GuardrailResult::Blocked(format!(
                "Token limit ({}/hour) would be exceeded",
                max
            )),
        }
    }

    /// Record cost.
    pub fn record_cost(&self, cents: f64) {
        self.metrics.record_cost(cents);
    }

    /// Get current metrics.
    pub fn get_metrics(&self) -> &RuntimeMetrics {
        &self.metrics
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(guardrails.check_file_path("config/secrets/api.key").is_blocked());
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
            GuardrailResult::Warning(_) => {},
            other => panic!("Expected Warning, got {:?}", other),
        }

        // Should block when exceeding
        assert!(guardrails.check_tokens(300).is_blocked());
    }

    #[test]
    fn test_check_and_record_tokens_atomic() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Guardrails::new(config);

        // Step 1: Fresh state — 500 tokens fits in 1000, should be Allowed
        match guardrails.check_and_record_tokens(500) {
            GuardrailResult::Allowed => {}
            other => panic!("Expected Allowed, got {:?}", other),
        }
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 500);
        assert_eq!(guardrails.get_metrics().get_total_tokens(), 500);

        // Step 2: Add 300 more via record_tokens to reach 800 (exactly 80%, not > 80%)
        guardrails.record_tokens(300);
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 800);

        // Step 3: At 85%: 800 + 50 = 850 > (1000 * 80 / 100 = 800) → Warning
        //         Tokens ARE recorded atomically.
        match guardrails.check_and_record_tokens(50) {
            GuardrailResult::Warning(msg) => {
                assert!(msg.contains("850"), "Warning message should contain new total: {}", msg);
            }
            other => panic!("Expected Warning, got {:?}", other),
        }
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 850);

        // Step 4: Would exceed — 850 + 200 = 1050 > 1000 → Blocked
        //         Counter must NOT change (atomicity guarantee).
        match guardrails.check_and_record_tokens(200) {
            GuardrailResult::Blocked(msg) => {
                assert!(msg.contains("1000"), "Blocked message should contain limit: {}", msg);
            }
            other => panic!("Expected Blocked, got {:?}", other),
        }
        assert_eq!(
            guardrails.get_metrics().get_tokens_this_hour(),
            850,
            "Counter must not change when request is blocked"
        );

        // Step 5: Exactly at boundary — 850 + 150 = 1000 == max → Warning (not Blocked)
        //         (1000 > 800 → Warning, 1000 <= 1000 → not Blocked)
        match guardrails.check_and_record_tokens(150) {
            GuardrailResult::Warning(_) => {}
            other => panic!("Expected Warning at exactly max, got {:?}", other),
        }
        assert_eq!(guardrails.get_metrics().get_tokens_this_hour(), 1000);
    }

    #[tokio::test]
    async fn test_check_and_record_tokens_concurrent() {
        let config = GuardrailsConfig {
            max_tokens_per_hour: 1000,
            ..Default::default()
        };
        let guardrails = Arc::new(Guardrails::new(config));

        // Spawn 20 tasks each requesting 100 tokens.
        // Only 10 can succeed (10 * 100 = 1000 == max).
        // The other 10 must be blocked. Total recorded must never exceed 1000.
        let handles: Vec<_> = (0..20)
            .map(|_| {
                let g = Arc::clone(&guardrails);
                tokio::spawn(async move { g.check_and_record_tokens(100) })
            })
            .collect();

        let mut allowed_count = 0u32;
        let mut blocked_count = 0u32;
        for handle in handles {
            match handle.await.expect("task panicked") {
                GuardrailResult::Allowed | GuardrailResult::Warning(_) => allowed_count += 1,
                GuardrailResult::Blocked(_) => blocked_count += 1,
            }
        }

        let total_recorded = guardrails.get_metrics().get_tokens_this_hour();

        // Hard invariant: never exceed the limit
        assert!(
            total_recorded <= 1000,
            "Total tokens recorded ({}) exceeded max (1000) — atomicity failure!",
            total_recorded
        );

        // Exactly 10 allowed (each 100 tokens; max=1000, boundary inclusive)
        assert_eq!(allowed_count, 10, "Expected exactly 10 allowed tasks");
        assert_eq!(blocked_count, 10, "Expected exactly 10 blocked tasks");
        assert_eq!(total_recorded, 1000, "Expected exactly 1000 tokens recorded");

        // total_tokens_used must also match (both counters updated together)
        assert_eq!(guardrails.get_metrics().get_total_tokens(), 1000);
    }
}

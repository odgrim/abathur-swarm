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
    pub async fn check_agent_spawn(&self, agent_id: &str) -> GuardrailResult {
        let agents = self.current_agents.read().await;

        if agents.contains(agent_id) {
            return GuardrailResult::Blocked(format!(
                "Agent '{}' is already running",
                agent_id
            ));
        }

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
                assert!(msg.contains("already running"), "Expected 'already running' message, got: {}", msg);
            }
            _ => panic!("Expected Blocked result"),
        }

        // After ending, the same ID can be re-used
        guardrails.register_agent_end(agent_id).await;
        assert!(guardrails.check_agent_spawn(agent_id).await.is_allowed());
    }
}

//! Circuit breaker pattern for failure detection and recovery.
//!
//! Implements the circuit breaker pattern to detect repeated failures
//! and halt execution of affected task chains, preventing cascade failures.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Configuration for circuit breakers.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit.
    pub failure_threshold: u32,
    /// Duration to keep circuit open before trying half-open.
    pub open_timeout: Duration,
    /// Number of successful calls in half-open state to close circuit.
    pub success_threshold: u32,
    /// Window size for tracking failures (older failures are forgotten).
    pub failure_window: Duration,
    /// Whether to enable circuit breakers.
    pub enabled: bool,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            open_timeout: Duration::minutes(5),
            success_threshold: 2,
            failure_window: Duration::minutes(10),
            enabled: true,
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a more sensitive circuit breaker.
    pub fn sensitive() -> Self {
        Self {
            failure_threshold: 3,
            open_timeout: Duration::minutes(2),
            success_threshold: 1,
            failure_window: Duration::minutes(5),
            enabled: true,
        }
    }

    /// Create a more resilient circuit breaker.
    pub fn resilient() -> Self {
        Self {
            failure_threshold: 10,
            open_timeout: Duration::minutes(10),
            success_threshold: 3,
            failure_window: Duration::minutes(15),
            enabled: true,
        }
    }
}

/// State of a circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Circuit is closed, requests flow normally.
    Closed,
    /// Circuit is open, requests are blocked.
    Open,
    /// Circuit is testing if the system has recovered.
    HalfOpen,
}

impl CircuitState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

/// Scope of a circuit breaker.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitScope {
    /// Circuit for a specific task chain (goal/parent task).
    TaskChain(Uuid),
    /// Circuit for a specific agent type.
    Agent(String),
    /// Circuit for a specific operation type.
    Operation(String),
    /// Global circuit (affects everything).
    Global,
}

impl CircuitScope {
    pub fn task_chain(id: Uuid) -> Self {
        Self::TaskChain(id)
    }

    pub fn agent(name: impl Into<String>) -> Self {
        Self::Agent(name.into())
    }

    pub fn operation(name: impl Into<String>) -> Self {
        Self::Operation(name.into())
    }
}

/// A failure record for tracking purposes.
#[derive(Debug, Clone)]
pub struct FailureRecord {
    /// When the failure occurred.
    pub timestamp: DateTime<Utc>,
    /// Error message or description.
    pub error: String,
    /// Related entity (task, agent, etc.).
    pub entity_id: Option<Uuid>,
}

impl FailureRecord {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            error: error.into(),
            entity_id: None,
        }
    }

    pub fn with_entity(mut self, id: Uuid) -> Self {
        self.entity_id = Some(id);
        self
    }
}

/// Individual circuit breaker state.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    /// Circuit scope.
    pub scope: CircuitScope,
    /// Current state.
    pub state: CircuitState,
    /// Recent failures.
    pub failures: Vec<FailureRecord>,
    /// Successful calls in half-open state.
    pub half_open_successes: u32,
    /// When the circuit was opened.
    pub opened_at: Option<DateTime<Utc>>,
    /// When state last changed.
    pub state_changed_at: DateTime<Utc>,
    /// Total times circuit opened.
    pub open_count: u32,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    pub fn new(scope: CircuitScope) -> Self {
        Self {
            scope,
            state: CircuitState::Closed,
            failures: Vec::new(),
            half_open_successes: 0,
            opened_at: None,
            state_changed_at: Utc::now(),
            open_count: 0,
        }
    }

    /// Record a failure.
    pub fn record_failure(&mut self, failure: FailureRecord, config: &CircuitBreakerConfig) {
        self.failures.push(failure);

        // Prune old failures
        let cutoff = Utc::now() - config.failure_window;
        self.failures.retain(|f| f.timestamp > cutoff);

        // Check if we should open the circuit
        if self.state == CircuitState::Closed
            && self.failures.len() as u32 >= config.failure_threshold
        {
            self.open();
        } else if self.state == CircuitState::HalfOpen {
            // Any failure in half-open reopens the circuit
            self.open();
        }
    }

    /// Record a success.
    pub fn record_success(&mut self, config: &CircuitBreakerConfig) {
        if self.state == CircuitState::HalfOpen {
            self.half_open_successes += 1;
            if self.half_open_successes >= config.success_threshold {
                self.close();
            }
        }
    }

    /// Open the circuit.
    fn open(&mut self) {
        self.state = CircuitState::Open;
        self.opened_at = Some(Utc::now());
        self.state_changed_at = Utc::now();
        self.half_open_successes = 0;
        self.open_count += 1;
    }

    /// Close the circuit.
    fn close(&mut self) {
        self.state = CircuitState::Closed;
        self.opened_at = None;
        self.state_changed_at = Utc::now();
        self.half_open_successes = 0;
        self.failures.clear();
    }

    /// Transition to half-open.
    fn half_open(&mut self) {
        self.state = CircuitState::HalfOpen;
        self.state_changed_at = Utc::now();
        self.half_open_successes = 0;
    }

    /// Check if the circuit allows requests.
    pub fn allows(&mut self, config: &CircuitBreakerConfig) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if timeout has passed
                if let Some(opened_at) = self.opened_at {
                    if Utc::now() > opened_at + config.open_timeout {
                        self.half_open();
                        true // Allow one test request
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true, // Allow test requests
        }
    }

    /// Get recent failure count within the window.
    pub fn recent_failure_count(&self, config: &CircuitBreakerConfig) -> usize {
        let cutoff = Utc::now() - config.failure_window;
        self.failures.iter().filter(|f| f.timestamp > cutoff).count()
    }

    /// Manually reset the circuit.
    pub fn reset(&mut self) {
        self.close();
        self.open_count = 0;
    }
}

/// Result of a circuit breaker check.
#[derive(Debug, Clone)]
pub enum CircuitCheckResult {
    /// Request is allowed.
    Allowed,
    /// Request is blocked by open circuit.
    Blocked {
        scope: CircuitScope,
        opened_at: DateTime<Utc>,
        retry_after: DateTime<Utc>,
    },
    /// Circuit is in half-open state, testing recovery.
    Testing { scope: CircuitScope },
}

impl CircuitCheckResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed | Self::Testing { .. })
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }
}

/// Recovery action to take when a circuit opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Just halt and wait for timeout.
    Halt,
    /// Trigger DAG restructuring to find alternative approach.
    TriggerRestructure {
        task_id: Uuid,
        reason: String,
    },
    /// Spawn a diagnostic agent to investigate.
    SpawnDiagnostic {
        task_id: Uuid,
        error_context: String,
    },
    /// Escalate to meta-planner for task re-decomposition.
    EscalateToMetaPlanner {
        goal_id: Uuid,
        reason: String,
    },
    /// Notify and continue (for non-critical circuits).
    NotifyOnly {
        message: String,
    },
}

impl RecoveryAction {
    /// Create a restructure action.
    pub fn restructure(task_id: Uuid, reason: impl Into<String>) -> Self {
        Self::TriggerRestructure {
            task_id,
            reason: reason.into(),
        }
    }

    /// Create a diagnostic action.
    pub fn diagnose(task_id: Uuid, context: impl Into<String>) -> Self {
        Self::SpawnDiagnostic {
            task_id,
            error_context: context.into(),
        }
    }

    /// Create an escalation action.
    pub fn escalate(goal_id: Uuid, reason: impl Into<String>) -> Self {
        Self::EscalateToMetaPlanner {
            goal_id,
            reason: reason.into(),
        }
    }
}

/// Event emitted when a circuit breaker trips.
#[derive(Debug, Clone)]
pub struct CircuitTrippedEvent {
    /// Which circuit tripped.
    pub scope: CircuitScope,
    /// When it tripped.
    pub tripped_at: DateTime<Utc>,
    /// How many times this circuit has opened.
    pub open_count: u32,
    /// Recent failures that caused the trip.
    pub recent_failures: Vec<String>,
    /// Recommended recovery action.
    pub recovery_action: RecoveryAction,
}

/// Policy for determining recovery actions.
#[derive(Debug, Clone)]
pub struct RecoveryPolicy {
    /// Default action when no specific policy matches.
    pub default_action: RecoveryAction,
    /// Map from task chain to specific recovery action.
    pub task_chain_policies: HashMap<Uuid, RecoveryAction>,
    /// Whether to always try restructuring first.
    pub prefer_restructure: bool,
    /// Threshold for escalating to meta-planner.
    pub escalation_threshold: u32,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            default_action: RecoveryAction::Halt,
            task_chain_policies: HashMap::new(),
            prefer_restructure: true,
            escalation_threshold: 3,
        }
    }
}

impl RecoveryPolicy {
    /// Create a policy that prefers restructuring.
    pub fn restructure_first() -> Self {
        Self {
            default_action: RecoveryAction::Halt,
            prefer_restructure: true,
            ..Default::default()
        }
    }

    /// Determine recovery action for a tripped circuit.
    pub fn determine_action(&self, scope: &CircuitScope, open_count: u32) -> RecoveryAction {
        // Check for specific task chain policy
        if let CircuitScope::TaskChain(task_id) = scope {
            if let Some(action) = self.task_chain_policies.get(task_id) {
                return action.clone();
            }

            // If prefer restructure is enabled and we have a task chain
            if self.prefer_restructure && open_count < self.escalation_threshold {
                return RecoveryAction::restructure(
                    *task_id,
                    format!("Circuit opened {} times, attempting restructure", open_count),
                );
            }

            // Escalate if we've exceeded threshold
            if open_count >= self.escalation_threshold {
                return RecoveryAction::escalate(
                    *task_id, // Using task_id as goal_id placeholder
                    format!(
                        "Circuit opened {} times exceeds escalation threshold {}",
                        open_count, self.escalation_threshold
                    ),
                );
            }
        }

        self.default_action.clone()
    }

    /// Register a specific policy for a task chain.
    pub fn register_task_policy(&mut self, task_id: Uuid, action: RecoveryAction) {
        self.task_chain_policies.insert(task_id, action);
    }
}

/// Statistics for a circuit breaker.
#[derive(Debug, Clone, Serialize)]
pub struct CircuitStats {
    pub scope: String,
    pub state: String,
    pub failure_count: usize,
    pub open_count: u32,
    pub opened_at: Option<DateTime<Utc>>,
    pub state_changed_at: DateTime<Utc>,
}

/// Service for managing circuit breakers.
pub struct CircuitBreakerService {
    config: CircuitBreakerConfig,
    circuits: Arc<RwLock<HashMap<CircuitScope, CircuitBreaker>>>,
    recovery_policy: Arc<RwLock<RecoveryPolicy>>,
    event_sender: Option<tokio::sync::mpsc::Sender<CircuitTrippedEvent>>,
}

impl CircuitBreakerService {
    /// Create a new circuit breaker service.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            circuits: Arc::new(RwLock::new(HashMap::new())),
            recovery_policy: Arc::new(RwLock::new(RecoveryPolicy::default())),
            event_sender: None,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Create with a recovery policy.
    pub fn with_recovery_policy(config: CircuitBreakerConfig, policy: RecoveryPolicy) -> Self {
        Self {
            config,
            circuits: Arc::new(RwLock::new(HashMap::new())),
            recovery_policy: Arc::new(RwLock::new(policy)),
            event_sender: None,
        }
    }

    /// Set the event sender for circuit tripped events.
    pub fn with_event_sender(mut self, sender: tokio::sync::mpsc::Sender<CircuitTrippedEvent>) -> Self {
        self.event_sender = Some(sender);
        self
    }

    /// Update the recovery policy.
    pub async fn set_recovery_policy(&self, policy: RecoveryPolicy) {
        let mut rp = self.recovery_policy.write().await;
        *rp = policy;
    }

    /// Register a task-specific recovery action.
    pub async fn register_task_recovery(&self, task_id: Uuid, action: RecoveryAction) {
        let mut policy = self.recovery_policy.write().await;
        policy.register_task_policy(task_id, action);
    }

    /// Check if a request is allowed for the given scope.
    pub async fn check(&self, scope: CircuitScope) -> CircuitCheckResult {
        if !self.config.enabled {
            return CircuitCheckResult::Allowed;
        }

        let mut circuits = self.circuits.write().await;
        let circuit = circuits.entry(scope.clone()).or_insert_with(|| CircuitBreaker::new(scope.clone()));

        if circuit.allows(&self.config) {
            if circuit.state == CircuitState::HalfOpen {
                CircuitCheckResult::Testing { scope }
            } else {
                CircuitCheckResult::Allowed
            }
        } else {
            CircuitCheckResult::Blocked {
                scope,
                opened_at: circuit.opened_at.unwrap_or_else(Utc::now),
                retry_after: circuit.opened_at.unwrap_or_else(Utc::now) + self.config.open_timeout,
            }
        }
    }

    /// Record a failure for the given scope.
    pub async fn record_failure(&self, scope: CircuitScope, error: impl Into<String>) {
        if !self.config.enabled {
            return;
        }

        let error_str = error.into();
        let mut circuits = self.circuits.write().await;
        let circuit = circuits.entry(scope.clone()).or_insert_with(|| CircuitBreaker::new(scope.clone()));

        let was_closed = circuit.state == CircuitState::Closed;
        circuit.record_failure(FailureRecord::new(error_str), &self.config);

        // Check if we just tripped the circuit
        if was_closed && circuit.state == CircuitState::Open {
            // Emit event with recovery action
            if let Some(ref sender) = self.event_sender {
                let policy = self.recovery_policy.read().await;
                let recovery_action = policy.determine_action(&scope, circuit.open_count);

                let event = CircuitTrippedEvent {
                    scope,
                    tripped_at: Utc::now(),
                    open_count: circuit.open_count,
                    recent_failures: circuit.failures.iter().map(|f| f.error.clone()).collect(),
                    recovery_action,
                };

                let _ = sender.try_send(event);
            }
        }
    }

    /// Record a failure with entity ID.
    pub async fn record_failure_with_entity(
        &self,
        scope: CircuitScope,
        error: impl Into<String>,
        entity_id: Uuid,
    ) {
        if !self.config.enabled {
            return;
        }

        let mut circuits = self.circuits.write().await;
        let circuit = circuits.entry(scope.clone()).or_insert_with(|| CircuitBreaker::new(scope));
        circuit.record_failure(FailureRecord::new(error).with_entity(entity_id), &self.config);
    }

    /// Record a success for the given scope.
    pub async fn record_success(&self, scope: CircuitScope) {
        if !self.config.enabled {
            return;
        }

        let mut circuits = self.circuits.write().await;
        if let Some(circuit) = circuits.get_mut(&scope) {
            circuit.record_success(&self.config);
        }
    }

    /// Get the state of a circuit.
    pub async fn get_state(&self, scope: &CircuitScope) -> Option<CircuitState> {
        let circuits = self.circuits.read().await;
        circuits.get(scope).map(|c| c.state)
    }

    /// Get statistics for all circuits.
    pub async fn stats(&self) -> Vec<CircuitStats> {
        let circuits = self.circuits.read().await;
        circuits
            .values()
            .map(|c| CircuitStats {
                scope: format!("{:?}", c.scope),
                state: c.state.as_str().to_string(),
                failure_count: c.recent_failure_count(&self.config),
                open_count: c.open_count,
                opened_at: c.opened_at,
                state_changed_at: c.state_changed_at,
            })
            .collect()
    }

    /// Get open circuits.
    pub async fn get_open_circuits(&self) -> Vec<CircuitScope> {
        let circuits = self.circuits.read().await;
        circuits
            .iter()
            .filter(|(_, c)| c.state == CircuitState::Open)
            .map(|(s, _)| s.clone())
            .collect()
    }

    /// Manually reset a circuit.
    pub async fn reset(&self, scope: &CircuitScope) {
        let mut circuits = self.circuits.write().await;
        if let Some(circuit) = circuits.get_mut(scope) {
            circuit.reset();
        }
    }

    /// Reset all circuits.
    pub async fn reset_all(&self) {
        let mut circuits = self.circuits.write().await;
        for circuit in circuits.values_mut() {
            circuit.reset();
        }
    }

    /// Remove a circuit.
    pub async fn remove(&self, scope: &CircuitScope) {
        let mut circuits = self.circuits.write().await;
        circuits.remove(scope);
    }

    /// Get configuration.
    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }
}

/// Execute a function with circuit breaker protection.
pub async fn with_circuit_breaker<F, T, E>(
    service: &CircuitBreakerService,
    scope: CircuitScope,
    f: F,
) -> Result<T, CircuitBreakerError<E>>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let check = service.check(scope.clone()).await;

    match check {
        CircuitCheckResult::Blocked {
            scope,
            opened_at,
            retry_after,
        } => Err(CircuitBreakerError::CircuitOpen {
            scope,
            opened_at,
            retry_after,
        }),
        CircuitCheckResult::Allowed | CircuitCheckResult::Testing { .. } => {
            match f.await {
                Ok(result) => {
                    service.record_success(scope).await;
                    Ok(result)
                }
                Err(e) => {
                    service.record_failure(scope, format!("{:?}", &e as &dyn std::fmt::Debug)).await;
                    Err(CircuitBreakerError::OperationFailed(e))
                }
            }
        }
    }
}

/// Error from circuit breaker protected operation.
#[derive(Debug)]
pub enum CircuitBreakerError<E> {
    /// The circuit is open and blocking requests.
    CircuitOpen {
        scope: CircuitScope,
        opened_at: DateTime<Utc>,
        retry_after: DateTime<Utc>,
    },
    /// The underlying operation failed.
    OperationFailed(E),
}

impl<E: std::fmt::Display> std::fmt::Display for CircuitBreakerError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CircuitOpen { scope, retry_after, .. } => {
                write!(
                    f,
                    "Circuit breaker open for {:?}, retry after {}",
                    scope, retry_after
                )
            }
            Self::OperationFailed(e) => write!(f, "Operation failed: {}", e),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CircuitBreakerError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CircuitOpen { .. } => None,
            Self::OperationFailed(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.failure_threshold, 5);
        assert!(config.enabled);
    }

    #[test]
    fn test_circuit_state() {
        let mut circuit = CircuitBreaker::new(CircuitScope::Global);
        assert_eq!(circuit.state, CircuitState::Closed);

        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };

        // Record failures below threshold
        circuit.record_failure(FailureRecord::new("error 1"), &config);
        circuit.record_failure(FailureRecord::new("error 2"), &config);
        assert_eq!(circuit.state, CircuitState::Closed);

        // Third failure opens the circuit
        circuit.record_failure(FailureRecord::new("error 3"), &config);
        assert_eq!(circuit.state, CircuitState::Open);
        assert!(circuit.opened_at.is_some());
        assert_eq!(circuit.open_count, 1);
    }

    #[test]
    fn test_circuit_allows() {
        let mut circuit = CircuitBreaker::new(CircuitScope::Global);
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            open_timeout: Duration::seconds(1),
            ..Default::default()
        };

        assert!(circuit.allows(&config)); // Closed circuit allows

        circuit.record_failure(FailureRecord::new("error 1"), &config);
        circuit.record_failure(FailureRecord::new("error 2"), &config);
        assert_eq!(circuit.state, CircuitState::Open);
        assert!(!circuit.allows(&config)); // Open circuit blocks
    }

    #[test]
    fn test_half_open_recovery() {
        let mut circuit = CircuitBreaker::new(CircuitScope::Global);
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            ..Default::default()
        };

        // Open the circuit
        circuit.record_failure(FailureRecord::new("error 1"), &config);
        circuit.record_failure(FailureRecord::new("error 2"), &config);
        assert_eq!(circuit.state, CircuitState::Open);

        // Simulate timeout by manually transitioning
        circuit.half_open();
        assert_eq!(circuit.state, CircuitState::HalfOpen);

        // One success isn't enough
        circuit.record_success(&config);
        assert_eq!(circuit.state, CircuitState::HalfOpen);

        // Two successes close the circuit
        circuit.record_success(&config);
        assert_eq!(circuit.state, CircuitState::Closed);
    }

    #[test]
    fn test_half_open_failure() {
        let mut circuit = CircuitBreaker::new(CircuitScope::Global);
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            ..Default::default()
        };

        // Open the circuit
        circuit.record_failure(FailureRecord::new("error 1"), &config);
        circuit.record_failure(FailureRecord::new("error 2"), &config);

        // Transition to half-open
        circuit.half_open();
        assert_eq!(circuit.state, CircuitState::HalfOpen);

        // Failure in half-open reopens
        circuit.record_failure(FailureRecord::new("error 3"), &config);
        assert_eq!(circuit.state, CircuitState::Open);
        assert_eq!(circuit.open_count, 2);
    }

    #[test]
    fn test_circuit_scope() {
        let task_scope = CircuitScope::task_chain(Uuid::new_v4());
        assert!(matches!(task_scope, CircuitScope::TaskChain(_)));

        let agent_scope = CircuitScope::agent("code-implementer");
        assert!(matches!(agent_scope, CircuitScope::Agent(_)));

        let op_scope = CircuitScope::operation("file-write");
        assert!(matches!(op_scope, CircuitScope::Operation(_)));
    }

    #[test]
    fn test_circuit_reset() {
        let mut circuit = CircuitBreaker::new(CircuitScope::Global);
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            ..Default::default()
        };

        // Open the circuit
        circuit.record_failure(FailureRecord::new("error 1"), &config);
        circuit.record_failure(FailureRecord::new("error 2"), &config);
        assert_eq!(circuit.state, CircuitState::Open);
        assert_eq!(circuit.open_count, 1);

        // Reset
        circuit.reset();
        assert_eq!(circuit.state, CircuitState::Closed);
        assert_eq!(circuit.open_count, 0);
        assert!(circuit.failures.is_empty());
    }

    #[tokio::test]
    async fn test_circuit_breaker_service() {
        let service = CircuitBreakerService::new(CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        let scope = CircuitScope::agent("test-agent");

        // Initially allowed
        let result = service.check(scope.clone()).await;
        assert!(result.is_allowed());

        // Record failures
        service.record_failure(scope.clone(), "error 1").await;
        service.record_failure(scope.clone(), "error 2").await;

        // Still allowed (below threshold)
        let result = service.check(scope.clone()).await;
        assert!(result.is_allowed());

        // Third failure trips the circuit
        service.record_failure(scope.clone(), "error 3").await;

        // Now blocked
        let result = service.check(scope.clone()).await;
        assert!(result.is_blocked());
    }

    #[tokio::test]
    async fn test_circuit_breaker_stats() {
        let service = CircuitBreakerService::with_defaults();

        service.record_failure(CircuitScope::agent("agent-1"), "error").await;
        service.record_failure(CircuitScope::agent("agent-2"), "error").await;

        let stats = service.stats().await;
        assert_eq!(stats.len(), 2);
    }

    #[tokio::test]
    async fn test_circuit_breaker_disabled() {
        let service = CircuitBreakerService::new(CircuitBreakerConfig {
            enabled: false,
            failure_threshold: 1,
            ..Default::default()
        });

        let scope = CircuitScope::Global;

        // Record many failures
        for _ in 0..10 {
            service.record_failure(scope.clone(), "error").await;
        }

        // Still allowed because circuit breaker is disabled
        let result = service.check(scope).await;
        assert!(result.is_allowed());
    }

    #[test]
    fn test_check_result_methods() {
        let allowed = CircuitCheckResult::Allowed;
        assert!(allowed.is_allowed());
        assert!(!allowed.is_blocked());

        let blocked = CircuitCheckResult::Blocked {
            scope: CircuitScope::Global,
            opened_at: Utc::now(),
            retry_after: Utc::now() + Duration::minutes(5),
        };
        assert!(blocked.is_blocked());
        assert!(!blocked.is_allowed());

        let testing = CircuitCheckResult::Testing {
            scope: CircuitScope::Global,
        };
        assert!(testing.is_allowed());
        assert!(!testing.is_blocked());
    }

    #[test]
    fn test_recovery_action_helpers() {
        let task_id = Uuid::new_v4();
        let goal_id = Uuid::new_v4();

        let restructure = RecoveryAction::restructure(task_id, "test reason");
        assert!(matches!(restructure, RecoveryAction::TriggerRestructure { .. }));

        let diagnose = RecoveryAction::diagnose(task_id, "error context");
        assert!(matches!(diagnose, RecoveryAction::SpawnDiagnostic { .. }));

        let escalate = RecoveryAction::escalate(goal_id, "escalation reason");
        assert!(matches!(escalate, RecoveryAction::EscalateToMetaPlanner { .. }));
    }

    #[test]
    fn test_recovery_policy_default() {
        let policy = RecoveryPolicy::default();
        assert!(policy.prefer_restructure);
        assert_eq!(policy.escalation_threshold, 3);
    }

    #[test]
    fn test_recovery_policy_determine_action() {
        let policy = RecoveryPolicy::restructure_first();
        let task_id = Uuid::new_v4();

        // First time - should restructure
        let action = policy.determine_action(&CircuitScope::TaskChain(task_id), 1);
        assert!(matches!(action, RecoveryAction::TriggerRestructure { .. }));

        // At threshold - should escalate
        let action = policy.determine_action(&CircuitScope::TaskChain(task_id), 3);
        assert!(matches!(action, RecoveryAction::EscalateToMetaPlanner { .. }));

        // Global scope - should use default
        let action = policy.determine_action(&CircuitScope::Global, 1);
        assert!(matches!(action, RecoveryAction::Halt));
    }

    #[test]
    fn test_recovery_policy_task_specific() {
        let mut policy = RecoveryPolicy::default();
        let task_id = Uuid::new_v4();

        // Register specific action for task
        policy.register_task_policy(task_id, RecoveryAction::NotifyOnly {
            message: "Test notification".to_string(),
        });

        // Should use the specific policy
        let action = policy.determine_action(&CircuitScope::TaskChain(task_id), 1);
        assert!(matches!(action, RecoveryAction::NotifyOnly { .. }));
    }

    #[tokio::test]
    async fn test_circuit_breaker_event_emission() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        let service = CircuitBreakerService::new(CircuitBreakerConfig {
            failure_threshold: 2,
            ..Default::default()
        }).with_event_sender(tx);

        let scope = CircuitScope::task_chain(Uuid::new_v4());

        // First failure - no event
        service.record_failure(scope.clone(), "error 1").await;
        assert!(rx.try_recv().is_err());

        // Second failure trips circuit - should get event
        service.record_failure(scope.clone(), "error 2").await;

        let event = rx.try_recv().unwrap();
        assert!(matches!(event.recovery_action, RecoveryAction::TriggerRestructure { .. }));
        assert_eq!(event.open_count, 1);
        assert_eq!(event.recent_failures.len(), 2);
    }

    #[tokio::test]
    async fn test_register_task_recovery() {
        let service = CircuitBreakerService::with_defaults();
        let task_id = Uuid::new_v4();

        service.register_task_recovery(
            task_id,
            RecoveryAction::diagnose(task_id, "custom diagnostic"),
        ).await;

        // Verify policy was updated
        let policy = service.recovery_policy.read().await;
        assert!(policy.task_chain_policies.contains_key(&task_id));
    }
}

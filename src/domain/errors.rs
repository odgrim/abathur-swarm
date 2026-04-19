//! Domain errors for the Abathur swarm system.

use thiserror::Error;
use uuid::Uuid;

/// Domain-level errors that can occur in the Abathur system.
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("Goal not found: {0}")]
    GoalNotFound(Uuid),

    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("Invalid state transition from {from} to {to}: {reason}")]
    InvalidStateTransition {
        from: String,
        to: String,
        reason: String,
    },

    #[error("Task dependency cycle detected involving task: {0}")]
    DependencyCycle(Uuid),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Memory not found: {0}")]
    MemoryNotFound(Uuid),

    /// Generic validation error for user input / invariants that don't fit
    /// a more specific variant. Prefer a specific variant where the failure
    /// mode is clear (see `WorkflowError`, `ConfigError`, `LimitExceeded`, etc).
    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Concurrency conflict: {entity} {id} was modified")]
    ConcurrencyConflict { entity: String, id: String },

    /// Backstop for execution failures that don't match a more specific
    /// variant. Prefer `SubstrateError`, `ExternalServiceError`, or
    /// `TimeoutError` where the failure mode is clear.
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Task schedule not found: {0}")]
    TaskScheduleNotFound(Uuid),

    /// Configuration error (missing dependencies, unset env vars, etc.).
    #[error("Configuration error: {key}: {reason}")]
    ConfigError { key: String, reason: String },

    /// Operation exceeded its time limit.
    #[error("Operation '{operation}' timed out after {limit_secs}s")]
    TimeoutError { operation: String, limit_secs: u64 },

    /// LLM / agent substrate failure (Claude Code process, Anthropic API,
    /// Overmind invocation).
    #[error("Substrate error: {0}")]
    SubstrateError(String),

    /// A resource limit (spawn count, budget, depth, quota) was exceeded.
    #[error("{kind} limit exceeded: {value} > {limit}")]
    LimitExceeded {
        kind: String,
        value: u64,
        limit: u64,
    },

    /// Error interacting with an external service (git, GitHub, ClickUp,
    /// federation peer, etc.).
    #[error("External service '{service}' error: {reason}")]
    ExternalServiceError { service: String, reason: String },

    /// A code path that explicitly isn't built or a feature unsupported by
    /// the current adapter.
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// Workflow-state-machine error (missing workflow state, unknown template,
    /// illegal phase transition).
    #[error("Workflow '{workflow}' error: {reason}")]
    WorkflowError { workflow: String, reason: String },
}

pub type DomainResult<T> = Result<T, DomainError>;

impl From<sqlx::Error> for DomainError {
    fn from(err: sqlx::Error) -> Self {
        DomainError::DatabaseError(err.to_string())
    }
}

impl From<serde_json::Error> for DomainError {
    fn from(err: serde_json::Error) -> Self {
        DomainError::SerializationError(err.to_string())
    }
}

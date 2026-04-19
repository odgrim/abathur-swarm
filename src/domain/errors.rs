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

impl From<std::io::Error> for DomainError {
    fn from(err: std::io::Error) -> Self {
        DomainError::ExecutionFailed(err.to_string())
    }
}

impl From<tokio::task::JoinError> for DomainError {
    fn from(err: tokio::task::JoinError) -> Self {
        DomainError::ExecutionFailed(err.to_string())
    }
}

impl DomainError {
    /// Prepend an operator-facing context message, preserving the original
    /// variant where possible. Variants that don't carry a `String`-shaped
    /// message fall back to `ValidationFailed("{ctx}: {variant-Display}")`.
    ///
    /// Variants that preserve their tag (context prepended):
    ///   DatabaseError, ValidationFailed, SerializationError, ExecutionFailed,
    ///   SubstrateError, NotImplemented, InvalidStateTransition (`reason`),
    ///   ConfigError (`reason`), WorkflowError (`reason`),
    ///   ExternalServiceError (`reason`).
    ///
    /// Variants that lossy-wrap into ValidationFailed:
    ///   GoalNotFound, TaskNotFound, DependencyCycle, AgentNotFound,
    ///   MemoryNotFound, ConcurrencyConflict, TaskScheduleNotFound,
    ///   TimeoutError, LimitExceeded.
    pub fn with_context_msg(self, ctx: String) -> Self {
        match self {
            Self::DatabaseError(m) => Self::DatabaseError(format!("{ctx}: {m}")),
            Self::ValidationFailed(m) => Self::ValidationFailed(format!("{ctx}: {m}")),
            Self::SerializationError(m) => Self::SerializationError(format!("{ctx}: {m}")),
            Self::ExecutionFailed(m) => Self::ExecutionFailed(format!("{ctx}: {m}")),
            Self::SubstrateError(m) => Self::SubstrateError(format!("{ctx}: {m}")),
            Self::NotImplemented(m) => Self::NotImplemented(format!("{ctx}: {m}")),
            Self::InvalidStateTransition {
                from,
                to,
                reason,
            } => Self::InvalidStateTransition {
                from,
                to,
                reason: format!("{ctx}: {reason}"),
            },
            Self::ConfigError { key, reason } => Self::ConfigError {
                key,
                reason: format!("{ctx}: {reason}"),
            },
            Self::WorkflowError { workflow, reason } => Self::WorkflowError {
                workflow,
                reason: format!("{ctx}: {reason}"),
            },
            Self::ExternalServiceError { service, reason } => Self::ExternalServiceError {
                service,
                reason: format!("{ctx}: {reason}"),
            },
            // Fallback: variants with structured non-message fields get
            // lossy-wrapped so context isn't silently dropped.
            other => Self::ValidationFailed(format!("{ctx}: {other}")),
        }
    }
}

/// Extension trait that adds `.context()` / `.with_context()` to any
/// `Result<T, E>` where `E: Into<DomainError>`, preserving variant info.
///
/// This is the `DomainError` analogue of `anyhow::Context`. Use it at `?`
/// boundaries where a bare error loses information the operator needs to
/// diagnose a failure (which DB op, which git verb, which file path).
pub trait DomainResultExt<T> {
    fn context<C: Into<String>>(self, ctx: C) -> DomainResult<T>;
    fn with_context<C: Into<String>, F: FnOnce() -> C>(self, f: F) -> DomainResult<T>;
}

impl<T, E: Into<DomainError>> DomainResultExt<T> for Result<T, E> {
    fn context<C: Into<String>>(self, ctx: C) -> DomainResult<T> {
        self.map_err(|e| e.into().with_context_msg(ctx.into()))
    }
    fn with_context<C: Into<String>, F: FnOnce() -> C>(self, f: F) -> DomainResult<T> {
        self.map_err(|e| e.into().with_context_msg(f().into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_preserves_database_error() {
        let err: DomainResult<()> = Err(DomainError::DatabaseError("timeout".to_string()));
        let ctxed = err.context("loading merge_requests");
        let e = ctxed.unwrap_err();
        match e {
            DomainError::DatabaseError(m) => assert_eq!(m, "loading merge_requests: timeout"),
            other => panic!("expected DatabaseError, got {:?}", other),
        }
    }

    #[test]
    fn context_preserves_validation_failed() {
        let err: DomainResult<()> = Err(DomainError::ValidationFailed("bad input".to_string()));
        let ctxed = err.context("validating workdir");
        let e = ctxed.unwrap_err();
        match e {
            DomainError::ValidationFailed(m) => assert_eq!(m, "validating workdir: bad input"),
            other => panic!("expected ValidationFailed, got {:?}", other),
        }
    }

    #[test]
    fn context_preserves_invalid_state_transition_reason() {
        let err: DomainResult<()> = Err(DomainError::InvalidStateTransition {
            from: "Queued".to_string(),
            to: "Completed".to_string(),
            reason: "not allowed".to_string(),
        });
        let ctxed = err.context("merge_queue.process_next");
        let e = ctxed.unwrap_err();
        match e {
            DomainError::InvalidStateTransition {
                from,
                to,
                reason,
            } => {
                assert_eq!(from, "Queued");
                assert_eq!(to, "Completed");
                assert_eq!(reason, "merge_queue.process_next: not allowed");
            }
            other => panic!("expected InvalidStateTransition, got {:?}", other),
        }
    }

    #[test]
    fn context_is_noop_on_ok() {
        let ok: DomainResult<i32> = Ok(42);
        let after = ok.context("some context");
        assert_eq!(after.unwrap(), 42);
    }

    #[test]
    fn with_context_closure_not_called_on_ok() {
        use std::cell::Cell;
        let called = Cell::new(false);
        let ok: DomainResult<i32> = Ok(7);
        let after = ok.with_context(|| {
            called.set(true);
            "unused"
        });
        assert_eq!(after.unwrap(), 7);
        assert!(!called.get(), "closure must not run on Ok");
    }

    #[test]
    fn with_context_closure_runs_on_err() {
        use std::cell::Cell;
        let called = Cell::new(false);
        let err: DomainResult<i32> = Err(DomainError::ExecutionFailed("boom".to_string()));
        let after = err.with_context(|| {
            called.set(true);
            "ran"
        });
        let e = after.unwrap_err();
        assert!(called.get(), "closure must run on Err");
        match e {
            DomainError::ExecutionFailed(m) => assert_eq!(m, "ran: boom"),
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }

    #[test]
    fn context_preserves_external_service_error_reason() {
        let err: DomainResult<()> = Err(DomainError::ExternalServiceError {
            service: "git".to_string(),
            reason: "exit 1".to_string(),
        });
        let ctxed = err.context("git merge-tree on main");
        match ctxed.unwrap_err() {
            DomainError::ExternalServiceError { service, reason } => {
                assert_eq!(service, "git");
                assert_eq!(reason, "git merge-tree on main: exit 1");
            }
            other => panic!("expected ExternalServiceError, got {:?}", other),
        }
    }

    #[test]
    fn context_fallback_wraps_task_not_found() {
        let id = Uuid::nil();
        let err: DomainResult<()> = Err(DomainError::TaskNotFound(id));
        let ctxed = err.context("merge_queue.queue_stage2_if_ready");
        match ctxed.unwrap_err() {
            DomainError::ValidationFailed(m) => {
                assert!(m.starts_with("merge_queue.queue_stage2_if_ready:"));
                assert!(m.contains("Task not found"));
            }
            other => panic!("expected ValidationFailed fallback, got {:?}", other),
        }
    }

    #[test]
    fn from_io_error_yields_execution_failed() {
        let ioerr = std::io::Error::other("disk gone");
        let de: DomainError = ioerr.into();
        match de {
            DomainError::ExecutionFailed(m) => assert!(m.contains("disk gone")),
            other => panic!("expected ExecutionFailed, got {:?}", other),
        }
    }
}

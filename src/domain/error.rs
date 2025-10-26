use thiserror::Error;
use uuid::Uuid;

use super::models::task::TaskStatus;

/// Domain-level errors for task operations
#[derive(Error, Debug)]
pub enum TaskError {
    #[error("Invalid summary: {0}")]
    InvalidSummary(String),

    #[error("Invalid priority: {0} (must be 0-10)")]
    InvalidPriority(u8),

    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition { from: TaskStatus, to: TaskStatus },

    #[error("Circular dependency detected: {0:?}")]
    CircularDependency(Vec<Uuid>),

    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("Task has unmet dependencies: {0:?}")]
    UnmetDependencies(Vec<Uuid>),

    #[error("Task cannot be retried: retry count {retry_count} exceeds max retries {max_retries}")]
    MaxRetriesExceeded { retry_count: u32, max_retries: u32 },

    #[error("Task execution timeout exceeded: {timeout_seconds}s")]
    TimeoutExceeded { timeout_seconds: u32 },

    #[error("Task is in terminal state: {0:?}")]
    TaskInTerminalState(TaskStatus),
}

/// Domain-level errors
#[derive(Error, Debug)]
pub enum DomainError {
    #[error("Task error: {0}")]
    Task(#[from] TaskError),
}

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

    #[error("Invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    #[error("Task dependency cycle detected involving task: {0}")]
    DependencyCycle(Uuid),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Memory not found: {0}")]
    MemoryNotFound(Uuid),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Concurrency conflict: {entity} {id} was modified")]
    ConcurrencyConflict { entity: String, id: String },

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
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

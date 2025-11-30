use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Query failed: {0}")]
    QueryFailed(#[from] sqlx::Error),

    #[error("UUID parse error: {0}")]
    UuidParseError(#[from] uuid::Error),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("DateTime parse error: {0}")]
    DateTimeParseError(#[from] chrono::ParseError),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Not found: {0}")]
    NotFound(uuid::Uuid),

    #[error("Session not found: {0}")]
    SessionNotFound(uuid::Uuid),

    #[error("Invalid state update: {0}")]
    InvalidStateUpdate(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Anyhow error: {0}")]
    AnyhowError(#[from] anyhow::Error),

    /// Optimistic lock conflict - task was modified by another process.
    /// The caller should re-read the task and retry the operation.
    #[error("Optimistic lock conflict for task {task_id}: expected version {expected_version}, but task was modified")]
    OptimisticLockConflict {
        task_id: uuid::Uuid,
        expected_version: u32,
    },
}

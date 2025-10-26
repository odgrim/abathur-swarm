use thiserror::Error;

/// Database operation errors
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Query failed: {0}")]
    QueryFailed(#[from] sqlx::Error),

    #[error("Task not found: {0}")]
    TaskNotFound(uuid::Uuid),

    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(#[from] chrono::ParseError),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Connection pool error: {0}")]
    ConnectionPoolError(String),

    #[error("Migration error: {0}")]
    MigrationError(String),

    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("Parse error: {0}")]
    ParseError(String),
}

impl From<anyhow::Error> for DatabaseError {
    fn from(err: anyhow::Error) -> Self {
        DatabaseError::ParseError(err.to_string())
    }
}

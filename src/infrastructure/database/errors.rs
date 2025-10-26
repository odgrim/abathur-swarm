use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Query failed: {0}")]
    QueryFailed(#[from] sqlx::Error),

    #[error("UUID parse error: {0}")]
    UuidParseError(#[from] uuid::Error),

    #[error("DateTime parse error: {0}")]
    DateTimeParseError(#[from] chrono::ParseError),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Session not found: {0}")]
    SessionNotFound(uuid::Uuid),

    #[error("Invalid state update: {0}")]
    InvalidStateUpdate(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Not found: {0}")]
    NotFound(uuid::Uuid),
}

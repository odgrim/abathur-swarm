//! Domain errors for the Abathur swarm system.

use std::fmt;
use thiserror::Error;
use uuid::Uuid;

/// Categorized database error that preserves retry-relevant distinctions.
///
/// This replaces the former `DatabaseError(String)` catch-all, allowing callers
/// to make informed decisions about whether a failure is transient (retry-safe)
/// or permanent (should not be retried with the same input).
#[derive(Debug, Clone)]
pub enum DbErrorCategory {
    /// Connection errors, database busy/locked, timeouts.
    /// Always safe to retry with backoff.
    Transient(String),
    /// Unique constraint violations, foreign key violations.
    /// Never retryable with the same input.
    Constraint(String),
    /// Missing table, missing column — indicates a bug or migration issue.
    /// Never retryable.
    Schema(String),
    /// Fallback for unrecognized errors.
    Unknown(String),
}

impl DbErrorCategory {
    /// Returns `true` if this error category is safe to retry with backoff.
    pub fn is_retryable(&self) -> bool {
        matches!(self, DbErrorCategory::Transient(_))
    }

    /// Returns the inner error message.
    pub fn message(&self) -> &str {
        match self {
            DbErrorCategory::Transient(msg) => msg,
            DbErrorCategory::Constraint(msg) => msg,
            DbErrorCategory::Schema(msg) => msg,
            DbErrorCategory::Unknown(msg) => msg,
        }
    }
}

impl fmt::Display for DbErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbErrorCategory::Transient(msg) => write!(f, "transient: {msg}"),
            DbErrorCategory::Constraint(msg) => write!(f, "constraint: {msg}"),
            DbErrorCategory::Schema(msg) => write!(f, "schema: {msg}"),
            DbErrorCategory::Unknown(msg) => write!(f, "unknown: {msg}"),
        }
    }
}

/// Categorize a `sqlx::Error` into a `DbErrorCategory`.
///
/// This mapping happens at the repository boundary so all code above
/// the repository works with the categorized type.
fn categorize_sqlx_error(err: &sqlx::Error) -> DbErrorCategory {
    match err {
        // Connection-level failures are always transient
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
            DbErrorCategory::Transient(err.to_string())
        }
        // Database-specific errors need further inspection
        sqlx::Error::Database(db_err) => {
            let msg = db_err.message();
            let code = db_err.code().unwrap_or_default();

            // SQLite busy/locked (codes 5, 6) are transient
            if code == "5" || code == "6"
                || msg.contains("database is locked")
                || msg.contains("database is busy")
            {
                DbErrorCategory::Transient(err.to_string())
            }
            // Constraint violations (codes 19, 2067 for UNIQUE, 787 for FK)
            else if code == "19" || code == "2067" || code == "787"
                || msg.contains("UNIQUE constraint")
                || msg.contains("FOREIGN KEY constraint")
                || msg.contains("NOT NULL constraint")
                || msg.contains("CHECK constraint")
            {
                DbErrorCategory::Constraint(err.to_string())
            }
            // Schema errors (code 1 for generic SQL error often means schema issue)
            else if msg.contains("no such table")
                || msg.contains("no such column")
                || msg.contains("has no column")
            {
                DbErrorCategory::Schema(err.to_string())
            } else {
                DbErrorCategory::Unknown(err.to_string())
            }
        }
        // Column not found / decode errors indicate schema mismatch
        sqlx::Error::ColumnNotFound(_) | sqlx::Error::ColumnDecode { .. } => {
            DbErrorCategory::Schema(err.to_string())
        }
        // Row not found is not really an error category — treat as unknown
        sqlx::Error::RowNotFound => DbErrorCategory::Unknown(err.to_string()),
        // Everything else is unknown
        _ => DbErrorCategory::Unknown(err.to_string()),
    }
}

/// Domain-level errors that can occur in the Abathur system.
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("Goal not found: {0}")]
    GoalNotFound(Uuid),

    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("Invalid state transition from {from} to {to}: {reason}")]
    InvalidStateTransition { from: String, to: String, reason: String },

    #[error("Task dependency cycle detected involving task: {0}")]
    DependencyCycle(Uuid),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Memory not found: {0}")]
    MemoryNotFound(Uuid),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(DbErrorCategory),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Concurrency conflict: {entity} {id} was modified")]
    ConcurrencyConflict { entity: String, id: String },

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Task schedule not found: {0}")]
    TaskScheduleNotFound(Uuid),
}

impl DomainError {
    /// Returns `true` if this error is safe to retry with backoff.
    ///
    /// Only transient database errors and concurrency conflicts are retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            DomainError::DatabaseError(cat) => cat.is_retryable(),
            DomainError::ConcurrencyConflict { .. } => true,
            _ => false,
        }
    }
}

pub type DomainResult<T> = Result<T, DomainError>;

impl From<sqlx::Error> for DomainError {
    fn from(err: sqlx::Error) -> Self {
        DomainError::DatabaseError(categorize_sqlx_error(&err))
    }
}

impl From<serde_json::Error> for DomainError {
    fn from(err: serde_json::Error) -> Self {
        DomainError::SerializationError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_error_category_display() {
        let transient = DbErrorCategory::Transient("connection refused".into());
        assert!(transient.to_string().contains("transient"));
        assert!(transient.to_string().contains("connection refused"));

        let constraint = DbErrorCategory::Constraint("UNIQUE constraint failed".into());
        assert!(constraint.to_string().contains("constraint"));

        let schema = DbErrorCategory::Schema("no such table: foo".into());
        assert!(schema.to_string().contains("schema"));

        let unknown = DbErrorCategory::Unknown("something weird".into());
        assert!(unknown.to_string().contains("unknown"));
    }

    #[test]
    fn test_db_error_category_is_retryable() {
        assert!(DbErrorCategory::Transient("busy".into()).is_retryable());
        assert!(!DbErrorCategory::Constraint("unique".into()).is_retryable());
        assert!(!DbErrorCategory::Schema("missing table".into()).is_retryable());
        assert!(!DbErrorCategory::Unknown("wat".into()).is_retryable());
    }

    #[test]
    fn test_db_error_category_message() {
        let cat = DbErrorCategory::Transient("pool timed out".into());
        assert_eq!(cat.message(), "pool timed out");
    }

    #[test]
    fn test_domain_error_is_retryable() {
        let db_transient = DomainError::DatabaseError(DbErrorCategory::Transient("busy".into()));
        assert!(db_transient.is_retryable());

        let db_constraint =
            DomainError::DatabaseError(DbErrorCategory::Constraint("unique".into()));
        assert!(!db_constraint.is_retryable());

        let concurrency = DomainError::ConcurrencyConflict {
            entity: "Task".into(),
            id: "123".into(),
        };
        assert!(concurrency.is_retryable());

        let validation = DomainError::ValidationFailed("bad input".into());
        assert!(!validation.is_retryable());
    }

    #[test]
    fn test_sqlx_pool_timeout_is_transient() {
        let err = sqlx::Error::PoolTimedOut;
        let domain_err: DomainError = err.into();
        assert!(domain_err.is_retryable());
        match &domain_err {
            DomainError::DatabaseError(DbErrorCategory::Transient(_)) => {}
            other => panic!("Expected Transient, got: {other:?}"),
        }
    }

    #[test]
    fn test_sqlx_row_not_found_is_not_retryable() {
        let err = sqlx::Error::RowNotFound;
        let domain_err: DomainError = err.into();
        assert!(!domain_err.is_retryable());
    }
}

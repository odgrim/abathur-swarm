//! SQLite database adapters for the Abathur swarm system.

pub mod agent_repository;
pub mod connection;
pub mod event_repository;
pub mod goal_repository;
pub mod memory_repository;
pub mod migrations;
pub mod task_repository;
pub mod trigger_rule_repository;
pub mod worktree_repository;

pub use agent_repository::SqliteAgentRepository;
pub use connection::{create_pool, create_test_pool, verify_connection, ConnectionError, PoolConfig};
pub use event_repository::SqliteEventRepository;
pub use goal_repository::SqliteGoalRepository;
pub use memory_repository::SqliteMemoryRepository;
pub use migrations::{all_embedded_migrations, Migration, MigrationError, Migrator};
pub use task_repository::SqliteTaskRepository;
pub use trigger_rule_repository::SqliteTriggerRuleRepository;
pub use worktree_repository::SqliteWorktreeRepository;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};

/// Parse a UUID string from a SQLite row field.
pub fn parse_uuid(s: &str) -> DomainResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| DomainError::SerializationError(e.to_string()))
}

/// Parse an optional UUID string from a SQLite row field.
pub fn parse_optional_uuid(s: Option<String>) -> DomainResult<Option<Uuid>> {
    s.map(|s| Uuid::parse_str(&s))
        .transpose()
        .map_err(|e| DomainError::SerializationError(e.to_string()))
}

/// Parse an RFC3339 datetime string from a SQLite row field.
pub fn parse_datetime(s: &str) -> DomainResult<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map_err(|e| DomainError::SerializationError(e.to_string()))
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parse an optional RFC3339 datetime string from a SQLite row field.
pub fn parse_optional_datetime(s: Option<String>) -> DomainResult<Option<DateTime<Utc>>> {
    s.map(|s| chrono::DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc)))
        .transpose()
        .map_err(|e| DomainError::SerializationError(e.to_string()))
}

/// Parse a JSON string from a SQLite row field, falling back to the type's default.
pub fn parse_json_or_default<T: serde::de::DeserializeOwned + Default>(s: Option<String>) -> DomainResult<T> {
    s.map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| DomainError::SerializationError(e.to_string()))
        .map(|opt| opt.unwrap_or_default())
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Connection error: {0}")]
    Connection(#[from] ConnectionError),
    #[error("Migration error: {0}")]
    Migration(#[from] MigrationError),
    #[error("Query error: {0}")]
    Query(#[from] sqlx::Error),
}

pub async fn initialize_database(database_url: &str) -> Result<SqlitePool, DatabaseError> {
    let pool = create_pool(database_url, None).await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;
    Ok(pool)
}

pub async fn initialize_default_database() -> Result<SqlitePool, DatabaseError> {
    initialize_database("sqlite:.abathur/abathur.db").await
}

/// Create an in-memory test pool with all migrations applied.
pub async fn create_migrated_test_pool() -> Result<SqlitePool, DatabaseError> {
    let pool = create_test_pool().await?;
    let migrator = Migrator::new(pool.clone());
    migrator.run_embedded_migrations(all_embedded_migrations()).await?;
    Ok(pool)
}

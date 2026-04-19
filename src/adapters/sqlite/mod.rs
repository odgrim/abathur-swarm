//! SQLite database adapters for the Abathur swarm system.

pub mod agent_repository;
pub mod connection;
pub mod event_repository;
pub mod federated_goal_repository;
pub mod goal_repository;
pub mod memory_repository;
pub mod merge_request_repository;
pub mod migrations;
pub mod outbox_repository;
pub mod refinement_repository;
pub mod task_repository;
pub mod task_schedule_repository;
pub mod trajectory_repository;
pub mod tx_context;
pub mod trigger_rule_repository;
pub mod worktree_repository;
pub mod quiet_window_repository;

pub use agent_repository::SqliteAgentRepository;
pub use connection::{create_pool, create_test_pool, verify_connection, ConnectionError, PoolConfig};
pub use event_repository::SqliteEventRepository;
pub use federated_goal_repository::SqliteFederatedGoalRepository;
pub use goal_repository::SqliteGoalRepository;
pub use memory_repository::SqliteMemoryRepository;
pub use merge_request_repository::SqliteMergeRequestRepository;
pub use migrations::{all_embedded_migrations, Migration, MigrationError, Migrator};
pub use outbox_repository::SqliteOutboxRepository;
pub use refinement_repository::SqliteRefinementRepository;
pub use task_repository::SqliteTaskRepository;
pub use trajectory_repository::SqliteTrajectoryRepository;
pub use task_schedule_repository::SqliteTaskScheduleRepository;
pub use trigger_rule_repository::SqliteTriggerRuleRepository;
pub use worktree_repository::SqliteWorktreeRepository;
pub use quiet_window_repository::SqliteQuietWindowRepository;

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

/// Convert a Vec of DB rows into domain objects, skipping (and warning about)
/// any row that fails to deserialize. Intended for list-style queries where a
/// single corrupt row should not poison the entire result set. Single-row
/// getters should use the normal `try_into` path so callers see the error.
pub fn rows_into_lossy<R, T>(rows: Vec<R>, source: &'static str) -> Vec<T>
where
    R: TryInto<T, Error = DomainError>,
{
    rows.into_iter()
        .filter_map(|r| match r.try_into() {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(
                    source = source,
                    error = %e,
                    "skipping corrupt row during list query"
                );
                None
            }
        })
        .collect()
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

/// Insert a minimal task row for FK constraint satisfaction in tests.
pub async fn insert_test_task(pool: &SqlitePool, task_id: uuid::Uuid) {
    sqlx::query("INSERT OR IGNORE INTO tasks (id, title, status) VALUES (?, 'test task', 'pending')")
        .bind(task_id.to_string())
        .execute(pool)
        .await
        .expect("Failed to insert test task");
}

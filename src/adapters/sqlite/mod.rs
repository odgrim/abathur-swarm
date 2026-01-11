//! SQLite database adapters for the Abathur swarm system.

pub mod agent_repository;
pub mod connection;
pub mod goal_repository;
pub mod memory_repository;
pub mod migrations;
pub mod task_repository;
pub mod worktree_repository;

pub use agent_repository::SqliteAgentRepository;
pub use connection::{create_pool, create_test_pool, verify_connection, ConnectionError, PoolConfig};
pub use goal_repository::SqliteGoalRepository;
pub use memory_repository::SqliteMemoryRepository;
pub use migrations::{all_embedded_migrations, Migration, MigrationError, Migrator};
pub use task_repository::SqliteTaskRepository;
pub use worktree_repository::SqliteWorktreeRepository;

use sqlx::SqlitePool;

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

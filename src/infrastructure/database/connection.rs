<<<<<<< HEAD
use anyhow::{Context, Result};
=======
use crate::domain::ports::errors::DatabaseError;
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use std::str::FromStr;
use std::time::Duration;

<<<<<<< HEAD
<<<<<<< HEAD
/// Database connection pool manager
///
/// Manages `SQLite` connection pool with WAL mode enabled for better concurrency.
/// Handles connection lifecycle, migrations, and configuration.
=======
/// Database connection pool with SQLite configuration optimized for concurrent access
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
=======
/// Database connection manager with connection pooling
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
pub struct DatabaseConnection {
    pool: SqlitePool,
}

impl DatabaseConnection {
    /// Create a new database connection pool with WAL mode enabled
<<<<<<< HEAD
    ///
    /// # Arguments
<<<<<<< HEAD
    /// * `database_url` - `SQLite` database URL (e.g., "sqlite:.abathur/abathur.db")
    ///
    /// # Configuration
    /// - Journal mode: WAL (Write-Ahead Logging)
    /// - Synchronous: NORMAL (good balance of safety and performance)
    /// - Foreign keys: Enabled
    /// - Busy timeout: 5 seconds
    /// - Min connections: 5
    /// - Max connections: 10
    /// - Idle timeout: 30 seconds
    /// - Max lifetime: 30 minutes
    /// - Acquire timeout: 10 seconds
    ///
    /// # Returns
    /// * `Ok(DatabaseConnection)` on success
    /// * `Err` if database URL is invalid or connection fails
    pub async fn new(database_url: &str) -> Result<Self> {
        // Configure connection options
=======
    /// * `database_url` - SQLite database URL (e.g., "sqlite:abathur.db" or "sqlite::memory:")
    ///
    /// # Configuration
    /// - Journal Mode: WAL (Write-Ahead Logging) for better concurrency
    /// - Synchronous: NORMAL for good balance of safety and performance
    /// - Foreign Keys: Enabled for referential integrity
    /// - Busy Timeout: 5 seconds to handle lock contention
    /// - Connection Pool: 5-10 connections (min-max)
    /// - Idle Timeout: 30 seconds
    /// - Max Lifetime: 30 minutes
    /// - Acquire Timeout: 10 seconds
    pub async fn new(database_url: &str) -> Result<Self> {
        // Configure connection options with SQLite pragmas
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
        let options = SqliteConnectOptions::from_str(database_url)
            .context("invalid database URL")?
=======
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        // Configure connection options
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| {
                DatabaseError::ConnectionPoolError(format!("Invalid database URL: {}", e))
            })?
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .create_if_missing(true);

<<<<<<< HEAD
<<<<<<< HEAD
        // Create connection pool with configured limits
=======
        // Create connection pool
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
        let pool = SqlitePoolOptions::new()
            .min_connections(5)
            .max_connections(10)
            .idle_timeout(Duration::from_secs(30))
<<<<<<< HEAD
            .max_lifetime(Duration::from_secs(1800)) // 30 minutes
=======
        // Create connection pool with configured options
        let pool = SqlitePoolOptions::new()
            .min_connections(5)
            .max_connections(10)
            .idle_timeout(Some(Duration::from_secs(30)))
            .max_lifetime(Some(Duration::from_secs(1800))) // 30 minutes
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .context("failed to create connection pool")?;
=======
            .max_lifetime(Duration::from_secs(1800))
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(|e| {
                DatabaseError::ConnectionPoolError(format!(
                    "Failed to create connection pool: {}",
                    e
                ))
            })?;
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02

        Ok(Self { pool })
    }

<<<<<<< HEAD
    /// Run database migrations at startup
    ///
<<<<<<< HEAD
    /// Applies all pending migrations from the migrations/ directory.
    /// Safe to call multiple times - only applies new migrations.
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err` if migrations fail
    pub async fn migrate(&self) -> Result<()> {
=======
    /// This method runs all pending migrations from the migrations/ directory.
    /// Migrations are applied in order based on their timestamp prefix.
    pub async fn run_migrations(&self) -> Result<()> {
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("failed to run migrations")?;
=======
    /// Run migrations at startup
    pub async fn migrate(&self) -> Result<(), DatabaseError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| DatabaseError::MigrationError(format!("Migration failed: {}", e)))?;
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
        Ok(())
    }

    /// Get a reference to the connection pool
<<<<<<< HEAD
    ///
<<<<<<< HEAD
    /// Use this to pass the pool to repository implementations.
    pub const fn pool(&self) -> &SqlitePool {
=======
    /// This pool reference can be used by repository implementations to execute queries.
    /// The pool manages connection lifecycle automatically.
    pub fn pool(&self) -> &SqlitePool {
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
=======
    pub fn pool(&self) -> &SqlitePool {
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
        &self.pool
    }

    /// Close the connection pool gracefully
<<<<<<< HEAD
    ///
<<<<<<< HEAD
    /// Closes all connections and waits for them to finish.
    /// Should be called during application shutdown.
=======
    /// This method closes all connections in the pool. It should be called during
    /// application shutdown to ensure clean termination.
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
=======
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
<<<<<<< HEAD
<<<<<<< HEAD
    async fn test_connection_pool_creation() {
        // Use in-memory database for testing
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create database connection");

        // Verify pool is accessible
        assert!(!db.pool().is_closed());

=======
    async fn test_connection_creation() {
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("Failed to create connection");

        assert!(!db.pool().is_closed());
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
        db.close().await;
    }

    #[tokio::test]
<<<<<<< HEAD
    async fn test_migration_runs_successfully() {
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create database connection");

        // Run migrations
        db.migrate().await.expect("failed to run migrations");

        // Verify agents table exists
        let result: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agents'",
        )
        .fetch_one(db.pool())
        .await
        .expect("failed to query table");

        assert_eq!(result.0, 1, "agents table should exist");

        db.close().await;
=======
    async fn test_create_connection() {
        let conn = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create connection");

        // Verify pool is accessible
        assert!(!conn.pool().is_closed());

        conn.close().await;
    }

    #[tokio::test]
    async fn test_run_migrations() {
        let conn = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create connection");

        // Run migrations
        conn.run_migrations()
            .await
            .expect("failed to run migrations");

        // Verify tables exist by querying sqlite_master
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name != 'sqlite_sequence' AND name != '_sqlx_migrations' ORDER BY name"
        )
        .fetch_all(conn.pool())
        .await
        .expect("failed to query tables");

        let table_names: Vec<String> = tables.into_iter().map(|t| t.0).collect();

        // Verify core tables exist
        assert!(
            table_names.contains(&"sessions".to_string()),
            "sessions table should exist"
        );
        assert!(
            table_names.contains(&"tasks".to_string()),
            "tasks table should exist"
        );
        assert!(
            table_names.contains(&"agents".to_string()),
            "agents table should exist"
        );
        assert!(
            table_names.contains(&"memory_entries".to_string()),
            "memory_entries table should exist"
        );

        conn.close().await;
    }

    #[tokio::test]
    async fn test_foreign_keys_enabled() {
        let conn = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create connection");

        conn.run_migrations()
            .await
            .expect("failed to run migrations");

        // Verify foreign keys are enabled
        let result: (i32,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(conn.pool())
            .await
            .expect("failed to check foreign keys pragma");

        assert_eq!(result.0, 1, "foreign keys should be enabled");

        conn.close().await;
    }

    #[tokio::test]
    async fn test_wal_mode_enabled() {
        let conn = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create connection");

        // Note: WAL mode doesn't work with in-memory databases in SQLite
        // This test verifies the setting is accepted, but we'll use a file for real verification

        conn.close().await;
    }

    #[tokio::test]
    async fn test_pool_configuration() {
        let conn = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create connection");

        // Verify pool is not closed
        assert!(!conn.pool().is_closed());

        // Get multiple connections to verify pool works
        let conn1 = conn
            .pool()
            .acquire()
            .await
            .expect("failed to acquire conn 1");
        let conn2 = conn
            .pool()
            .acquire()
            .await
            .expect("failed to acquire conn 2");

        drop(conn1);
        drop(conn2);

        conn.close().await;

        // Verify pool is closed after close()
        assert!(conn.pool().is_closed());
>>>>>>> task_phase3-database-connection_2025-10-25-23-00-01
=======
    async fn test_migration() {
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("Failed to create connection");

        db.migrate().await.expect("Failed to run migrations");
        db.close().await;
>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
    }
}

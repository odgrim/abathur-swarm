use anyhow::{Context, Result};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use std::str::FromStr;
use std::time::Duration;

/// Database connection pool manager
///
/// Manages `SQLite` connection pool with WAL mode enabled for better concurrency.
/// Handles connection lifecycle, migrations, and configuration.
pub struct DatabaseConnection {
    pool: SqlitePool,
}

impl DatabaseConnection {
    /// Create a new database connection pool with WAL mode enabled
    ///
    /// # Arguments
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
        let options = SqliteConnectOptions::from_str(database_url)
            .context("invalid database URL")?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .create_if_missing(true);

        // Create connection pool with configured limits
        let pool = SqlitePoolOptions::new()
            .min_connections(5)
            .max_connections(10)
            .idle_timeout(Duration::from_secs(30))
            .max_lifetime(Duration::from_secs(1800)) // 30 minutes
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .context("failed to create connection pool")?;

        Ok(Self { pool })
    }

    /// Run database migrations at startup
    ///
    /// Applies all pending migrations from the migrations/ directory.
    /// Safe to call multiple times - only applies new migrations.
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err` if migrations fail
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("failed to run migrations")?;
        Ok(())
    }

    /// Get a reference to the connection pool
    ///
    /// Use this to pass the pool to repository implementations.
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the connection pool gracefully
    ///
    /// Closes all connections and waits for them to finish.
    /// Should be called during application shutdown.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_pool_creation() {
        // Use in-memory database for testing
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("failed to create database connection");

        // Verify pool is accessible
        assert!(!db.pool().is_closed());

        db.close().await;
    }

    #[tokio::test]
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
    }
}

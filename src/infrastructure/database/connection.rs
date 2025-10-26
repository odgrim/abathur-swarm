use crate::domain::ports::errors::DatabaseError;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use std::str::FromStr;
use std::time::Duration;

/// Database connection manager with connection pooling
pub struct DatabaseConnection {
    pool: SqlitePool,
}

impl DatabaseConnection {
    /// Create a new database connection pool with WAL mode enabled
    pub async fn new(database_url: &str) -> Result<Self, DatabaseError> {
        // Configure connection options
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| {
                DatabaseError::ConnectionPoolError(format!("Invalid database URL: {}", e))
            })?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .create_if_missing(true);

        // Create connection pool
        let pool = SqlitePoolOptions::new()
            .min_connections(5)
            .max_connections(10)
            .idle_timeout(Duration::from_secs(30))
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

        Ok(Self { pool })
    }

    /// Run migrations at startup
    pub async fn migrate(&self) -> Result<(), DatabaseError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| DatabaseError::MigrationError(format!("Migration failed: {}", e)))?;
        Ok(())
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the connection pool gracefully
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_creation() {
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("Failed to create connection");

        assert!(!db.pool().is_closed());
        db.close().await;
    }

    #[tokio::test]
    async fn test_migration() {
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("Failed to create connection");

        db.migrate().await.expect("Failed to run migrations");
        db.close().await;
    }
}

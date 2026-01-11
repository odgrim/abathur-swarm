//! SQLite database connection pool management.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("Failed to create pool: {0}")]
    PoolCreationFailed(#[source] sqlx::Error),
    #[error("Invalid database URL: {0}")]
    InvalidDatabaseUrl(String),
    #[error("Failed to create directory: {0}")]
    DirectoryCreationFailed(#[source] std::io::Error),
    #[error("Connection failed: {0}")]
    ConnectionFailed(#[source] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 5,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(3),
        }
    }
}

pub async fn create_pool(database_url: &str, config: Option<PoolConfig>) -> Result<SqlitePool, ConnectionError> {
    let config = config.unwrap_or_default();
    ensure_database_directory(database_url)?;

    let connect_options = SqliteConnectOptions::from_str(database_url)
        .map_err(|_| ConnectionError::InvalidDatabaseUrl(database_url.to_string()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(30));

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect_with(connect_options)
        .await
        .map_err(ConnectionError::PoolCreationFailed)?;

    Ok(pool)
}

pub async fn create_test_pool() -> Result<SqlitePool, ConnectionError> {
    let connect_options = SqliteConnectOptions::from_str("sqlite::memory:")
        .map_err(|_| ConnectionError::InvalidDatabaseUrl("sqlite::memory:".to_string()))?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .shared_cache(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .map_err(ConnectionError::PoolCreationFailed)
}

fn ensure_database_directory(database_url: &str) -> Result<(), ConnectionError> {
    let path = database_url
        .strip_prefix("sqlite:")
        .or_else(|| database_url.strip_prefix("sqlite://"))
        .unwrap_or(database_url);

    if path == ":memory:" || path.is_empty() {
        return Ok(());
    }

    let path = Path::new(path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(ConnectionError::DirectoryCreationFailed)?;
        }
    }
    Ok(())
}

pub async fn verify_connection(pool: &SqlitePool) -> Result<(), ConnectionError> {
    sqlx::query("SELECT 1").fetch_one(pool).await.map_err(ConnectionError::ConnectionFailed)?;
    Ok(())
}

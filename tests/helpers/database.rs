use abathur_cli::infrastructure::database::DatabaseConnection;
use sqlx::SqlitePool;

/// Create an in-memory `SQLite` database for testing with vec0 extension loaded
#[allow(dead_code)]
pub async fn setup_test_db() -> SqlitePool {
    let db_conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create test database");

    // Run migrations
    db_conn
        .migrate()
        .await
        .expect("failed to run migrations");

    db_conn.pool().clone()
}

/// Teardown test database
#[allow(dead_code)]
pub async fn teardown_test_db(pool: SqlitePool) {
    pool.close().await;
}

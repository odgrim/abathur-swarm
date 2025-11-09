use sqlx::SqlitePool;

/// Create an in-memory `SQLite` database for testing
pub async fn setup_test_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to create test database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");

    pool
}

/// Teardown test database
#[allow(dead_code)]
pub async fn teardown_test_db(pool: SqlitePool) {
    pool.close().await;
}

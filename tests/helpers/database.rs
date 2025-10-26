use sqlx::SqlitePool;

<<<<<<< HEAD
/// Create an in-memory `SQLite` database for testing
=======
/// Create an in-memory SQLite database for testing
///
/// Creates a fresh in-memory database with migrations applied.
/// Each call creates a completely isolated database instance.
///
/// # Example
/// ```
/// use tests::helpers::database::setup_test_db;
///
/// #[tokio::test]
/// async fn my_test() {
///     let pool = setup_test_db().await;
///     // Use pool for testing...
///     teardown_test_db(pool).await;
/// }
/// ```
#[allow(dead_code)]
>>>>>>> task_phase3-agent-repository_2025-10-25-23-00-03
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
<<<<<<< HEAD
=======
///
/// Closes the connection pool and cleans up resources.
/// Always call this at the end of your test to avoid resource leaks.
#[allow(dead_code)]
>>>>>>> task_phase3-agent-repository_2025-10-25-23-00-03
pub async fn teardown_test_db(pool: SqlitePool) {
    pool.close().await;
}

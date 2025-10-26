use abathur::DatabaseConnection;
use sqlx::Row;

#[tokio::test]
async fn test_database_connection_lifecycle() {
    // Create connection
    let conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create connection");

    // Run migrations
    conn.run_migrations()
        .await
        .expect("failed to run migrations");

    // Verify connection is active
    assert!(!conn.pool().is_closed());

    // Close connection
    conn.close().await;

    // Verify connection is closed
    assert!(conn.pool().is_closed());
}

#[tokio::test]
async fn test_migrations_create_all_tables() {
    let conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create connection");

    conn.run_migrations()
        .await
        .expect("failed to run migrations");

    // Query all table names
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name"
    )
    .fetch_all(conn.pool())
    .await
    .expect("failed to query tables");

    let table_names: Vec<String> = rows.iter().map(|row| row.get("name")).collect();

    // Verify all expected tables exist
    let expected_tables = vec![
        "agents",
        "audit",
        "checkpoints",
        "document_index",
        "memory_entries",
        "metrics",
        "sessions",
        "state",
        "tasks",
    ];

    for table in expected_tables {
        assert!(
            table_names.contains(&table.to_string()),
            "table {} should exist",
            table
        );
    }

    conn.close().await;
}

#[tokio::test]
async fn test_indexes_created() {
    let conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create connection");

    conn.run_migrations()
        .await
        .expect("failed to run migrations");

    // Query all indexes
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )
    .fetch_all(conn.pool())
    .await
    .expect("failed to query indexes");

    let index_count = rows.len();

    // Verify we have a substantial number of indexes (should be ~30+)
    assert!(
        index_count >= 20,
        "should have at least 20 indexes, found {}",
        index_count
    );

    conn.close().await;
}

#[tokio::test]
async fn test_foreign_keys_enforced() {
    let conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create connection");

    conn.run_migrations()
        .await
        .expect("failed to run migrations");

    // Verify foreign keys pragma is enabled
    let row: (i32,) = sqlx::query_as("PRAGMA foreign_keys")
        .fetch_one(conn.pool())
        .await
        .expect("failed to check foreign keys");

    assert_eq!(row.0, 1, "foreign keys should be enabled");

    // Try to insert a task with invalid session_id (should fail due to FK constraint)
    let result = sqlx::query(
        r#"
        INSERT INTO tasks (id, prompt, agent_type, priority, status, input_data, submitted_at, last_updated_at, session_id)
        VALUES ('task-1', 'test', 'general', 5, 'pending', '{}', '2025-10-25T00:00:00Z', '2025-10-25T00:00:00Z', 'invalid-session-id')
        "#
    )
    .execute(conn.pool())
    .await;

    // Should fail due to foreign key constraint
    assert!(
        result.is_err(),
        "inserting task with invalid session_id should fail"
    );

    conn.close().await;
}

#[tokio::test]
async fn test_json_validation_constraints() {
    let conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create connection");

    conn.run_migrations()
        .await
        .expect("failed to run migrations");

    // Try to insert a session with invalid JSON for events
    let result = sqlx::query(
        r#"
        INSERT INTO sessions (id, app_name, user_id, events, state, metadata, created_at, last_update_time)
        VALUES ('sess-1', 'abathur', 'user-1', 'invalid json', '{}', '{}', '2025-10-25T00:00:00Z', '2025-10-25T00:00:00Z')
        "#
    )
    .execute(conn.pool())
    .await;

    // Should fail due to JSON validation constraint
    assert!(
        result.is_err(),
        "inserting session with invalid JSON should fail"
    );

    // Valid JSON should succeed
    let result = sqlx::query(
        r#"
        INSERT INTO sessions (id, app_name, user_id, events, state, metadata, created_at, last_update_time)
        VALUES ('sess-1', 'abathur', 'user-1', '[]', '{}', '{}', '2025-10-25T00:00:00Z', '2025-10-25T00:00:00Z')
        "#
    )
    .execute(conn.pool())
    .await;

    assert!(
        result.is_ok(),
        "inserting session with valid JSON should succeed"
    );

    conn.close().await;
}

#[tokio::test]
async fn test_connection_pool_concurrency() {
    let conn = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("failed to create connection");

    conn.run_migrations()
        .await
        .expect("failed to run migrations");

    // Spawn multiple concurrent tasks that acquire connections
    let mut handles = vec![];

    for i in 0..5 {
        let pool = conn.pool().clone();
        let handle = tokio::spawn(async move {
            let _acquired = pool.acquire().await.expect("failed to acquire connection");
            // Simulate some work
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            i
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.expect("task should complete successfully");
    }

    conn.close().await;
}

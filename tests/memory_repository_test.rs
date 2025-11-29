use abathur_cli::domain::models::{Memory, MemoryType};
use abathur_cli::domain::ports::MemoryRepository;
use abathur_cli::infrastructure::database::MemoryRepositoryImpl;
use serde_json::json;
use sqlx::SqlitePool;

async fn setup_test_db() -> SqlitePool {
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

#[tokio::test]
async fn test_memory_crud_operations() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    // Create
    let memory = Memory::new(
        "user:test:settings".to_string(),
        "theme".to_string(),
        json!({"mode": "dark", "font": "monospace"}),
        MemoryType::Semantic,
        "test_user".to_string(),
    );

    repo.insert(memory.clone())
        .await
        .expect("failed to add memory");

    // Read
    let retrieved = repo
        .get("user:test:settings", "theme")
        .await
        .expect("failed to get memory")
        .expect("memory not found");

    assert_eq!(retrieved.namespace, "user:test:settings");
    assert_eq!(retrieved.key, "theme");
    assert_eq!(retrieved.value, json!({"mode": "dark", "font": "monospace"}));
    assert_eq!(retrieved.memory_type, MemoryType::Semantic);
    assert!(!retrieved.is_deleted);

    // Update
    repo.update(
        "user:test:settings",
        "theme",
        json!({"mode": "light", "font": "monospace"}),
        "test_user",
    )
    .await
    .expect("failed to update memory");

    let updated = repo
        .get("user:test:settings", "theme")
        .await
        .expect("failed to get updated memory")
        .expect("memory not found");

    assert_eq!(updated.value, json!({"mode": "light", "font": "monospace"}));

    // Delete
    repo.delete("user:test:settings", "theme")
        .await
        .expect("failed to delete memory");

    let deleted = repo
        .get("user:test:settings", "theme")
        .await
        .expect("failed to get deleted memory");

    assert!(deleted.is_none());

    pool.close().await;
}

#[tokio::test]
async fn test_namespace_prefix_search() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    // Add memories with hierarchical namespaces
    let test_data = vec![
        ("user:alice:preferences:ui", "theme", "dark"),
        ("user:alice:preferences:ui", "font", "monospace"),
        ("user:alice:preferences:privacy", "tracking", "disabled"),
        ("user:alice:history", "last_login", "2025-10-25"),
        ("user:bob:preferences:ui", "theme", "light"),
    ];

    for (namespace, key, value) in test_data {
        let memory = Memory::new(
            namespace.to_string(),
            key.to_string(),
            json!(value),
            MemoryType::Semantic,
            "test".to_string(),
        );
        repo.insert(memory).await.expect("failed to add memory");
    }

    // Search for all alice's preferences
    let alice_prefs = repo
        .search("user:alice:preferences", None, 100)
        .await
        .expect("failed to search");

    assert_eq!(alice_prefs.len(), 3);

    // Search for all alice's ui preferences
    let alice_ui = repo
        .search("user:alice:preferences:ui", None, 100)
        .await
        .expect("failed to search");

    assert_eq!(alice_ui.len(), 2);

    // Search for all alice's memories
    let alice_all = repo
        .search("user:alice", None, 100)
        .await
        .expect("failed to search");

    assert_eq!(alice_all.len(), 4);

    pool.close().await;
}

#[tokio::test]
async fn test_memory_type_filtering() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    // Add memories of different types
    let semantic = Memory::new(
        "knowledge:facts".to_string(),
        "earth_radius".to_string(),
        json!(6371),
        MemoryType::Semantic,
        "system".to_string(),
    );

    let episodic = Memory::new(
        "knowledge:events".to_string(),
        "user_login_2025_10_25".to_string(),
        json!({"timestamp": "2025-10-25T10:00:00Z"}),
        MemoryType::Episodic,
        "system".to_string(),
    );

    let procedural = Memory::new(
        "knowledge:procedures".to_string(),
        "login_flow".to_string(),
        json!(["authenticate", "fetch_profile", "load_preferences"]),
        MemoryType::Procedural,
        "system".to_string(),
    );

    repo.insert(semantic).await.expect("failed to add semantic");
    repo.insert(episodic).await.expect("failed to add episodic");
    repo.insert(procedural)
        .await
        .expect("failed to add procedural");

    // Search for semantic memories only
    let semantic_results = repo
        .search("knowledge:", Some(MemoryType::Semantic), 100)
        .await
        .expect("failed to search");

    assert_eq!(semantic_results.len(), 1);
    assert_eq!(semantic_results[0].memory_type, MemoryType::Semantic);

    // Search for episodic memories only
    let episodic_results = repo
        .search("knowledge:", Some(MemoryType::Episodic), 100)
        .await
        .expect("failed to search");

    assert_eq!(episodic_results.len(), 1);
    assert_eq!(episodic_results[0].memory_type, MemoryType::Episodic);

    // Search for all types
    let all_results = repo
        .search("knowledge:", None, 100)
        .await
        .expect("failed to search");

    assert_eq!(all_results.len(), 3);

    pool.close().await;
}

#[tokio::test]
async fn test_soft_delete_behavior() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    let memory = Memory::new(
        "test:namespace".to_string(),
        "test_key".to_string(),
        json!({"value": "test"}),
        MemoryType::Semantic,
        "test".to_string(),
    );

    repo.insert(memory).await.expect("failed to add memory");

    // Soft delete
    repo.delete("test:namespace", "test_key")
        .await
        .expect("failed to delete");

    // get() should not return deleted memory
    let result = repo
        .get("test:namespace", "test_key")
        .await
        .expect("failed to query");
    assert!(result.is_none());

    // search() should not return deleted memory
    let search_results = repo
        .search("test:", None, 100)
        .await
        .expect("failed to search");
    assert_eq!(search_results.len(), 0);

    // Verify row still exists in database (soft delete)
    let row = sqlx::query!(
        "SELECT is_deleted FROM memories WHERE namespace = ? AND key = ?",
        "test:namespace",
        "test_key"
    )
    .fetch_optional(&pool)
    .await
    .expect("failed to query database");

    assert!(row.is_some());
    assert_eq!(row.unwrap().is_deleted, 1);

    pool.close().await;
}

#[tokio::test]
async fn test_version_increment_on_update() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    let memory = Memory::new(
        "versioning:test".to_string(),
        "counter".to_string(),
        json!(1),
        MemoryType::Semantic,
        "test".to_string(),
    );

    repo.insert(memory).await.expect("failed to add");

    // Multiple updates
    for i in 2..=5 {
        repo.update(
            "versioning:test",
            "counter",
            json!(i),
            "test",
        )
        .await
        .expect("failed to update");

        let current = repo
            .get("versioning:test", "counter")
            .await
            .expect("failed to get")
            .unwrap();

        assert_eq!(current.value, json!(i));
    }

    pool.close().await;
}

#[tokio::test]
async fn test_unique_namespace_key_constraint() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    let memory1 = Memory::new(
        "unique:test".to_string(),
        "key".to_string(),
        json!("value1"),
        MemoryType::Semantic,
        "test".to_string(),
    );

    let memory2 = Memory::new(
        "unique:test".to_string(),
        "key".to_string(),
        json!("value2"),
        MemoryType::Semantic,
        "test".to_string(),
    );

    repo.insert(memory1).await.expect("failed to add first");

    // Duplicate namespace+key should fail
    let result = repo.insert(memory2).await;
    assert!(result.is_err());

    pool.close().await;
}

#[tokio::test]
async fn test_update_nonexistent_memory_fails() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    let result = repo
        .update(
            "nonexistent:namespace",
            "nonexistent_key",
            json!({}),
            "test",
        )
        .await;

    assert!(result.is_err());

    pool.close().await;
}

#[tokio::test]
async fn test_delete_nonexistent_memory_fails() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    let result = repo
        .delete("nonexistent:namespace", "nonexistent_key")
        .await;

    assert!(result.is_err());

    pool.close().await;
}

#[tokio::test]
async fn test_metadata_persistence() {
    let pool = setup_test_db().await;
    let repo = MemoryRepositoryImpl::new(pool.clone());

    let memory = Memory::with_metadata(
        "meta:test".to_string(),
        "data".to_string(),
        json!({"content": "value"}),
        MemoryType::Semantic,
        json!({"source": "api", "priority": "high"}),
        "system".to_string(),
    );

    repo.insert(memory).await.expect("failed to add");

    let retrieved = repo
        .get("meta:test", "data")
        .await
        .expect("failed to get")
        .unwrap();

    assert!(retrieved.metadata.is_some());
    assert_eq!(
        retrieved.metadata.unwrap(),
        json!({"source": "api", "priority": "high"})
    );

    pool.close().await;
}

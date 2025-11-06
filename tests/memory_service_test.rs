mod helpers;

use abathur::domain::models::{Memory, MemoryType};
use abathur::domain::ports::MemoryRepository;
use abathur::infrastructure::database::MemoryRepositoryImpl;
use abathur::services::MemoryService;
use serde_json::json;
use std::sync::Arc;

use helpers::database::{setup_test_db, teardown_test_db};

#[tokio::test]
async fn test_add_memory() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let memory = Memory::new(
        "test:user:alice".to_string(),
        "preferences".to_string(),
        json!({"theme": "dark", "language": "en"}),
        MemoryType::Semantic,
        "alice".to_string(),
    );

    let result = service.add(memory.clone()).await;
    assert!(result.is_ok());

    // Verify we can retrieve it
    let retrieved = service
        .get("test:user:alice", "preferences")
        .await
        .expect("failed to get memory");

    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(
        retrieved.value,
        json!({"theme": "dark", "language": "en"})
    );
    assert_eq!(retrieved.memory_type, MemoryType::Semantic);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_add_duplicate_memory_fails() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let memory = Memory::new(
        "test:duplicate".to_string(),
        "key".to_string(),
        json!({"value": 1}),
        MemoryType::Semantic,
        "test".to_string(),
    );

    // First insert should succeed
    service
        .add(memory.clone())
        .await
        .expect("first add should succeed");

    // Second insert should fail
    let result = service.add(memory).await;
    assert!(result.is_err());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_nonexistent_memory() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let result = service.get("nonexistent", "key").await.expect("query should not fail");
    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_memory() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add initial memory
    let memory = Memory::new(
        "test:update".to_string(),
        "settings".to_string(),
        json!({"version": 1}),
        MemoryType::Semantic,
        "test".to_string(),
    );

    service.add(memory).await.expect("failed to add memory");

    // Update memory
    service
        .update("test:update", "settings", json!({"version": 2}), "test")
        .await
        .expect("failed to update memory");

    // Verify update
    let retrieved = service
        .get("test:update", "settings")
        .await
        .expect("failed to get memory")
        .unwrap();

    assert_eq!(retrieved.value, json!({"version": 2}));
    assert_eq!(retrieved.version, 2);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_nonexistent_memory_fails() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let result = service
        .update("nonexistent", "key", json!({"value": 1}), "test")
        .await;

    assert!(result.is_err());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_delete_memory() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add memory
    let memory = Memory::new(
        "test:delete".to_string(),
        "temporary".to_string(),
        json!({"data": "value"}),
        MemoryType::Episodic,
        "test".to_string(),
    );

    service.add(memory).await.expect("failed to add memory");

    // Delete memory
    service
        .delete("test:delete", "temporary")
        .await
        .expect("failed to delete memory");

    // Verify deletion (should not be retrievable)
    let result = service
        .get("test:delete", "temporary")
        .await
        .expect("query should not fail");

    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_search_by_namespace_prefix() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add multiple memories with hierarchical namespaces
    let memories = vec![
        Memory::new(
            "test:user:alice:preferences".to_string(),
            "theme".to_string(),
            json!("dark"),
            MemoryType::Semantic,
            "alice".to_string(),
        ),
        Memory::new(
            "test:user:alice:preferences".to_string(),
            "language".to_string(),
            json!("en"),
            MemoryType::Semantic,
            "alice".to_string(),
        ),
        Memory::new(
            "test:user:alice:history".to_string(),
            "last_login".to_string(),
            json!("2024-01-01"),
            MemoryType::Episodic,
            "alice".to_string(),
        ),
        Memory::new(
            "test:user:bob:preferences".to_string(),
            "theme".to_string(),
            json!("light"),
            MemoryType::Semantic,
            "bob".to_string(),
        ),
    ];

    for memory in memories {
        service.add(memory).await.expect("failed to add memory");
    }

    // Search for alice's preferences
    let alice_prefs = service
        .search("test:user:alice:preferences", None, None)
        .await
        .expect("failed to search");

    assert_eq!(alice_prefs.len(), 2);

    // Search for all alice's memories
    let alice_all = service
        .search("test:user:alice", None, None)
        .await
        .expect("failed to search");

    assert_eq!(alice_all.len(), 3);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_search_by_memory_type() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add memories of different types
    let semantic = Memory::new(
        "test:knowledge".to_string(),
        "fact1".to_string(),
        json!("Paris is the capital of France"),
        MemoryType::Semantic,
        "test".to_string(),
    );

    let episodic = Memory::new(
        "test:events".to_string(),
        "event1".to_string(),
        json!({"timestamp": "2024-01-01", "action": "login"}),
        MemoryType::Episodic,
        "test".to_string(),
    );

    let procedural = Memory::new(
        "test:procedures".to_string(),
        "proc1".to_string(),
        json!(["step1", "step2", "step3"]),
        MemoryType::Procedural,
        "test".to_string(),
    );

    service.add(semantic).await.expect("failed to add semantic");
    service.add(episodic).await.expect("failed to add episodic");
    service.add(procedural).await.expect("failed to add procedural");

    // Search for semantic memories only
    let semantic_results = service
        .search("test:", Some(MemoryType::Semantic), None)
        .await
        .expect("failed to search");

    assert_eq!(semantic_results.len(), 1);
    assert_eq!(semantic_results[0].memory_type, MemoryType::Semantic);

    // Search for episodic memories only
    let episodic_results = service
        .search("test:", Some(MemoryType::Episodic), None)
        .await
        .expect("failed to search");

    assert_eq!(episodic_results.len(), 1);
    assert_eq!(episodic_results[0].memory_type, MemoryType::Episodic);

    // Search for all types
    let all_results = service
        .search("test:", None, None)
        .await
        .expect("failed to search");

    assert_eq!(all_results.len(), 3);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_search_with_limit() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add multiple memories
    for i in 0..10 {
        let memory = Memory::new(
            "test:items".to_string(),
            format!("item{}", i),
            json!(i),
            MemoryType::Semantic,
            "test".to_string(),
        );
        service.add(memory).await.expect("failed to add memory");
    }

    // Search with limit
    let limited_results = service
        .search("test:items", None, Some(5))
        .await
        .expect("failed to search");

    assert_eq!(limited_results.len(), 5);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_count_memories() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add memories
    for i in 0..7 {
        let memory = Memory::new(
            "test:counter".to_string(),
            format!("key{}", i),
            json!(i),
            MemoryType::Semantic,
            "test".to_string(),
        );
        service.add(memory).await.expect("failed to add memory");
    }

    // Count memories
    let count = service
        .count("test:counter", None)
        .await
        .expect("failed to count");

    assert_eq!(count, 7);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_memory_versioning() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add initial memory
    let memory = Memory::new(
        "test:version".to_string(),
        "counter".to_string(),
        json!(1),
        MemoryType::Semantic,
        "test".to_string(),
    );

    service.add(memory).await.expect("failed to add memory");

    // Update multiple times
    for i in 2..=5 {
        service
            .update("test:version", "counter", json!(i), "test")
            .await
            .expect("failed to update");
    }

    // Verify final version
    let final_memory = service
        .get("test:version", "counter")
        .await
        .expect("failed to get memory")
        .unwrap();

    assert_eq!(final_memory.version, 5);
    assert_eq!(final_memory.value, json!(5));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_memory_with_metadata() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let metadata = json!({"source": "api", "confidence": 0.95});

    let memory = Memory::with_metadata(
        "test:meta".to_string(),
        "data".to_string(),
        json!({"value": "test"}),
        MemoryType::Semantic,
        metadata.clone(),
        "test".to_string(),
    );

    service.add(memory).await.expect("failed to add memory");

    let retrieved = service
        .get("test:meta", "data")
        .await
        .expect("failed to get memory")
        .unwrap();

    assert_eq!(retrieved.metadata, Some(metadata));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_update_deleted_memory_fails() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    // Add and then delete memory
    let memory = Memory::new(
        "test:deleted".to_string(),
        "key".to_string(),
        json!("value"),
        MemoryType::Semantic,
        "test".to_string(),
    );

    service.add(memory).await.expect("failed to add memory");
    service
        .delete("test:deleted", "key")
        .await
        .expect("failed to delete memory");

    // Try to update deleted memory
    let result = service
        .update("test:deleted", "key", json!("new value"), "test")
        .await;

    assert!(result.is_err());

    teardown_test_db(pool).await;
}

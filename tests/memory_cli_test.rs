use abathur_cli::{
    domain::models::{Memory, MemoryType},
    infrastructure::database::{connection::DatabaseConnection, memory_repo::MemoryRepositoryImpl},
    services::MemoryService,
};
use anyhow::Result;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_memory_service_integration() -> Result<()> {
    // Load configuration and connect to database
    let config = abathur_cli::infrastructure::config::ConfigLoader::load()?;
    let database_url = format!("sqlite:{}", config.database.path);
    let db = DatabaseConnection::new(&database_url).await?;

    // Run migrations
    db.migrate().await?;

    // Create repository and service
    let repo = Arc::new(MemoryRepositoryImpl::new(db.pool().clone()));
    let service = MemoryService::new(repo);

    // Add test memory
    let memory = Memory::new(
        "test:user:alice".to_string(),
        "preferences".to_string(),
        json!({"theme": "dark", "language": "en"}),
        MemoryType::Semantic,
        "alice".to_string(),
    );

    service.add(memory).await?;

    // Add another memory
    let memory2 = Memory::new(
        "test:user:bob".to_string(),
        "settings".to_string(),
        json!({"notifications": true}),
        MemoryType::Semantic,
        "bob".to_string(),
    );

    service.add(memory2).await?;

    // Add an episodic memory
    let memory3 = Memory::new(
        "test:session:123".to_string(),
        "event".to_string(),
        json!({"action": "login", "timestamp": "2024-01-01"}),
        MemoryType::Episodic,
        "system".to_string(),
    );

    service.add(memory3).await?;

    // Test search
    let results = service.search("test:user", None, None).await?;
    assert_eq!(results.len(), 2);

    // Test search with type filter
    let semantic_results = service.search("test:", Some(MemoryType::Semantic), None).await?;
    assert_eq!(semantic_results.len(), 2);

    let episodic_results = service.search("test:", Some(MemoryType::Episodic), None).await?;
    assert_eq!(episodic_results.len(), 1);

    // Test get
    let retrieved = service.get("test:user:alice", "preferences").await?;
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().value, json!({"theme": "dark", "language": "en"}));

    // Test count
    let count = service.count("test:user", None).await?;
    assert_eq!(count, 2);

    println!("✓ All memory service integration tests passed!");

    Ok(())
}

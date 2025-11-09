/// Integration test for vec0 extension with VectorStore
use abathur_cli::domain::models::EmbeddingModel;
use abathur_cli::infrastructure::database::DatabaseConnection;
use abathur_cli::infrastructure::vector::{LocalEmbeddingService, VectorStore, VectorImplementation};
use std::sync::Arc;

#[tokio::test]
async fn test_vector_store_uses_vec0() {
    // Create database connection (registers vec0 extension)
    let db = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("Failed to create database");

    // Run migrations to create vec0 tables
    db.migrate().await.expect("Migrations should succeed");

    // Create embedding service
    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service")
    );

    // Create vector store
    let vector_store = VectorStore::new(
        Arc::new(db.pool().clone()),
        embedding_service
    )
    .await
    .expect("Failed to create vector store");

    // Verify it's using NativeVec0 implementation
    assert_eq!(
        vector_store.implementation(),
        VectorImplementation::NativeVec0,
        "VectorStore should use NativeVec0 when extension is available"
    );

    // Initialize vector index
    vector_store.create_vector_index()
        .await
        .expect("Failed to create vector index");

    db.close().await;
}

#[tokio::test]
async fn test_vec0_with_real_database() {
    // Use a temporary database file
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}", db_path.display());

    // Create database connection
    let db = DatabaseConnection::new(&db_url)
        .await
        .expect("Failed to create database");

    // Run migrations
    db.migrate().await.expect("Migrations should succeed");

    // Verify vec0 tables exist
    let vec0_table_exists: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='vec_memory_vec0'"
    )
    .fetch_one(db.pool())
    .await
    .expect("Failed to check for vec0 table");

    assert_eq!(vec0_table_exists.0, 1, "vec0 virtual table should exist");

    // Verify we can query vec_version()
    let version: (String,) = sqlx::query_as("SELECT vec_version()")
        .fetch_one(db.pool())
        .await
        .expect("Should be able to query vec_version()");

    println!("sqlite-vec version: {}", version.0);
    assert!(!version.0.is_empty(), "Version should not be empty");

    db.close().await;
}

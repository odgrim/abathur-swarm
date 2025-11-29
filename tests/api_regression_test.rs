//! API Regression Test Suite
//!
//! This test suite ensures 100% backward compatibility of public APIs.
//! These tests verify that the following traits remain unchanged:
//! 1. EmbeddingService trait
//! 2. EmbeddingRepository (VectorStore) trait
//! 3. ChunkingService trait
//! 4. Public API method signatures
//! 5. Existing test compatibility
//!
//! Critical: These tests MUST NOT be modified unless there is a breaking API change.
//! If any test fails, it indicates a breaking change to the public API.

mod helpers;

use abathur_cli::domain::models::{Citation, EmbeddingModel};
use abathur_cli::domain::ports::{EmbeddingRepository, EmbeddingService};
use abathur_cli::infrastructure::vector::{LocalEmbeddingService, VectorStore};
use helpers::database::setup_test_db;
use std::sync::Arc;

/// Regression Test 1: EmbeddingService trait interface unchanged
///
/// This test verifies that the EmbeddingService trait maintains its contract:
/// - async fn embed(&self, text: &str) -> Result<Vec<f32>>
/// - async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>
/// - fn dimensions(&self) -> usize
/// - fn model_type(&self) -> EmbeddingModel
#[tokio::test]
async fn regression_test_embedding_service_trait_unchanged() {
    // Arrange: Create EmbeddingService implementation
    let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
        .expect("Failed to create embedding service");

    // Test 1: embed method signature and behavior
    let text = "Test text for regression";
    let embedding: Vec<f32> = service
        .embed(text)
        .await
        .expect("embed() should return Result<Vec<f32>>");

    assert!(!embedding.is_empty(), "embed() should return non-empty vector");
    assert_eq!(
        embedding.len(),
        384,
        "MiniLM embed() should return 384-dim vector"
    );

    // Test 2: embed_batch method signature and behavior
    let texts: &[&str] = &["Text 1", "Text 2", "Text 3"];
    let embeddings: Vec<Vec<f32>> = service
        .embed_batch(texts)
        .await
        .expect("embed_batch() should return Result<Vec<Vec<f32>>>");

    assert_eq!(
        embeddings.len(),
        3,
        "embed_batch() should return same number of embeddings as inputs"
    );
    for emb in &embeddings {
        assert_eq!(emb.len(), 384, "Each embedding should have 384 dimensions");
    }

    // Test 3: dimensions method signature and behavior
    let dims: usize = service.dimensions();
    assert_eq!(dims, 384, "dimensions() should return usize (384 for MiniLM)");

    // Test 4: model_type method signature and behavior
    let model: EmbeddingModel = service.model_type();
    assert_eq!(
        model,
        EmbeddingModel::LocalMiniLM,
        "model_type() should return EmbeddingModel"
    );
}

/// Regression Test 2: EmbeddingRepository (VectorStore) trait interface unchanged
///
/// This test verifies that the EmbeddingRepository trait maintains its contract:
/// - async fn insert(id, namespace, content, metadata, citation) -> Result<i64>
/// - async fn insert_batch(memories: Vec<(String, String, String)>) -> Result<Vec<i64>>
/// - async fn search_similar(query, limit, namespace_filter) -> Result<Vec<SearchResult>>
/// - async fn hybrid_search(query, limit, alpha) -> Result<Vec<SearchResult>>
/// - async fn get(id) -> Result<Option<VectorMemory>>
/// - async fn delete(id) -> Result<()>
/// - async fn count(namespace_prefix) -> Result<usize>
/// - async fn list_namespaces() -> Result<Vec<(String, usize)>>
#[tokio::test]
async fn regression_test_embedding_repository_trait_unchanged() {
    // Arrange: Setup database and VectorStore
    let pool = setup_test_db().await;
    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("Failed to create vector store");

    // Test 1: insert method signature and behavior
    let id = "regression-test-1";
    let namespace = "regression:test";
    let content = "Test content for regression";
    let metadata = Some(serde_json::json!({"test": "metadata"}));
    let citation = Some(Citation::from_file("/test/file.txt".to_string()));

    let rowid: i64 = vector_store
        .insert(id, namespace, content, metadata.clone(), citation.clone())
        .await
        .expect("insert() should return Result<i64>");

    assert!(rowid > 0, "insert() should return positive rowid");

    // Test 2: insert_batch method signature and behavior
    let memories: Vec<(String, String, String)> = vec![
        (
            "batch-1".to_string(),
            "regression:batch".to_string(),
            "Batch content 1".to_string(),
        ),
        (
            "batch-2".to_string(),
            "regression:batch".to_string(),
            "Batch content 2".to_string(),
        ),
    ];

    let rowids: Vec<i64> = vector_store
        .insert_batch(memories)
        .await
        .expect("insert_batch() should return Result<Vec<i64>>");

    assert_eq!(
        rowids.len(),
        2,
        "insert_batch() should return Vec<i64> with same length as input"
    );
    for rowid in &rowids {
        assert!(*rowid > 0, "Each rowid should be positive");
    }

    // Test 3: search_similar method signature and behavior
    let query = "search query";
    let limit = 10usize;
    let namespace_filter: Option<&str> = Some("regression:");

    let results = vector_store
        .search_similar(query, limit, namespace_filter)
        .await
        .expect("search_similar() should return Result<Vec<SearchResult>>");

    assert!(
        results.len() <= limit,
        "search_similar() should respect limit"
    );

    // Verify SearchResult structure hasn't changed
    for result in &results {
        let _id: &str = &result.id;
        let _namespace: &str = &result.namespace;
        let _content: &str = &result.content;
        let _distance: f32 = result.distance;
        let _metadata: &serde_json::Value = &result.metadata;
        let _citation: &Option<Citation> = &result.citation;
    }

    // Test 4: hybrid_search method signature and behavior
    let alpha = 0.7f32;
    let hybrid_results = vector_store
        .hybrid_search(query, limit, alpha)
        .await
        .expect("hybrid_search() should return Result<Vec<SearchResult>>");

    assert!(
        hybrid_results.len() <= limit,
        "hybrid_search() should respect limit"
    );

    // Test 5: get method signature and behavior
    let retrieved = vector_store
        .get(id)
        .await
        .expect("get() should return Result<Option<VectorMemory>>");

    assert!(retrieved.is_some(), "get() should return Some for existing id");

    // Verify VectorMemory structure hasn't changed
    if let Some(memory) = retrieved {
        let _id: String = memory.id;
        let _namespace: String = memory.namespace;
        let _content: String = memory.content;
        let _embedding: Vec<f32> = memory.embedding;
        let _metadata: serde_json::Value = memory.metadata;
        let _citation: Option<Citation> = memory.source_citation;
        let _created_at: chrono::DateTime<chrono::Utc> = memory.created_at;
        let _updated_at: chrono::DateTime<chrono::Utc> = memory.updated_at;
        let _created_by: String = memory.created_by;
    }

    // Test 6: count method signature and behavior
    let count: usize = vector_store
        .count("regression:")
        .await
        .expect("count() should return Result<usize>");

    assert!(count > 0, "count() should return usize");

    // Test 7: list_namespaces method signature and behavior
    let namespaces: Vec<(String, usize)> = vector_store
        .list_namespaces()
        .await
        .expect("list_namespaces() should return Result<Vec<(String, usize)>>");

    assert!(!namespaces.is_empty(), "list_namespaces() should return data");
    for (ns, count) in &namespaces {
        let _namespace_str: &str = ns;
        let _count_val: usize = *count;
    }

    // Test 8: delete method signature and behavior
    vector_store
        .delete(id)
        .await
        .expect("delete() should return Result<()>");

    let after_delete = vector_store
        .get(id)
        .await
        .expect("get() should work after delete");
    assert!(
        after_delete.is_none(),
        "get() should return None after delete"
    );
}

/// Regression Test 3: Public constructor signatures unchanged
///
/// Verifies that public constructors maintain their signatures
#[test]
fn regression_test_constructor_signatures_unchanged() {
    // Test 1: LocalEmbeddingService::new signature
    let service_result: anyhow::Result<LocalEmbeddingService> =
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM);
    assert!(service_result.is_ok(), "LocalEmbeddingService::new should accept EmbeddingModel and return Result<Self>");

    // Test 2: VectorStore::new signature
    let pool = Arc::new(setup_test_db_sync());
    let embedding_service = Arc::new(service_result.unwrap());

    let vector_store_result: anyhow::Result<VectorStore> =
        VectorStore::new(pool, embedding_service);
    assert!(
        vector_store_result.is_ok(),
        "VectorStore::new should accept Arc<SqlitePool> and Arc<dyn EmbeddingService> and return Result<Self>"
    );
}

/// Helper function for synchronous test
fn setup_test_db_sync() -> sqlx::SqlitePool {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(setup_test_db())
}

/// Regression Test 4: Determinism property maintained
///
/// Verifies that the deterministic embedding property is maintained
#[tokio::test]
async fn regression_test_determinism_property_maintained() {
    let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
        .expect("Failed to create service");

    let text = "Deterministic test text";

    // Generate embeddings multiple times
    let emb1 = service.embed(text).await.expect("embed() failed");
    let emb2 = service.embed(text).await.expect("embed() failed");
    let emb3 = service.embed(text).await.expect("embed() failed");

    // All should be identical (backward compatibility requirement)
    assert_eq!(
        emb1, emb2,
        "Determinism property must be maintained: same text should always produce same embedding"
    );
    assert_eq!(
        emb2, emb3,
        "Determinism property must be maintained: same text should always produce same embedding"
    );
}

/// Regression Test 5: Batch equivalence maintained
///
/// Verifies that batch processing produces same results as sequential (backward compat)
#[tokio::test]
async fn regression_test_batch_equivalence_maintained() {
    let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
        .expect("Failed to create service");

    let texts = vec!["Text A", "Text B", "Text C"];

    // Sequential embeddings
    let mut sequential = Vec::new();
    for text in &texts {
        sequential.push(service.embed(text).await.expect("embed() failed"));
    }

    // Batch embeddings
    let batch = service
        .embed_batch(&texts)
        .await
        .expect("embed_batch() failed");

    // Must be equivalent (backward compatibility requirement)
    assert_eq!(
        sequential.len(),
        batch.len(),
        "Batch equivalence must be maintained"
    );
    for (i, (seq, bat)) in sequential.iter().zip(batch.iter()).enumerate() {
        assert_eq!(
            seq, bat,
            "Batch equivalence must be maintained: batch[{}] != sequential[{}]",
            i, i
        );
    }
}

/// Regression Test 6: Normalization property maintained
///
/// Verifies that L2 normalization is still applied (backward compat)
#[tokio::test]
async fn regression_test_normalization_maintained() {
    let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
        .expect("Failed to create service");

    let embedding = service
        .embed("Normalization test")
        .await
        .expect("embed() failed");

    // Calculate L2 norm
    let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();

    // Must be normalized (backward compatibility requirement)
    assert!(
        (magnitude - 1.0).abs() < 1e-6,
        "Normalization property must be maintained: L2 norm should be 1.0, got {}",
        magnitude
    );

    // Verify no NaN or Inf
    for val in &embedding {
        assert!(
            val.is_finite(),
            "All embedding values must be finite (backward compatibility requirement)"
        );
    }
}

/// Regression Test 7: VectorStore cosine_distance public API unchanged
///
/// Verifies that public utility methods remain available
#[test]
fn regression_test_cosine_distance_public_api_unchanged() {
    // VectorStore::cosine_distance is a public method
    let vec_a = vec![1.0, 0.0, 0.0];
    let vec_b = vec![1.0, 0.0, 0.0];

    let distance: f32 = VectorStore::cosine_distance(&vec_a, &vec_b);

    // Identical vectors should have distance ~0
    assert!(
        distance.abs() < 1e-6,
        "cosine_distance() public API must be maintained"
    );

    // Test with orthogonal vectors
    let vec_c = vec![0.0, 1.0, 0.0];
    let distance2: f32 = VectorStore::cosine_distance(&vec_a, &vec_c);

    // Orthogonal vectors should have distance ~1
    assert!(
        (distance2 - 1.0).abs() < 1e-6,
        "cosine_distance() calculation must be maintained"
    );
}

/// Regression Test 8: Error handling backward compatibility
///
/// Verifies that error cases still work as expected
#[tokio::test]
async fn regression_test_error_handling_unchanged() {
    // Test 1: LocalEmbeddingService rejects API-based models
    let api_model_result = LocalEmbeddingService::new(EmbeddingModel::OpenAIAda002);
    assert!(
        api_model_result.is_err(),
        "LocalEmbeddingService must reject API-based models (backward compatibility)"
    );

    // Test 2: VectorStore delete of non-existent ID returns error
    let pool = setup_test_db().await;
    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("Failed to create vector store");

    let delete_result = vector_store.delete("non-existent-id").await;
    assert!(
        delete_result.is_err(),
        "delete() must return error for non-existent ID (backward compatibility)"
    );
}

/// Regression Test 9: Model dimensions unchanged
///
/// Verifies that model dimensions remain constant
#[test]
fn regression_test_model_dimensions_unchanged() {
    // MiniLM must be 384 dimensions
    let minilm_service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
        .expect("Failed to create MiniLM service");
    assert_eq!(
        minilm_service.dimensions(),
        384,
        "MiniLM dimensions must remain 384 (backward compatibility)"
    );

    // MPNet must be 768 dimensions
    let mpnet_service = LocalEmbeddingService::new(EmbeddingModel::LocalMPNet)
        .expect("Failed to create MPNet service");
    assert_eq!(
        mpnet_service.dimensions(),
        768,
        "MPNet dimensions must remain 768 (backward compatibility)"
    );
}

/// Regression Test 10: Existing integration tests still pass
///
/// This test verifies that the existing integration test patterns still work
#[tokio::test]
async fn regression_test_existing_patterns_still_work() {
    // This mirrors the pattern from vector_embedding_integration_test.rs
    let pool = setup_test_db().await;
    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("Failed to create vector store");

    // Pattern 1: Insert and retrieve
    let text = "The quick brown fox jumps over the lazy dog";
    let rowid = vector_store
        .insert("test-1", "test:namespace", text, None, None)
        .await
        .expect("Failed to insert");
    assert!(rowid > 0);

    // Pattern 2: Search
    let results = vector_store
        .search_similar("fox jumping", 3, Some("test:namespace"))
        .await
        .expect("Failed to search");
    assert!(!results.is_empty());

    // Pattern 3: Get by ID
    let memory = vector_store
        .get("test-1")
        .await
        .expect("Failed to get")
        .expect("Memory not found");
    assert_eq!(memory.id, "test-1");
    assert_eq!(memory.content, text);

    // All existing patterns must continue to work (backward compatibility)
}

/// Regression Test 11: Citation API unchanged
///
/// Verifies Citation struct and its usage remains compatible
#[tokio::test]
async fn regression_test_citation_api_unchanged() {
    // Citation::from_file must still work
    let citation = Citation::from_file("/path/to/file.txt".to_string());

    // Citation must have source field
    let _source: String = citation.source.clone();

    // Citation must be serializable/deserializable
    let json = serde_json::to_string(&citation).expect("Citation must be serializable");
    let deserialized: Citation =
        serde_json::from_str(&json).expect("Citation must be deserializable");

    assert_eq!(
        citation.source, deserialized.source,
        "Citation serialization roundtrip must work"
    );

    // Citation must work with VectorStore
    let pool = setup_test_db().await;
    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("Failed to create vector store");

    vector_store
        .insert(
            "citation-test",
            "test:citation",
            "Content with citation",
            None,
            Some(citation.clone()),
        )
        .await
        .expect("Citation must work with insert()");

    let retrieved = vector_store
        .get("citation-test")
        .await
        .expect("get() failed")
        .expect("Memory not found");

    assert!(
        retrieved.source_citation.is_some(),
        "Citation must be retrievable"
    );
    assert_eq!(
        retrieved.source_citation.unwrap().source,
        citation.source,
        "Citation data must be preserved"
    );
}

/// Regression Test 12: Migration path - old tests still pass
///
/// This test ensures that code written against the old API still compiles and works
#[tokio::test]
async fn regression_test_migration_path_validated() {
    // Simulate code written against the old API

    // Old pattern 1: Create embedding service
    let embedding_service: LocalEmbeddingService =
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM).unwrap();

    // Old pattern 2: Generate embedding
    let embedding: Vec<f32> = embedding_service.embed("test").await.unwrap();
    assert_eq!(embedding.len(), 384);

    // Old pattern 3: Create vector store
    let pool = setup_test_db().await;
    let vector_store = VectorStore::new(
        Arc::new(pool),
        Arc::new(embedding_service) as Arc<dyn EmbeddingService>,
    )
    .await
    .unwrap();

    // Old pattern 4: Insert and search
    vector_store
        .insert("id", "namespace", "content", None, None)
        .await
        .unwrap();

    let results = vector_store
        .search_similar("query", 10, None)
        .await
        .unwrap();

    // All old patterns must still work without modification
    assert!(results.len() <= 10);
}

/// Regression Test 13: Trait object compatibility maintained
///
/// Verifies that trait objects (Arc<dyn EmbeddingService>) still work
#[tokio::test]
async fn regression_test_trait_object_compatibility() {
    let embedding_service: Arc<dyn EmbeddingService> = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    // Must be able to call trait methods through Arc<dyn EmbeddingService>
    let embedding = embedding_service
        .embed("test text")
        .await
        .expect("embed() through trait object failed");
    assert_eq!(embedding.len(), 384);

    let batch = embedding_service
        .embed_batch(&["text1", "text2"])
        .await
        .expect("embed_batch() through trait object failed");
    assert_eq!(batch.len(), 2);

    let dims = embedding_service.dimensions();
    assert_eq!(dims, 384);

    let model = embedding_service.model_type();
    assert_eq!(model, EmbeddingModel::LocalMiniLM);

    // Must be able to pass trait object to VectorStore::new
    let pool = setup_test_db().await;
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("VectorStore::new must accept Arc<dyn EmbeddingService>");

    // Must be able to use VectorStore with trait object
    vector_store
        .insert("trait-test", "test", "content", None, None)
        .await
        .expect("VectorStore operations must work with trait object");
}

/// Regression Test 14: Empty batch handling unchanged
///
/// Verifies that edge cases are handled consistently
#[tokio::test]
async fn regression_test_edge_cases_unchanged() {
    let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
        .expect("Failed to create service");

    // Empty batch must return empty result
    let empty_batch = service
        .embed_batch(&[])
        .await
        .expect("embed_batch([]) must succeed");
    assert_eq!(
        empty_batch.len(),
        0,
        "Empty batch must return empty result"
    );

    // Empty string must still work
    let empty_embedding = service
        .embed("")
        .await
        .expect("embed('') must succeed");
    assert_eq!(
        empty_embedding.len(),
        384,
        "Empty string must produce 384-dim embedding"
    );

    // VectorStore empty batch
    let pool = setup_test_db().await;
    let embedding_service = Arc::new(service);
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("Failed to create vector store");

    let empty_rowids = vector_store
        .insert_batch(vec![])
        .await
        .expect("insert_batch(vec![]) must succeed");
    assert_eq!(
        empty_rowids.len(),
        0,
        "Empty batch insert must return empty rowids"
    );
}

/// Regression Test 15: Namespace filtering behavior unchanged
///
/// Verifies namespace filtering works as before
#[tokio::test]
async fn regression_test_namespace_filtering_unchanged() {
    let pool = setup_test_db().await;
    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );
    let vector_store = VectorStore::new(Arc::new(pool), embedding_service.clone())
        .await
        .expect("Failed to create vector store");

    // Insert in different namespaces
    vector_store
        .insert("ns1", "namespace:one", "Content 1", None, None)
        .await
        .unwrap();
    vector_store
        .insert("ns2", "namespace:two", "Content 2", None, None)
        .await
        .unwrap();
    vector_store
        .insert("other", "other:space", "Content 3", None, None)
        .await
        .unwrap();

    // Filter by exact namespace
    let exact = vector_store
        .search_similar("query", 10, Some("namespace:one"))
        .await
        .unwrap();
    assert_eq!(exact.len(), 1, "Exact namespace filter must work");
    assert_eq!(exact[0].namespace, "namespace:one");

    // Filter by prefix
    let prefix = vector_store
        .search_similar("query", 10, Some("namespace:"))
        .await
        .unwrap();
    assert_eq!(prefix.len(), 2, "Prefix namespace filter must work");

    // No filter returns all
    let all = vector_store.search_similar("query", 10, None).await.unwrap();
    assert_eq!(all.len(), 3, "No filter must return all namespaces");
}

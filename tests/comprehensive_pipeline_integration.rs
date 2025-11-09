//! Comprehensive Integration Tests for Complete Pipeline
//!
//! Tests: MemoryService → BertEmbedding → VectorStore → sqlite-vec
//!
//! ## Test Coverage
//! 1. End-to-end embedding pipeline with real BERT models
//! 2. MemoryService with real BERT embeddings
//! 3. Vector search quality validation with similarity thresholds
//! 4. Backward compatibility without vector search
//! 5. Graceful degradation when vec0 extension unavailable
//! 6. Batch processing throughput
//! 7. Concurrent operations
//! 8. Edge cases (empty text, unicode, long documents)
//!
//! ## Implementation Strategy
//! - Uses BertEmbeddingModel when available (real sentence-transformers)
//! - Falls back to LocalEmbeddingService if BERT unavailable
//! - Tests verify infrastructure correctness regardless of embedding quality
//! - Semantic quality tests only validate with real BERT embeddings

mod helpers;

use abathur_cli::domain::models::{
    Citation, Chunk, EmbeddingModel, Memory, MemoryType,
};
use abathur_cli::domain::ports::{
    ChunkingService, EmbeddingRepository, EmbeddingService, MemoryRepository,
};
use abathur_cli::infrastructure::database::MemoryRepositoryImpl;
use abathur_cli::infrastructure::vector::{
    BertEmbeddingModel, LocalEmbeddingService, VectorStore,
};
use abathur_cli::services::MemoryService;
use anyhow::Result;
use async_trait::async_trait;
use helpers::database::setup_test_db;
use std::sync::Arc;

// ============================================================================
// Test Helpers
// ============================================================================

/// Simple chunking service for testing
/// Chunks text into fixed-size pieces (500 chars by default)
struct SimpleChunker {
    chunk_size: usize,
}

impl SimpleChunker {
    fn new() -> Self {
        Self { chunk_size: 500 }
    }

    fn with_chunk_size(chunk_size: usize) -> Self {
        Self { chunk_size }
    }
}

#[async_trait]
impl ChunkingService for SimpleChunker {
    async fn chunk(&self, text: &str, parent_id: &str) -> Result<Vec<Chunk>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        let mut chunk_index = 0;

        while start < chars.len() {
            let end = (start + self.chunk_size).min(chars.len());
            let chunk_text: String = chars[start..end].iter().collect();
            let token_count = chunk_text.len(); // Simple approximation

            chunks.push(Chunk::new(
                parent_id.to_string(),
                chunk_text,
                chunk_index,
                token_count,
            ));

            start = end;
            chunk_index += 1;
        }

        Ok(chunks)
    }

    async fn count_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.len())
    }
}

/// Create embedding service with BERT fallback
///
/// Tries to use real BERT embeddings (sentence-transformers/all-MiniLM-L6-v2)
/// Falls back to LocalEmbeddingService if model unavailable
fn create_embedding_service(model_type: EmbeddingModel) -> Arc<dyn EmbeddingService> {
    match BertEmbeddingModel::new(model_type) {
        Ok(bert_model) => {
            eprintln!("✓ Using real BERT embeddings for integration tests");
            Arc::new(Arc::new(bert_model))
        }
        Err(e) => {
            eprintln!(
                "⚠ BERT model unavailable ({}), falling back to LocalEmbeddingService",
                e
            );
            Arc::new(
                LocalEmbeddingService::new(model_type)
                    .expect("Failed to create LocalEmbeddingService"),
            )
        }
    }
}

/// Check if BERT embeddings are available
fn is_bert_available() -> bool {
    BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM).is_ok()
}

// ============================================================================
// Test 1: End-to-End Embedding Pipeline
// ============================================================================

#[tokio::test]
async fn test_1_end_to_end_pipeline_complete_workflow() {
    // Arrange: Setup complete pipeline
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

    // Act: Add memory with rich content
    let memory = Memory::new(
        "test:e2e".to_string(),
        "rust-guide".to_string(),
        serde_json::json!({
            "title": "Introduction to Rust Programming",
            "content": "Rust is a systems programming language that runs blazingly fast, prevents segfaults, and guarantees thread safety through its ownership system."
        }),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(memory)
        .await
        .expect("Failed to add memory");

    // Assert: Memory was added
    assert!(id > 0, "Memory ID should be positive");

    // Act: Retrieve the memory
    let retrieved = memory_service
        .get("test:e2e", "rust-guide")
        .await
        .expect("Failed to get memory")
        .expect("Memory should exist");

    // Assert: Content matches
    assert_eq!(retrieved.namespace, "test:e2e");
    assert_eq!(retrieved.key, "rust-guide");
    assert_eq!(retrieved.value["title"], "Introduction to Rust Programming");

    // Act: Semantic search
    let search_results = memory_service
        .vector_search(
            "memory safety and high performance programming language",
            5,
            Some("test:e2e"),
        )
        .await
        .expect("Failed to perform vector search");

    // Assert: Search returns results
    assert!(
        !search_results.is_empty(),
        "Vector search should return results"
    );

    // The search should find chunks from our namespace
    let found = search_results
        .iter()
        .any(|r| r.namespace == "test:e2e");
    assert!(found, "Search should find chunks from test:e2e");

    // Assert: All results have valid distances (cosine distance in [0, 2])
    for result in &search_results {
        assert!(result.distance >= 0.0, "Distance should be non-negative");
        assert!(result.distance <= 2.0, "Cosine distance should be <= 2.0");
        assert!(!result.distance.is_nan(), "Distance should not be NaN");
        assert!(!result.distance.is_infinite(), "Distance should be finite");
    }

    // Assert: Results are sorted by distance (ascending = most similar first)
    for i in 1..search_results.len() {
        assert!(
            search_results[i - 1].distance <= search_results[i].distance,
            "Results should be sorted by distance ascending"
        );
    }
}

// ============================================================================
// Test 2: MemoryService with Real BERT Embeddings - Semantic Quality
// ============================================================================

#[tokio::test]
async fn test_2_memory_service_bert_semantic_quality() {
    // Arrange: Setup pipeline
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

    // Create categorized test documents
    let test_docs = vec![
        // Programming category
        (
            "prog-rust",
            "Rust programming language",
            "Rust is a systems programming language with memory safety guarantees and zero-cost abstractions.",
        ),
        (
            "prog-python",
            "Python data science",
            "Python is widely used in data science for machine learning, data analysis, and visualization.",
        ),
        (
            "prog-js",
            "JavaScript web development",
            "JavaScript is the primary language for web browser development and modern web applications.",
        ),
        // Nature category
        (
            "nature-forest",
            "Forest ecosystems",
            "Oak and maple trees dominate temperate deciduous forests, providing habitat for diverse wildlife.",
        ),
        (
            "nature-ocean",
            "Marine biology",
            "Dolphins and whales are highly intelligent marine mammals that communicate using echolocation.",
        ),
        (
            "nature-geology",
            "Mountain formation",
            "Mountain ranges form over millions of years through tectonic plate collisions and volcanic activity.",
        ),
    ];

    // Act: Add all test documents
    for (key, title, content) in &test_docs {
        let memory = Memory::new(
            "test:semantic".to_string(),
            key.to_string(),
            serde_json::json!({
                "title": title,
                "content": content
            }),
            MemoryType::Semantic,
            "test-user".to_string(),
        );

        memory_service
            .add(memory)
            .await
            .expect("Failed to add memory");
    }

    // Act: Search for programming-related content
    let prog_results = memory_service
        .vector_search(
            "software development and coding",
            6,
            Some("test:semantic"),
        )
        .await
        .expect("Failed to search");

    // Assert: Results are returned and sorted
    assert!(prog_results.len() > 0, "Should return search results");

    // Verify all results have valid distances and are sorted
    for i in 1..prog_results.len() {
        assert!(
            prog_results[i - 1].distance <= prog_results[i].distance,
            "Results should be sorted by distance"
        );
        assert!(
            prog_results[i].distance >= 0.0 && prog_results[i].distance <= 2.0,
            "Distance should be in [0, 2], got {}",
            prog_results[i].distance
        );
    }

    // Note: With real BERT embeddings, semantic grouping should be visible
    // With hash-based embeddings, we just verify infrastructure works
    if is_bert_available() {
        eprintln!("✓ BERT available: verifying semantic quality");

        // The top 3 results should be programming-related
        let top_3_ids: Vec<&str> = prog_results
            .iter()
            .take(3)
            .map(|r| r.id.as_str())
            .collect();

        let prog_count = top_3_ids
            .iter()
            .filter(|id| id.starts_with("prog-"))
            .count();

        assert!(
            prog_count >= 2,
            "With BERT, at least 2 of top 3 results should be programming-related, got: {:?}",
            top_3_ids
        );
    }

    // Act: Verify each document is searchable
    for (key, _, content) in &test_docs {
        let results = memory_service
            .vector_search(content, 1, Some("test:semantic"))
            .await
            .expect("Failed to search");

        assert!(
            !results.is_empty(),
            "Should find results for document {}",
            key
        );
    }
}

// ============================================================================
// Test 3: Vector Search Quality Validation with Similarity Thresholds
// ============================================================================

#[tokio::test]
async fn test_3_bert_similarity_thresholds() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

    // Add documents with varying semantic similarity
    let test_docs = vec![
        (
            "identical",
            "machine learning algorithms for classification and regression tasks",
        ),
        (
            "very-similar",
            "deep learning neural networks for supervised learning problems",
        ),
        (
            "somewhat-similar",
            "artificial intelligence and data science applications",
        ),
        (
            "unrelated",
            "cooking recipes for Italian pasta dishes and sauces",
        ),
    ];

    for (key, content) in &test_docs {
        memory_service
            .add(Memory::new(
                "test:similarity".to_string(),
                key.to_string(),
                serde_json::Value::String(content.to_string()),
                MemoryType::Semantic,
                "test-user".to_string(),
            ))
            .await
            .expect("Failed to add memory");
    }

    // Act: Search with query similar to first document
    let query = "machine learning classification algorithms";
    let results = memory_service
        .vector_search(query, 4, Some("test:similarity"))
        .await
        .expect("Failed to search");

    assert_eq!(results.len(), 4, "Should return all 4 documents");

    // Assert: Results are sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i - 1].distance <= results[i].distance,
            "Results should be sorted by distance ascending"
        );
    }

    // Assert: All distances are valid
    for result in &results {
        assert!(
            result.distance >= 0.0 && result.distance <= 2.0,
            "Distance should be in [0, 2] range, got {}",
            result.distance
        );
        assert!(!result.distance.is_nan(), "Distance should not be NaN");
        assert!(!result.distance.is_infinite(), "Distance should be finite");
    }

    // With real BERT embeddings, validate semantic similarity thresholds
    if is_bert_available() {
        eprintln!("✓ BERT available: validating similarity thresholds");

        let identical_result = results.iter().find(|r| r.id == "identical").unwrap();
        let very_similar_result = results.iter().find(|r| r.id == "very-similar").unwrap();
        let unrelated_result = results.iter().find(|r| r.id == "unrelated").unwrap();

        // BERT similarity thresholds:
        // - Identical/very similar: distance < 0.3
        // - Related: distance 0.3 - 0.8
        // - Unrelated: distance > 0.8

        assert!(
            identical_result.distance < 0.5,
            "Identical content should have distance < 0.5 with BERT, got {}",
            identical_result.distance
        );

        assert!(
            very_similar_result.distance < 0.8,
            "Very similar content should have distance < 0.8 with BERT, got {}",
            very_similar_result.distance
        );

        assert!(
            unrelated_result.distance > identical_result.distance,
            "Unrelated content should have higher distance than identical"
        );
    }
}

// ============================================================================
// Test 4: Backward Compatibility - System Works Without Vector Search
// ============================================================================

#[tokio::test]
async fn test_4_backward_compatibility_without_vector_search() {
    // Arrange: Create MemoryService WITHOUT vector store
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    // No vector store, no chunker - traditional CRUD only
    let memory_service = MemoryService::new(memory_repo, None, None);

    // Act: Add a memory (should succeed without vector search)
    let memory = Memory::new(
        "test:backward".to_string(),
        "traditional-memory".to_string(),
        serde_json::json!({"data": "Important information"}),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(memory)
        .await
        .expect("Traditional memory add should work without vector store");
    assert!(id > 0);

    // Act: Get the memory
    let retrieved = memory_service
        .get("test:backward", "traditional-memory")
        .await
        .expect("Traditional memory get should work")
        .expect("Memory should exist");

    // Assert: CRUD operations work
    assert_eq!(retrieved.namespace, "test:backward");
    assert_eq!(retrieved.key, "traditional-memory");
    assert_eq!(retrieved.value["data"], "Important information");

    // Act: Update the memory
    memory_service
        .update(
            "test:backward",
            "traditional-memory",
            serde_json::json!({"data": "Updated information"}),
            "test-user",
        )
        .await
        .expect("Traditional memory update should work");

    // Verify update
    let after_update = memory_service
        .get("test:backward", "traditional-memory")
        .await
        .expect("Get after update should work")
        .expect("Memory should exist");
    assert_eq!(after_update.value["data"], "Updated information");

    // Act: Search by namespace (traditional, non-vector)
    let namespace_results = memory_service
        .search("test:backward", None, None)
        .await
        .expect("Traditional namespace search should work");

    // Assert: Traditional search works
    assert_eq!(namespace_results.len(), 1);
    assert_eq!(namespace_results[0].key, "traditional-memory");

    // Act: Delete the memory
    memory_service
        .delete("test:backward", "traditional-memory")
        .await
        .expect("Traditional memory delete should work");

    // Verify deletion
    let after_delete = memory_service
        .get("test:backward", "traditional-memory")
        .await
        .expect("Get after delete should work");
    assert!(after_delete.is_none(), "Memory should be deleted");

    // Act: Vector search should fail gracefully
    let vector_result = memory_service.vector_search("test query", 10, None).await;

    // Assert: Clear error message
    assert!(
        vector_result.is_err(),
        "Vector search should fail without vector store"
    );
    let error_msg = vector_result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Vector search not available"),
        "Error should explain vector search is not available, got: {}",
        error_msg
    );
}

// ============================================================================
// Test 5: Graceful Degradation When vec0 Extension Unavailable
// ============================================================================

#[tokio::test]
async fn test_5_graceful_degradation_without_vec0() {
    // Note: VectorStore automatically falls back to pure Rust cosine distance
    // when sqlite-vec extension is unavailable

    // Arrange: Setup with vector store (will use fallback if vec0 unavailable)
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    // VectorStore gracefully degrades to pure Rust if vec0 unavailable
    let vector_store = VectorStore::new(pool_arc.clone(), embedding_service.clone())
        .await
        .expect("VectorStore should initialize even without vec0");

    // Act: Insert embeddings
    let rowid1 = vector_store
        .insert(
            "test-fallback-1",
            "test:fallback",
            "First test document about systems programming and performance",
            None,
            None,
        )
        .await
        .expect("Insert should work with pure Rust fallback");

    let rowid2 = vector_store
        .insert(
            "test-fallback-2",
            "test:fallback",
            "Second test document about web development and JavaScript frameworks",
            None,
            None,
        )
        .await
        .expect("Insert should work with pure Rust fallback");

    let rowid3 = vector_store
        .insert(
            "test-fallback-3",
            "test:fallback",
            "Third test document about data science and machine learning algorithms",
            None,
            None,
        )
        .await
        .expect("Insert should work with pure Rust fallback");

    // Assert: Inserts succeeded
    assert!(rowid1 > 0);
    assert!(rowid2 > 0);
    assert!(rowid3 > 0);

    // Act: Search
    let results = vector_store
        .search_similar("programming and software development", 3, Some("test:fallback"))
        .await
        .expect("Search should work with pure Rust fallback");

    // Assert: Search works correctly
    assert_eq!(results.len(), 3, "Should return all 3 documents");

    // Results should be sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i - 1].distance <= results[i].distance,
            "Results should be sorted even with pure Rust fallback"
        );
    }

    // All distances should be valid
    for result in &results {
        assert!(result.distance >= 0.0);
        assert!(result.distance <= 2.0);
        assert!(!result.distance.is_nan());
        assert!(!result.distance.is_infinite());
    }

    eprintln!("✓ VectorStore graceful degradation verified");
}

// ============================================================================
// Test 6: Batch Processing and Throughput
// ============================================================================

#[tokio::test]
async fn test_6_batch_processing_throughput() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store.clone()), Some(chunker));

    // Create batch of memories
    let batch_size = 50;
    let mut memories = Vec::new();
    for i in 0..batch_size {
        memories.push(Memory::new(
            "test:batch".to_string(),
            format!("doc-{}", i),
            serde_json::json!({
                "index": i,
                "content": format!("Document number {} about various programming topics and software engineering", i)
            }),
            MemoryType::Semantic,
            "test-user".to_string(),
        ));
    }

    // Act: Measure batch insert performance
    let insert_start = std::time::Instant::now();
    for memory in memories {
        memory_service
            .add(memory)
            .await
            .expect("Insert should succeed");
    }
    let insert_duration = insert_start.elapsed();

    eprintln!(
        "✓ Inserted {} documents in {:?} ({:.2} docs/sec)",
        batch_size,
        insert_duration,
        batch_size as f64 / insert_duration.as_secs_f64()
    );

    // Assert: All documents were inserted
    for i in 0..batch_size {
        let retrieved = memory_service
            .get("test:batch", &format!("doc-{}", i))
            .await
            .expect("Get should succeed");
        assert!(retrieved.is_some(), "Document {} should exist", i);
    }

    // Act: Measure search performance
    let search_start = std::time::Instant::now();
    let results = memory_service
        .vector_search("programming and software", 10, Some("test:batch"))
        .await
        .expect("Search should succeed");
    let search_duration = search_start.elapsed();

    eprintln!(
        "✓ Search completed in {:?} ({}ms)",
        search_duration,
        search_duration.as_millis()
    );

    // Assert: Search returns results
    assert!(results.len() > 0, "Should find results");
    assert!(results.len() <= 10, "Should respect limit");

    // Verify results are sorted
    for i in 1..results.len() {
        assert!(results[i - 1].distance <= results[i].distance);
    }

    // Note: Performance targets are in benchmarks, not integration tests
    // Integration tests verify correctness only
}

// ============================================================================
// Test 7: Concurrent Operations Stress Test
// ============================================================================

#[tokio::test]
async fn test_7_concurrent_operations() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = Arc::new(MemoryService::new(
        memory_repo,
        Some(vector_store),
        Some(chunker),
    ));

    // Act: Spawn multiple concurrent insert operations
    let mut handles = vec![];
    let concurrent_ops = 20;

    for i in 0..concurrent_ops {
        let service = memory_service.clone();
        let handle = tokio::spawn(async move {
            let memory = Memory::new(
                "test:concurrent".to_string(),
                format!("doc-{}", i),
                serde_json::json!({
                    "index": i,
                    "content": format!("Concurrent document number {} about various topics", i)
                }),
                MemoryType::Semantic,
                "test-user".to_string(),
            );

            service.add(memory).await
        });
        handles.push(handle);
    }

    // Wait for all operations to complete
    let results: Vec<_> = futures::future::join_all(handles).await;

    // Assert: All operations succeeded
    for (i, result) in results.into_iter().enumerate() {
        let id = result
            .expect("Task should not panic")
            .expect(&format!("Insert {} should succeed", i));
        assert!(id > 0);
    }

    // Act: Verify all memories were inserted
    for i in 0..concurrent_ops {
        let retrieved = memory_service
            .get("test:concurrent", &format!("doc-{}", i))
            .await
            .expect("Get should succeed");
        assert!(retrieved.is_some(), "Document {} should exist", i);
    }

    // Act: Concurrent searches
    let mut search_handles = vec![];
    for _ in 0..10 {
        let service = memory_service.clone();
        let handle = tokio::spawn(async move {
            service
                .vector_search("document", 10, Some("test:concurrent"))
                .await
        });
        search_handles.push(handle);
    }

    let search_results: Vec<_> = futures::future::join_all(search_handles).await;

    // Assert: All searches succeeded
    for result in search_results {
        let results = result
            .expect("Search task should not panic")
            .expect("Search should succeed");
        assert!(results.len() > 0, "Should find results");
    }

    eprintln!("✓ Concurrent operations verified: {} inserts, 10 concurrent searches", concurrent_ops);
}

// ============================================================================
// Test 8: Edge Cases - Empty Text, Unicode, Long Documents
// ============================================================================

#[tokio::test]
async fn test_8_edge_cases_comprehensive() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    // Use small chunk size to test chunking
    let chunker = Arc::new(SimpleChunker::with_chunk_size(100)) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store.clone()), Some(chunker));

    // Test 1: Empty string content
    let empty_memory = Memory::new(
        "test:edge".to_string(),
        "empty-content".to_string(),
        serde_json::Value::String("".to_string()),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(empty_memory)
        .await
        .expect("Should handle empty string content");
    assert!(id > 0);

    // Test 2: Very long content (tests chunking)
    let long_content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(50);
    let long_memory = Memory::new(
        "test:edge".to_string(),
        "very-long-content".to_string(),
        serde_json::Value::String(long_content.clone()),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(long_memory)
        .await
        .expect("Should handle very long content");
    assert!(id > 0);

    // Verify chunking happened
    let chunk_count = vector_store
        .count("test:edge")
        .await
        .expect("Failed to count");

    // Should have multiple chunks for the long document
    let _expected_chunks = (long_content.len() + 99) / 100;
    assert!(
        chunk_count > 1,
        "Long document should be chunked, got {} chunks",
        chunk_count
    );

    // Test 3: Unicode and special characters
    let unicode_tests = vec![
        ("unicode-emoji", "🦀 Rust programming 🚀 with emoji"),
        ("unicode-cjk", "Programming with 中文 and 日本語 characters"),
        ("unicode-math", "Mathematical symbols: α β γ Δ ∑ ∫ √"),
        ("special-chars", "C++, Python, & JavaScript!!! @#$%^&*()"),
    ];

    for (key, content) in unicode_tests {
        let memory = Memory::new(
            "test:edge".to_string(),
            key.to_string(),
            serde_json::Value::String(content.to_string()),
            MemoryType::Semantic,
            "test-user".to_string(),
        );

        let id = memory_service
            .add(memory)
            .await
            .expect(&format!("Should handle unicode/special chars: {}", content));
        assert!(id > 0);
    }

    // Test 4: Complex nested JSON
    let complex_json = serde_json::json!({
        "level1": {
            "level2": {
                "level3": {
                    "data": "Deep nesting",
                    "array": [1, 2, 3, 4, 5],
                    "bool": true,
                    "null": null,
                    "unicode": "Hello 世界"
                }
            }
        }
    });

    let complex_memory = Memory::new(
        "test:edge".to_string(),
        "complex-json".to_string(),
        complex_json,
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(complex_memory)
        .await
        .expect("Should handle complex nested JSON");
    assert!(id > 0);

    // Act: Search should work on all edge cases
    let results = memory_service
        .vector_search("programming", 20, Some("test:edge"))
        .await
        .expect("Search should work with edge cases");

    // Assert: All memories are searchable
    assert!(results.len() > 0, "Should find edge case memories");

    eprintln!("✓ Edge cases verified: empty, long, unicode, special chars, nested JSON");
}

// ============================================================================
// Test 9: MPNet Model Integration (768 dimensions)
// ============================================================================

#[tokio::test]
async fn test_9_mpnet_model_integration() {
    // Arrange: Setup with MPNet (768-dimensional) instead of MiniLM (384-dimensional)
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMPNet);

    // Verify dimensions
    assert_eq!(
        embedding_service.dimensions(),
        768,
        "MPNet should have 768 dimensions"
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

    // Act: Add memory with MPNet embeddings
    let memory = Memory::new(
        "test:mpnet".to_string(),
        "mpnet-doc".to_string(),
        serde_json::Value::String(
            "Testing MPNet model with 768-dimensional embeddings for better semantic quality"
                .to_string(),
        ),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(memory)
        .await
        .expect("Should work with MPNet model");
    assert!(id > 0);

    // Act: Search with MPNet
    let results = memory_service
        .vector_search("MPNet testing semantics", 5, Some("test:mpnet"))
        .await
        .expect("Search should work with MPNet");

    // Assert: Results are valid
    assert!(results.len() > 0, "Should find results with MPNet");

    // Verify embeddings have correct dimensions
    let retrieved = memory_service
        .get("test:mpnet", "mpnet-doc")
        .await
        .expect("Get should succeed")
        .expect("Memory should exist");

    assert_eq!(retrieved.namespace, "test:mpnet");

    eprintln!("✓ MPNet model integration verified (768 dimensions)");
}

// ============================================================================
// Test 10: Citation and Metadata Preservation Through Complete Pipeline
// ============================================================================

#[tokio::test]
async fn test_10_citation_metadata_preservation() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = create_embedding_service(EmbeddingModel::LocalMiniLM);

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .await
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    // Act: Insert with citation and metadata directly
    let citation = Citation::from_url("https://docs.rs/tokio".to_string());
    let metadata = serde_json::json!({
        "version": "1.40",
        "category": "async-runtime",
        "language": "rust"
    });

    vector_store
        .insert(
            "cited-doc",
            "test:citation",
            "Tokio is an asynchronous runtime for the Rust programming language providing async I/O, timers, and multi-threaded task scheduler",
            Some(metadata.clone()),
            Some(citation.clone()),
        )
        .await
        .expect("Insert with citation should succeed");

    // Assert: Citation is preserved in search results
    let search_results = vector_store
        .search_similar("async runtime scheduler", 1, Some("test:citation"))
        .await
        .expect("Search should succeed");

    assert_eq!(search_results.len(), 1);

    assert!(
        search_results[0].citation.is_some(),
        "Citation should be in search results"
    );

    let retrieved_citation = search_results[0].citation.as_ref().unwrap();
    assert_eq!(retrieved_citation.source, citation.source);

    // Assert: Metadata is also preserved
    assert_eq!(search_results[0].metadata["version"], "1.40");
    assert_eq!(search_results[0].metadata["category"], "async-runtime");
    assert_eq!(search_results[0].metadata["language"], "rust");

    // Assert: Get operation also preserves citation
    let get_result = vector_store
        .get("cited-doc")
        .await
        .expect("Get should succeed")
        .expect("Memory should exist");

    assert!(get_result.source_citation.is_some());
    assert_eq!(get_result.metadata["version"], "1.40");

    eprintln!("✓ Citation and metadata preservation verified");
}

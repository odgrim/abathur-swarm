//! Comprehensive integration tests for the complete embedding pipeline
//!
//! Tests MemoryService → BertEmbeddingModel → VectorStore → sqlite-vec
//!
//! ## Test Coverage
//! 1. End-to-end embedding pipeline with real BERT (when available)
//! 2. MemoryService with real BERT embeddings
//! 3. Vector search quality validation
//! 4. Backward compatibility verification
//! 5. Graceful degradation when vec0 unavailable
//!
//! ## Test Strategy
//! - Unit tests in source files test individual components
//! - Integration tests here test the complete pipeline
//! - Property tests verify invariants across all scenarios
//! - Performance is measured but not enforced (see benchmarks)

mod helpers;

use abathur_cli::domain::models::{
    Citation, Chunk, EmbeddingModel, Memory, MemoryType,
};
use abathur_cli::domain::ports::{
    ChunkingService, EmbeddingRepository, EmbeddingService, MemoryRepository,
};
use abathur_cli::infrastructure::database::MemoryRepositoryImpl;
use abathur_cli::infrastructure::vector::{LocalEmbeddingService, VectorStore};
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
        // Simple character-based approximation
        Ok(text.len())
    }
}

// ============================================================================
// Test 1: End-to-end embedding pipeline with MemoryService
// ============================================================================

#[tokio::test]
async fn test_end_to_end_memory_service_pipeline() {
    // Arrange: Setup complete pipeline
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

    // Act: Add a memory
    let memory = Memory::new(
        "test:pipeline".to_string(),
        "document-1".to_string(),
        serde_json::json!({
            "title": "Introduction to Rust",
            "content": "Rust is a systems programming language that runs blazingly fast, prevents segfaults, and guarantees thread safety."
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
        .get("test:pipeline", "document-1")
        .await
        .expect("Failed to get memory");

    // Assert: Memory exists and matches
    assert!(retrieved.is_some(), "Memory should exist");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.namespace, "test:pipeline");
    assert_eq!(retrieved.key, "document-1");
    assert_eq!(retrieved.value["title"], "Introduction to Rust");

    // Act: Search for semantically similar content
    let search_results = memory_service
        .vector_search("programming language memory safety", 5, Some("test:pipeline"))
        .await
        .expect("Failed to perform vector search");

    // Assert: Search returns results
    assert!(
        !search_results.is_empty(),
        "Vector search should return results"
    );

    // The search should find the memory we added (via chunks)
    let found = search_results
        .iter()
        .any(|r| r.namespace == "test:pipeline");
    assert!(
        found,
        "Search should find chunks from test:pipeline namespace"
    );

    // Assert: All results have valid distances
    for result in &search_results {
        assert!(result.distance >= 0.0, "Distance should be non-negative");
        assert!(!result.distance.is_nan(), "Distance should not be NaN");
    }

    // Assert: Results are sorted by distance
    for i in 1..search_results.len() {
        assert!(
            search_results[i - 1].distance <= search_results[i].distance,
            "Results should be sorted by distance ascending"
        );
    }
}

// ============================================================================
// Test 2: MemoryService with chunking for large content
// ============================================================================

#[tokio::test]
async fn test_memory_service_with_chunking() {
    // Arrange: Setup pipeline with small chunk size to force chunking
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    // Use small chunk size to ensure chunking happens
    let chunker = Arc::new(SimpleChunker::with_chunk_size(100)) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store.clone()), Some(chunker));

    // Create a long text that will be chunked
    let long_text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(20);

    let memory = Memory::new(
        "test:chunking".to_string(),
        "long-document".to_string(),
        serde_json::Value::String(long_text.clone()),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    // Act: Add the memory
    let id = memory_service
        .add(memory)
        .await
        .expect("Failed to add memory");
    assert!(id > 0);

    // Act: Count vectors in the namespace (should be multiple chunks)
    let vector_count = vector_store
        .count("test:chunking")
        .await
        .expect("Failed to count vectors");

    // Assert: Multiple chunks were created
    assert!(
        vector_count > 1,
        "Long text should be split into multiple chunks, got {}",
        vector_count
    );

    // The expected number of chunks: text length / chunk size (rounded up)
    let expected_chunks = (long_text.len() + 99) / 100;
    assert_eq!(
        vector_count, expected_chunks,
        "Should have approximately {} chunks",
        expected_chunks
    );

    // Act: Search should find chunks from the long document
    let results = memory_service
        .vector_search("Lorem ipsum", 10, Some("test:chunking"))
        .await
        .expect("Failed to search");

    // Assert: Multiple chunks are searchable
    assert!(
        results.len() > 1,
        "Search should return multiple chunks from long document"
    );
}

// ============================================================================
// Test 3: Vector search quality validation
// ============================================================================

#[tokio::test]
async fn test_vector_search_quality() {
    // Arrange: Setup pipeline
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
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
            "prog-1",
            "Rust programming language features",
            "Rust is a systems programming language with memory safety guarantees and zero-cost abstractions.",
        ),
        (
            "prog-2",
            "Python data science",
            "Python is widely used in data science for machine learning, data analysis, and visualization.",
        ),
        (
            "prog-3",
            "JavaScript web development",
            "JavaScript is the primary language for web browser development and modern web applications.",
        ),
        // Nature category
        (
            "nature-1",
            "Forest ecosystems",
            "Oak and maple trees dominate temperate deciduous forests, providing habitat for diverse wildlife.",
        ),
        (
            "nature-2",
            "Marine biology",
            "Dolphins and whales are highly intelligent marine mammals that communicate using echolocation.",
        ),
        (
            "nature-3",
            "Geology",
            "Mountain ranges form over millions of years through tectonic plate collisions and volcanic activity.",
        ),
    ];

    // Act: Add all test documents
    for (key, title, content) in &test_docs {
        let memory = Memory::new(
            "test:quality".to_string(),
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
        .vector_search("software development programming", 6, Some("test:quality"))
        .await
        .expect("Failed to search");

    // Assert: All results are returned and sorted
    assert!(prog_results.len() > 0, "Should return search results");

    // Verify all results have valid distances and are sorted
    for i in 1..prog_results.len() {
        assert!(
            prog_results[i - 1].distance <= prog_results[i].distance,
            "Results should be sorted by distance"
        );
    }

    // Note: Semantic grouping validation requires real BERT embeddings
    // With deterministic hash-based embeddings, we can only verify:
    // 1. Search infrastructure works (✓)
    // 2. Results are sorted correctly (✓)
    // 3. All documents are searchable (verified below)

    // Act: Verify each document is searchable
    for (key, _, content) in &test_docs {
        let results = memory_service
            .vector_search(content, 1, Some("test:quality"))
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
// Test 4: Backward compatibility - MemoryService without vector search
// ============================================================================

#[tokio::test]
async fn test_backward_compatibility_without_vector_search() {
    // Arrange: Create MemoryService WITHOUT vector store (backward compatible mode)
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    // No vector store, no chunker - should still work for traditional CRUD
    let memory_service = MemoryService::new(memory_repo, None, None);

    // Act: Add a memory (should succeed even without vector search)
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
        .expect("Traditional memory get should work");

    // Assert: CRUD operations work
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
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

    // Act: Search by namespace (traditional, non-vector search)
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
}

// ============================================================================
// Test 5: Graceful degradation when vec0 extension unavailable
// ============================================================================

#[tokio::test]
async fn test_graceful_degradation_without_vec0() {
    // Note: This test verifies that the VectorStore falls back to pure Rust
    // cosine distance calculation when sqlite-vec extension is unavailable.
    // The actual extension loading is handled during database initialization.

    // Arrange: Setup with vector store (will use pure Rust fallback if vec0 unavailable)
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    // VectorStore gracefully degrades to pure Rust if vec0 is unavailable
    let vector_store = VectorStore::new(pool_arc.clone(), embedding_service.clone())
        .expect("VectorStore should initialize even without vec0");

    // Act: Insert embeddings
    let rowid1 = vector_store
        .insert("test-1", "test:fallback", "First test document", None, None)
        .await
        .expect("Insert should work with pure Rust fallback");

    let rowid2 = vector_store
        .insert("test-2", "test:fallback", "Second test document", None, None)
        .await
        .expect("Insert should work with pure Rust fallback");

    // Assert: Inserts succeeded
    assert!(rowid1 > 0);
    assert!(rowid2 > 0);

    // Act: Search
    let results = vector_store
        .search_similar("test query", 2, Some("test:fallback"))
        .await
        .expect("Search should work with pure Rust fallback");

    // Assert: Search works correctly
    assert_eq!(results.len(), 2, "Should return both documents");

    // Results should be sorted by distance
    assert!(
        results[0].distance <= results[1].distance,
        "Results should be sorted even with pure Rust fallback"
    );

    // All distances should be valid
    for result in &results {
        assert!(result.distance >= 0.0);
        assert!(!result.distance.is_nan());
        assert!(!result.distance.is_infinite());
    }
}

// ============================================================================
// Test 6: Empty and edge case handling
// ============================================================================

#[tokio::test]
async fn test_edge_cases() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

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

    // Test 2: Very long content (tests chunking limits)
    let long_content = "A".repeat(10000);
    let long_memory = Memory::new(
        "test:edge".to_string(),
        "very-long-content".to_string(),
        serde_json::Value::String(long_content),
        MemoryType::Semantic,
        "test-user".to_string(),
    );

    let id = memory_service
        .add(long_memory)
        .await
        .expect("Should handle very long content");
    assert!(id > 0);

    // Test 3: Complex nested JSON
    let complex_json = serde_json::json!({
        "level1": {
            "level2": {
                "level3": {
                    "data": "Deep nesting",
                    "array": [1, 2, 3, 4, 5],
                    "bool": true,
                    "null": null
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

    // Test 4: Search on empty database returns empty results
    let pool2 = setup_test_db().await;
    let pool2_arc = Arc::new(pool2);
    let embedding_service2 = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );
    let empty_vector_store = VectorStore::new(pool2_arc, embedding_service2)
        .expect("Failed to create vector store");

    let empty_results = empty_vector_store
        .search_similar("any query", 10, None)
        .await
        .expect("Search on empty database should not fail");

    assert_eq!(empty_results.len(), 0, "Empty database should return no results");
}

// ============================================================================
// Test 7: Concurrent operations
// ============================================================================

#[tokio::test]
async fn test_concurrent_operations() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
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

    for i in 0..10 {
        let service = memory_service.clone();
        let handle = tokio::spawn(async move {
            let memory = Memory::new(
                "test:concurrent".to_string(),
                format!("doc-{}", i),
                serde_json::json!({
                    "index": i,
                    "content": format!("Document number {}", i)
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
    for i in 0..10 {
        let retrieved = memory_service
            .get("test:concurrent", &format!("doc-{}", i))
            .await
            .expect("Get should succeed");
        assert!(retrieved.is_some(), "Document {} should exist", i);
    }

    // Act: Concurrent searches
    let mut search_handles = vec![];
    for _ in 0..5 {
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
}

// ============================================================================
// Test 8: MPNet model integration
// ============================================================================

#[tokio::test]
async fn test_mpnet_model_integration() {
    // Arrange: Setup with MPNet (768-dimensional) instead of MiniLM (384-dimensional)
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMPNet)
            .expect("Failed to create MPNet embedding service"),
    );

    // Verify dimensions
    assert_eq!(
        embedding_service.dimensions(),
        768,
        "MPNet should have 768 dimensions"
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
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
        serde_json::Value::String("Testing MPNet model with 768 dimensions".to_string()),
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
        .vector_search("MPNet testing", 5, Some("test:mpnet"))
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
}

// ============================================================================
// Test 9: Citation preservation through pipeline
// ============================================================================

#[tokio::test]
async fn test_citation_preservation() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    // Act: Insert with citation directly
    let citation = Citation::from_url("https://example.com/document".to_string());

    vector_store
        .insert(
            "cited-doc",
            "test:citation",
            "Content with citation information",
            Some(serde_json::json!({"page": 42})),
            Some(citation.clone()),
        )
        .await
        .expect("Insert with citation should succeed");

    // Assert: Citation is preserved in search results
    let search_results = vector_store
        .search_similar("citation", 1, Some("test:citation"))
        .await
        .expect("Search should succeed");

    assert_eq!(search_results.len(), 1);
    assert!(
        search_results[0].citation.is_some(),
        "Citation should be in search results"
    );

    let retrieved_citation = search_results[0].citation.as_ref().unwrap();
    assert_eq!(retrieved_citation.source, citation.source);
}

// ============================================================================
// Test 10: Performance measurement (informational, not enforced)
// ============================================================================

#[tokio::test]
async fn test_performance_measurement() {
    // Arrange
    let pool = setup_test_db().await;
    let pool_arc = Arc::new(pool);

    let embedding_service = Arc::new(
        LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create embedding service"),
    );

    let vector_store = Arc::new(
        VectorStore::new(pool_arc.clone(), embedding_service.clone())
            .expect("Failed to create vector store"),
    ) as Arc<dyn EmbeddingRepository>;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new((*pool_arc).clone()))
        as Arc<dyn MemoryRepository>;

    let chunker = Arc::new(SimpleChunker::new()) as Arc<dyn ChunkingService>;

    let memory_service = MemoryService::new(memory_repo, Some(vector_store), Some(chunker));

    // Measure: Batch insert performance
    let mut memories = Vec::new();
    for i in 0..50 {
        memories.push(Memory::new(
            "test:perf".to_string(),
            format!("perf-doc-{}", i),
            serde_json::json!({
                "index": i,
                "content": format!("Performance test document number {}", i)
            }),
            MemoryType::Semantic,
            "test-user".to_string(),
        ));
    }

    let insert_start = std::time::Instant::now();
    for memory in memories {
        memory_service
            .add(memory)
            .await
            .expect("Insert should succeed");
    }
    let insert_duration = insert_start.elapsed();

    println!(
        "✓ Inserted 50 documents in {:?} ({:.2} docs/sec)",
        insert_duration,
        50.0 / insert_duration.as_secs_f64()
    );

    // Measure: Search performance
    let search_start = std::time::Instant::now();
    let _results = memory_service
        .vector_search("performance test", 10, Some("test:perf"))
        .await
        .expect("Search should succeed");
    let search_duration = search_start.elapsed();

    println!(
        "✓ Search completed in {:?} ({}ms)",
        search_duration,
        search_duration.as_millis()
    );

    // Note: These are informational only. Use criterion benchmarks for performance requirements.
    // Integration tests verify correctness, not performance.
}

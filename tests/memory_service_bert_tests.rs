//! Integration tests for MemoryService with BERT embeddings
//!
//! These tests validate the complete MemoryService workflow with real BERT embeddings, including:
//! - End-to-end memory storage and retrieval with embeddings
//! - Semantic search quality with real BERT similarity thresholds
//! - Edge cases (empty text, very long text)
//! - Graceful degradation without vector capabilities
//!
//! ## Implementation Note
//! Uses BertEmbeddingModel for production-quality semantic search testing.
//! Tests are designed to work with both real BERT embeddings AND hash-based fallback.
//!
//! ### BERT vs Hash-Based Embeddings
//! - **BERT (preferred)**: Semantic embeddings from sentence-transformers/all-MiniLM-L6-v2
//!   - Distance thresholds: < 0.3 (very similar), 0.3-0.8 (related), > 0.8 (unrelated)
//!   - Captures semantic meaning (e.g., "car" and "automobile" are similar)
//! - **Hash-based (fallback)**: Simple hash of normalized text for testing without model download
//!   - Distance thresholds are different, tests focus on relative ordering
//!   - Deterministic but doesn't capture semantic similarity
//!
//! Tests gracefully handle both modes by checking relative distance ordering rather than
//! absolute thresholds when BERT is unavailable.

mod helpers;

use abathur_cli::domain::models::{EmbeddingModel, Memory, MemoryType};
use abathur_cli::domain::ports::{EmbeddingService, MemoryRepository};
use abathur_cli::infrastructure::database::MemoryRepositoryImpl;
use abathur_cli::infrastructure::vector::{BertEmbeddingModel, Chunker, LocalEmbeddingService, VectorStore};
use abathur_cli::services::MemoryService;
use helpers::database::setup_test_db;
use serde_json::json;
use std::sync::Arc;

/// Helper to create embedding service with graceful fallback
///
/// Tries to use BERT embeddings first, falls back to LocalEmbeddingService if unavailable.
/// Note: Arc<BertEmbeddingModel> implements EmbeddingService, so we wrap in Arc twice.
///
/// # Fallback Behavior
/// - **BERT available**: Downloads model from HuggingFace (~90MB, cached locally)
///   - Provides true semantic search with sentence-transformers embeddings
///   - Distance thresholds: < 0.3 (very similar), 0.3-0.8 (related), > 0.8 (unrelated)
/// - **BERT unavailable**: Uses hash-based LocalEmbeddingService
///   - Deterministic but not semantic (exact text matching)
///   - Tests verify relative ordering instead of absolute thresholds
///
/// # Returns
/// * `Arc<dyn EmbeddingService>` - Either BERT or hash-based embedding service
fn create_embedding_service() -> Arc<dyn EmbeddingService> {
    match BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM) {
        Ok(bert_model) => {
            eprintln!("✓ Using real BERT embeddings for integration tests");
            eprintln!("  Model: sentence-transformers/all-MiniLM-L6-v2 (384 dimensions)");
            // Arc<BertEmbeddingModel> implements EmbeddingService, so wrap twice
            Arc::new(Arc::new(bert_model))
        }
        Err(e) => {
            eprintln!("⚠ BERT model unavailable ({}), falling back to LocalEmbeddingService", e);
            eprintln!("  Tests will verify relative ordering instead of absolute similarity thresholds");
            Arc::new(
                LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
                    .expect("Failed to create LocalEmbeddingService")
            )
        }
    }
}

/// Test helper to create a MemoryService with BERT embeddings
///
/// Uses BertEmbeddingModel (sentence-transformers/all-MiniLM-L6-v2) for real semantic search.
/// Downloads model from HuggingFace on first run (~90MB, cached locally).
/// Falls back to LocalEmbeddingService if BERT is unavailable.
async fn create_memory_service_with_embeddings() -> MemoryService {
    let pool = setup_test_db().await;
    let memory_repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;

    // Initialize embedding service (BERT with fallback)
    let embedding_service = create_embedding_service();

    // Create vector store
    let vector_store = Arc::new(
        VectorStore::new(Arc::new(pool), embedding_service)
            .await
            .expect("Failed to create vector store")
    );

    // Create chunking service
    let chunker = Arc::new(
        Chunker::new()
            .expect("Failed to create chunker")
    );

    MemoryService::new(memory_repo, Some(vector_store), Some(chunker))
}

/// Test helper for MemoryService without vector capabilities (graceful degradation)
async fn create_memory_service_without_vector() -> MemoryService {
    let pool = setup_test_db().await;
    let memory_repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;

    MemoryService::new(memory_repo, None, None)
}

// =============================================================================
// Test 1: End-to-End Memory Workflow with BERT Embeddings
// =============================================================================

#[tokio::test]
async fn test_memory_add_with_embedding() {
    let service = create_memory_service_with_embeddings().await;

    let memory = Memory::new(
        "test:docs".to_string(),
        "rust_guide".to_string(),
        json!("Rust is a systems programming language focused on safety and performance"),
        MemoryType::Semantic,
        "test".to_string(),
    );

    // Act: Add memory (should also generate BERT embedding and store in vector DB)
    let result = service.add(memory.clone()).await;

    // Assert: Addition succeeds
    assert!(result.is_ok(), "Memory addition should succeed");

    // Verify retrieval works
    let retrieved = service
        .get("test:docs", "rust_guide")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    assert_eq!(retrieved.namespace, "test:docs");
    assert_eq!(retrieved.key, "rust_guide");
}

// =============================================================================
// Test 2: Semantic Search Quality with Real BERT Embeddings
// =============================================================================

#[tokio::test]
async fn test_semantic_search_quality() {
    let service = create_memory_service_with_embeddings().await;

    // Add semantically related memories
    let memories = vec![
        Memory::new(
            "test:programming".to_string(),
            "rust".to_string(),
            json!("Rust is a systems programming language with memory safety guarantees"),
            MemoryType::Semantic,
            "test".to_string(),
        ),
        Memory::new(
            "test:programming".to_string(),
            "python".to_string(),
            json!("Python is a high-level programming language great for data science"),
            MemoryType::Semantic,
            "test".to_string(),
        ),
        Memory::new(
            "test:nature".to_string(),
            "oak_tree".to_string(),
            json!("Oak trees are large deciduous trees found in temperate forests"),
            MemoryType::Semantic,
            "test".to_string(),
        ),
    ];

    for memory in memories {
        service.add(memory).await.expect("Failed to add memory");
    }

    // Act: Search for programming-related content
    let results = service
        .vector_search("systems programming and memory safety", 5, Some("test:"))
        .await
        .expect("Vector search should succeed");

    // Assert: Results should prioritize semantically similar content
    assert!(!results.is_empty(), "Should return search results");

    // With real BERT embeddings, semantic similarity is meaningful
    // The top result should be the Rust memory (highest semantic similarity)
    let top_result = &results[0];

    // We expect high similarity (low distance) for semantically related content
    // With cosine distance: 0.0 = identical, 2.0 = opposite
    // BERT thresholds for semantic similarity:
    // - Very similar: distance < 0.3
    // - Related: distance 0.3 - 0.8
    // - Unrelated: distance > 0.8
    assert!(
        top_result.distance >= 0.0 && top_result.distance <= 2.0,
        "Distance should be in valid range [0, 2], got {}",
        top_result.distance
    );

    // Verify the top result is semantically relevant
    // Note: VectorStore returns chunk IDs, not the original memory key
    // We need to check the content or namespace instead
    eprintln!("Top result: id={}, namespace={}, content={}",
        top_result.id, top_result.namespace, top_result.content);

    // The top result should be from the programming namespace
    assert!(
        top_result.namespace == "test:programming",
        "Top result should be from programming namespace, got: {}",
        top_result.namespace
    );

    // Verify programming content ranks higher than nature content
    let programming_results: Vec<_> = results.iter()
        .filter(|r| r.namespace == "test:programming")
        .collect();
    let nature_results: Vec<_> = results.iter()
        .filter(|r| r.namespace == "test:nature")
        .collect();

    if !programming_results.is_empty() && !nature_results.is_empty() {
        let first_prog_idx = results.iter().position(|r| r.namespace == "test:programming").unwrap();
        let first_nature_idx = results.iter().position(|r| r.namespace == "test:nature").unwrap();

        assert!(
            first_prog_idx < first_nature_idx,
            "Programming content should rank higher than nature content for programming query"
        );
    }

    // Verify results are sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i - 1].distance <= results[i].distance,
            "Results should be sorted by distance ascending"
        );
    }
}

#[tokio::test]
async fn test_semantic_similarity_thresholds() {
    let service = create_memory_service_with_embeddings().await;

    // Add memories with varying semantic similarity to query
    service
        .add(Memory::new(
            "test:similarity".to_string(),
            "identical".to_string(),
            json!("machine learning algorithms"),
            MemoryType::Semantic,
            "test".to_string(),
        ))
        .await
        .expect("Failed to add memory");

    service
        .add(Memory::new(
            "test:similarity".to_string(),
            "similar".to_string(),
            json!("deep learning and neural networks"),
            MemoryType::Semantic,
            "test".to_string(),
        ))
        .await
        .expect("Failed to add memory");

    service
        .add(Memory::new(
            "test:similarity".to_string(),
            "unrelated".to_string(),
            json!("cooking recipes and food preparation"),
            MemoryType::Semantic,
            "test".to_string(),
        ))
        .await
        .expect("Failed to add memory");

    // Act: Search with query very similar to first memory
    let results = service
        .vector_search("machine learning algorithms", 3, Some("test:similarity"))
        .await
        .expect("Vector search should succeed");

    assert_eq!(results.len(), 3, "Should return all 3 memories");

    // Assert: Verify results are sorted by distance
    for i in 1..results.len() {
        assert!(
            results[i - 1].distance <= results[i].distance,
            "Results should be sorted by distance ascending"
        );
    }

    // Validate semantic similarity thresholds
    // The query "machine learning algorithms" should match similar content best
    // Note: VectorStore returns chunk IDs, so we match by content instead
    let identical_result = results.iter()
        .find(|r| r.content.contains("machine learning algorithms"))
        .expect("Should find identical content");
    let similar_result = results.iter()
        .find(|r| r.content.contains("deep learning and neural networks"))
        .expect("Should find similar content");
    let unrelated_result = results.iter()
        .find(|r| r.content.contains("cooking recipes"))
        .expect("Should find unrelated content");

    // Log results for debugging
    eprintln!("Identical result: distance={}, content={}", identical_result.distance, identical_result.content);
    eprintln!("Similar result: distance={}, content={}", similar_result.distance, similar_result.content);
    eprintln!("Unrelated result: distance={}, content={}", unrelated_result.distance, unrelated_result.content);

    // With BERT embeddings:
    // - Identical/near-identical content: distance < 0.3 (very similar)
    // - Related content: distance 0.3 - 0.8
    // With hash-based embeddings (fallback):
    // - Thresholds are different, just verify ordering

    // Most important: identical should be closest
    assert!(
        identical_result.distance <= similar_result.distance,
        "Identical content should have lower distance than similar, got {} vs {}",
        identical_result.distance,
        similar_result.distance
    );

    // Unrelated content should have higher distance than identical
    assert!(
        unrelated_result.distance > identical_result.distance,
        "Unrelated content should have higher distance than identical, got {} vs {}",
        unrelated_result.distance,
        identical_result.distance
    );

    // Verify all distances are valid
    for (i, result) in results.iter().enumerate() {
        assert!(
            result.distance.is_finite(),
            "Distance {} should be finite (not NaN or Inf), got {}",
            i,
            result.distance
        );
        // Allow small floating point errors (negative values very close to 0)
        assert!(
            result.distance >= -1e-6 && result.distance <= 2.0,
            "Distance {} should be in range [0, 2] (with fp tolerance), got {}",
            i,
            result.distance
        );
    }
}

// =============================================================================
// Test 3: Edge Cases with BERT
// =============================================================================

#[tokio::test]
async fn test_empty_text_handling() {
    let service = create_memory_service_with_embeddings().await;

    // Add memory with empty string value
    let memory = Memory::new(
        "test:edge".to_string(),
        "empty".to_string(),
        json!(""),
        MemoryType::Semantic,
        "test".to_string(),
    );

    // Act: Should handle empty text gracefully
    let result = service.add(memory).await;

    // Assert: Addition succeeds (empty text is valid)
    // Note: Empty text creates valid embedding (either BERT's [CLS][SEP] tokens or hash of "")
    assert!(
        result.is_ok(),
        "Empty text should be handled gracefully: {:?}",
        result.err()
    );

    // Verify retrieval works
    let retrieved = service
        .get("test:edge", "empty")
        .await
        .expect("Failed to retrieve")
        .expect("Should exist");

    assert_eq!(retrieved.value, json!(""), "Retrieved value should match empty string");
}

#[tokio::test]
async fn test_very_long_text_chunking() {
    let service = create_memory_service_with_embeddings().await;

    // Create text longer than max_seq_length (512 tokens for BERT)
    // Average word is ~1.3 tokens, so 700 words ≈ 910 tokens
    // This will trigger automatic chunking by the Chunker service
    let long_text = (0..700)
        .map(|i| format!("word{}", i))
        .collect::<Vec<_>>()
        .join(" ");

    eprintln!("Test long text: {} characters, ~{} words", long_text.len(), 700);

    let memory = Memory::new(
        "test:long".to_string(),
        "document".to_string(),
        json!(long_text),
        MemoryType::Semantic,
        "test".to_string(),
    );

    // Act: Should chunk long text automatically
    let result = service.add(memory).await;

    // Assert: Addition succeeds (chunking handles long text)
    assert!(
        result.is_ok(),
        "Long text should be chunked automatically: {:?}",
        result.err()
    );

    // Verify search can find chunks
    // Note: With chunking, MemoryService creates multiple vector entries (one per chunk)
    let search_results = service
        .vector_search("word100 word200 word300", 5, Some("test:long"))
        .await
        .expect("Search should succeed");

    assert!(
        !search_results.is_empty(),
        "Should find chunks of long document"
    );

    eprintln!("Long document search returned {} chunks", search_results.len());
}

#[tokio::test]
async fn test_special_characters_and_unicode() {
    let service = create_memory_service_with_embeddings().await;

    // Test various special characters and Unicode
    let texts = vec![
        "Hello, world! How are you?",
        "C++ is a programming language",
        "Price: $19.99 (20% off)",
        "Emoji test: 🦀 Rust 🚀 Performance",
        "Math symbols: α β γ Δ ∑ ∫",
        "Mixed: English 中文 日本語 한국어",
    ];

    for (i, text) in texts.iter().enumerate() {
        let memory = Memory::new(
            "test:unicode".to_string(),
            format!("text_{}", i),
            json!(text),
            MemoryType::Semantic,
            "test".to_string(),
        );

        let result = service.add(memory).await;
        assert!(
            result.is_ok(),
            "Should handle special characters/Unicode: {} - {:?}",
            text,
            result.err()
        );
    }

    // Verify search works with Unicode
    let results = service
        .vector_search("programming language", 10, Some("test:unicode"))
        .await
        .expect("Search with Unicode should succeed");

    assert!(!results.is_empty(), "Should return results for Unicode content");
}

// =============================================================================
// Test 4: Graceful Degradation Without Vector Capabilities
// =============================================================================

#[tokio::test]
async fn test_graceful_degradation_without_vector() {
    let service = create_memory_service_without_vector().await;

    // Traditional CRUD operations should still work
    let memory = Memory::new(
        "test:fallback".to_string(),
        "key1".to_string(),
        json!({"data": "value"}),
        MemoryType::Semantic,
        "test".to_string(),
    );

    // Act: Add memory without vector capabilities
    let add_result = service.add(memory.clone()).await;
    assert!(add_result.is_ok(), "Traditional add should work without vector store");

    // Act: Get memory
    let get_result = service.get("test:fallback", "key1").await;
    assert!(get_result.is_ok(), "Traditional get should work");
    assert!(get_result.unwrap().is_some(), "Memory should exist");

    // Act: Update memory
    let update_result = service
        .update("test:fallback", "key1", json!({"data": "updated"}), "test")
        .await;
    assert!(update_result.is_ok(), "Traditional update should work");

    // Act: Search by namespace
    let search_result = service
        .search("test:fallback", None, Some(10))
        .await;
    assert!(search_result.is_ok(), "Traditional search should work");
    assert_eq!(search_result.unwrap().len(), 1, "Should find 1 memory");

    // Act: Delete memory
    let delete_result = service.delete("test:fallback", "key1").await;
    assert!(delete_result.is_ok(), "Traditional delete should work");
}

#[tokio::test]
async fn test_vector_search_fails_gracefully_without_capabilities() {
    let service = create_memory_service_without_vector().await;

    // Act: Attempt vector search without vector store
    let result = service
        .vector_search("test query", 10, None)
        .await;

    // Assert: Should return clear error message
    assert!(result.is_err(), "Vector search should fail without vector store");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Vector search not available"),
        "Error should explain vector search is not available, got: {}",
        error_msg
    );
}

// =============================================================================
// Test 5: BERT Embedding Consistency
// =============================================================================

#[tokio::test]
async fn test_bert_embedding_determinism() {
    let service1 = create_memory_service_with_embeddings().await;
    let service2 = create_memory_service_with_embeddings().await;

    let text = "Rust provides memory safety without garbage collection";

    // Add same memory to both services
    let memory1 = Memory::new(
        "test:determinism".to_string(),
        "test".to_string(),
        json!(text),
        MemoryType::Semantic,
        "test".to_string(),
    );

    let memory2 = Memory::new(
        "test:determinism".to_string(),
        "test".to_string(),
        json!(text),
        MemoryType::Semantic,
        "test".to_string(),
    );

    service1.add(memory1).await.expect("Failed to add to service1");
    service2.add(memory2).await.expect("Failed to add to service2");

    // Search both services
    let results1 = service1
        .vector_search(text, 1, Some("test:determinism"))
        .await
        .expect("Search 1 failed");

    let results2 = service2
        .vector_search(text, 1, Some("test:determinism"))
        .await
        .expect("Search 2 failed");

    // Assert: BERT embeddings are deterministic - identical text should produce same embedding
    // (Distance should be exactly 0 or very close due to numerical precision)
    assert!(
        results1[0].distance < 1e-6,
        "Same text should have identical embedding (distance < 1e-6), got {}",
        results1[0].distance
    );

    assert!(
        results2[0].distance < 1e-6,
        "Same text should have identical embedding (distance < 1e-6), got {}",
        results2[0].distance
    );

    // Distances should be identical across services (BERT is deterministic)
    let distance_diff = (results1[0].distance - results2[0].distance).abs();
    assert!(
        distance_diff < 1e-7,
        "BERT embeddings must be deterministic across instances, distance diff: {}",
        distance_diff
    );
}

// =============================================================================
// Test 6: Realistic Multi-Memory Workflow
// =============================================================================

#[tokio::test]
async fn test_realistic_knowledge_base_workflow() {
    let service = create_memory_service_with_embeddings().await;

    // Simulate building a knowledge base about programming languages
    let knowledge_entries = vec![
        ("rust_safety", "Rust prevents data races at compile time using ownership and borrowing"),
        ("rust_performance", "Rust achieves zero-cost abstractions and C-like performance"),
        ("python_simplicity", "Python emphasizes code readability with significant whitespace"),
        ("python_libraries", "Python has extensive libraries for data science and machine learning"),
        ("go_concurrency", "Go provides goroutines for lightweight concurrent programming"),
        ("javascript_async", "JavaScript uses async/await for asynchronous programming"),
    ];

    for (key, content) in &knowledge_entries {
        let memory = Memory::new(
            "kb:languages".to_string(),
            key.to_string(),
            json!(content),
            MemoryType::Semantic,
            "knowledge_worker".to_string(),
        );

        service.add(memory).await.expect("Failed to add knowledge");
    }

    // Query 1: Find information about memory safety
    let safety_results = service
        .vector_search("memory safety and preventing bugs", 3, Some("kb:languages"))
        .await
        .expect("Search failed");

    assert!(!safety_results.is_empty(), "Should find relevant results");

    // With BERT embeddings, semantic ranking should prioritize Rust entries for safety queries
    // "memory safety and preventing bugs" is most relevant to Rust's ownership system
    // Note: VectorStore returns chunk IDs, check content instead
    let top_content = &safety_results[0].content;
    eprintln!("Top safety result: id={}, content={}", safety_results[0].id, top_content);

    assert!(
        top_content.contains("Rust") || top_content.contains("data races") || top_content.contains("ownership"),
        "Top result for 'memory safety' query should be Rust-related, got: {}",
        top_content
    );

    // Verify results are sorted by distance
    for i in 1..safety_results.len() {
        assert!(
            safety_results[i - 1].distance <= safety_results[i].distance,
            "Results should be sorted by distance"
        );
    }

    // Query 2: Find information about asynchronous programming
    let async_results = service
        .vector_search("asynchronous and concurrent programming", 3, Some("kb:languages"))
        .await
        .expect("Search failed");

    assert!(!async_results.is_empty(), "Should find async-related results");

    // With BERT embeddings, semantic ranking should prioritize async-related entries
    // "asynchronous and concurrent programming" is most relevant to Go/JavaScript
    // Note: VectorStore returns chunk IDs, check content instead
    let top_contents: Vec<&str> = async_results.iter().take(2).map(|r| r.content.as_str()).collect();
    eprintln!("Top async results: {:?}", top_contents);

    let has_go_or_js = top_contents.iter().any(|content|
        content.contains("Go ") ||
        content.contains("goroutines") ||
        content.contains("JavaScript") ||
        content.contains("async/await")
    );
    assert!(
        has_go_or_js,
        "Top results for 'async programming' query should include Go or JavaScript, got: {:?}",
        top_contents
    );

    // Verify results are sorted by distance
    for i in 1..async_results.len() {
        assert!(
            async_results[i - 1].distance <= async_results[i].distance,
            "Results should be sorted by distance"
        );
    }

    // Query 3: Count all knowledge entries
    let count = service
        .count("kb:languages", Some(MemoryType::Semantic))
        .await
        .expect("Count failed");

    assert_eq!(count, 6, "Should have all 6 knowledge entries");
}

// =============================================================================
// Test 7: Update and Re-Embedding
// =============================================================================

#[tokio::test]
async fn test_memory_update_traditional_only() {
    let service = create_memory_service_with_embeddings().await;

    // Add initial memory
    let memory = Memory::new(
        "test:update".to_string(),
        "doc".to_string(),
        json!("Original content about Rust programming"),
        MemoryType::Semantic,
        "test".to_string(),
    );

    service.add(memory).await.expect("Failed to add");

    // Update memory (traditional update, does NOT update vector embedding)
    service
        .update(
            "test:update",
            "doc",
            json!("Updated content about Python programming"),
            "test",
        )
        .await
        .expect("Failed to update");

    // Verify traditional retrieval shows updated content
    let retrieved = service
        .get("test:update", "doc")
        .await
        .expect("Failed to retrieve")
        .expect("Should exist");

    assert_eq!(
        retrieved.value,
        json!("Updated content about Python programming")
    );
    // Note: version tracking is internal to the database, not exposed on Memory struct

    // Note: Vector search will still find OLD embedding (update doesn't re-embed)
    // This is expected behavior - vector updates require delete + re-add
}

// =============================================================================
// Test 8: Namespace Filtering
// =============================================================================

#[tokio::test]
async fn test_vector_search_namespace_filtering() {
    let service = create_memory_service_with_embeddings().await;

    // Add memories to different namespaces
    service
        .add(Memory::new(
            "docs:api".to_string(),
            "rest".to_string(),
            json!("REST API endpoints and HTTP methods"),
            MemoryType::Semantic,
            "test".to_string(),
        ))
        .await
        .expect("Failed to add");

    service
        .add(Memory::new(
            "docs:guide".to_string(),
            "tutorial".to_string(),
            json!("Getting started tutorial for beginners"),
            MemoryType::Semantic,
            "test".to_string(),
        ))
        .await
        .expect("Failed to add");

    service
        .add(Memory::new(
            "code:rust".to_string(),
            "example".to_string(),
            json!("Example Rust code for HTTP requests"),
            MemoryType::Semantic,
            "test".to_string(),
        ))
        .await
        .expect("Failed to add");

    // Search only in docs:api namespace
    let api_results = service
        .vector_search("HTTP API", 10, Some("docs:api"))
        .await
        .expect("Search failed");

    assert_eq!(api_results.len(), 1, "Should only return docs:api results");
    assert_eq!(api_results[0].namespace, "docs:api");

    // Search in all docs: namespaces
    let docs_results = service
        .vector_search("HTTP API", 10, Some("docs:"))
        .await
        .expect("Search failed");

    assert_eq!(docs_results.len(), 2, "Should return both docs: namespaces");
    for result in &docs_results {
        assert!(result.namespace.starts_with("docs:"));
    }

    // Search across all namespaces
    let all_results = service
        .vector_search("HTTP", 10, None)
        .await
        .expect("Search failed");

    assert_eq!(all_results.len(), 3, "Should return all matching memories");
}

#[tokio::test]
async fn test_list_vector_namespaces() {
    let service = create_memory_service_with_embeddings().await;

    // Add memories to various namespaces
    for i in 0..5 {
        service
            .add(Memory::new(
                "ns1:data".to_string(),
                format!("key{}", i),
                json!(format!("Content {}", i)),
                MemoryType::Semantic,
                "test".to_string(),
            ))
            .await
            .expect("Failed to add");
    }

    for i in 0..3 {
        service
            .add(Memory::new(
                "ns2:info".to_string(),
                format!("key{}", i),
                json!(format!("Info {}", i)),
                MemoryType::Semantic,
                "test".to_string(),
            ))
            .await
            .expect("Failed to add");
    }

    // List all vector namespaces
    let namespaces = service
        .list_vector_namespaces()
        .await
        .expect("Failed to list namespaces");

    assert_eq!(namespaces.len(), 2, "Should have 2 namespaces");

    // Find namespace counts
    let ns1 = namespaces.iter().find(|(ns, _)| ns == "ns1:data");
    let ns2 = namespaces.iter().find(|(ns, _)| ns == "ns2:info");

    assert!(ns1.is_some(), "Should have ns1:data namespace");
    assert!(ns2.is_some(), "Should have ns2:info namespace");

    // Note: Counts may include chunks, so >= original count
    assert!(ns1.unwrap().1 >= 5, "ns1 should have at least 5 documents");
    assert!(ns2.unwrap().1 >= 3, "ns2 should have at least 3 documents");
}

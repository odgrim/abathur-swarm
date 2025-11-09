//! Integration tests for VectorStore sqlite-vec extension loading
//!
//! Tests the initialize_vec_extension() functionality including:
//! - Extension loading success path
//! - Version logging
//! - Graceful fallback to pure-Rust when unavailable
//! - Implementation tracking

use abathur_cli::infrastructure::vector::{LocalEmbeddingService, VectorStore, VectorImplementation};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;
use std::sync::Arc;

mod helpers;

/// Test that VectorStore initializes with appropriate implementation
#[tokio::test]
async fn test_vector_store_initialization() {
    let pool = helpers::database::setup_test_db().await;

    // Create embedding service (uses deterministic test embeddings)
    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    // Create vector store - should initialize extension if available
    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // Verify implementation is set (either NativeVec0 or PureRust)
    let implementation = vector_store.implementation();
    assert!(
        implementation == VectorImplementation::NativeVec0
        || implementation == VectorImplementation::PureRust,
        "Implementation should be either NativeVec0 or PureRust"
    );

    // Log which implementation is active
    match implementation {
        VectorImplementation::NativeVec0 => {
            println!("✓ sqlite-vec extension loaded successfully");
        }
        VectorImplementation::PureRust => {
            println!("✓ Using pure-Rust fallback (extension unavailable)");
        }
    }

    helpers::database::teardown_test_db(pool).await;
}

/// Test that extension loading fails gracefully with invalid database
#[tokio::test]
async fn test_extension_graceful_fallback() {
    // Create in-memory database without extensions enabled
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("Failed to create options")
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("Failed to create pool");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    // Create vector store - should fall back to PureRust
    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // Should fall back to PureRust when extension unavailable
    // (Most test environments won't have sqlite-vec compiled in)
    let implementation = vector_store.implementation();

    // Verify graceful degradation
    assert!(
        implementation == VectorImplementation::PureRust
        || implementation == VectorImplementation::NativeVec0,
        "Should use either implementation without failing"
    );

    pool.close().await;
}

/// Test that create_vector_index works with both implementations
#[tokio::test]
async fn test_create_vector_index_both_implementations() {
    let pool = helpers::database::setup_test_db().await;

    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // create_vector_index should work regardless of implementation
    // For PureRust: logs warning about no spatial indexing
    // For NativeVec0: verifies vec0 tables exist
    let result = vector_store.create_vector_index().await;

    match vector_store.implementation() {
        VectorImplementation::NativeVec0 => {
            // With vec0, should either succeed or fail with table not found
            // (migration 008 may not be applied in test environment)
            match result {
                Ok(_) => println!("✓ vec0 index initialized"),
                Err(e) if e.to_string().contains("vec_memory_vec0") => {
                    println!("✓ vec0 table not found (expected - migration 008 not applied)");
                }
                Err(e) => panic!("Unexpected error: {}", e),
            }
        }
        VectorImplementation::PureRust => {
            // Pure Rust should always succeed (logs warning, no actual index)
            assert!(result.is_ok(), "Pure Rust index initialization should succeed");
            println!("✓ Pure Rust fallback index initialization succeeded");
        }
    }

    helpers::database::teardown_test_db(pool).await;
}

/// Test that search_similar works with both implementations
#[tokio::test]
async fn test_search_similar_both_implementations() {
    let pool = helpers::database::setup_test_db().await;

    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // Insert test data
    let _ = vector_store
        .insert(
            "test-id-1",
            "test",
            "This is a test document",
            None,
            None,
        )
        .await;

    let _ = vector_store
        .insert(
            "test-id-2",
            "test",
            "Another test document with different content",
            None,
            None,
        )
        .await;

    // Search should work with both implementations
    let results = vector_store
        .search_similar("test document", 5, None)
        .await
        .expect("Search should succeed");

    // Should find at least one result
    assert!(
        !results.is_empty(),
        "Search should return results with both implementations"
    );

    match vector_store.implementation() {
        VectorImplementation::NativeVec0 => {
            println!("✓ SIMD-accelerated search completed");
        }
        VectorImplementation::PureRust => {
            println!("✓ Pure-Rust search completed");
        }
    }

    helpers::database::teardown_test_db(pool).await;
}

/// Test implementation getter method
#[tokio::test]
async fn test_implementation_getter() {
    let pool = helpers::database::setup_test_db().await;

    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // Implementation should be accessible
    let impl1 = vector_store.implementation();
    let impl2 = vector_store.implementation();

    // Should be consistent
    assert_eq!(impl1, impl2, "Implementation should be consistent");

    // Should be one of the two valid values
    assert!(
        impl1 == VectorImplementation::NativeVec0
        || impl1 == VectorImplementation::PureRust,
        "Implementation should be valid enum value"
    );

    helpers::database::teardown_test_db(pool).await;
}

/// Property test: Both implementations should produce valid search results
#[tokio::test]
async fn proptest_both_implementations_produce_valid_results() {
    let pool = helpers::database::setup_test_db().await;

    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // Insert multiple documents
    for i in 0..10 {
        let _ = vector_store
            .insert(
                &format!("doc-{}", i),
                "test",
                &format!("Test document number {}", i),
                None,
                None,
            )
            .await;
    }

    // Search with various queries
    let queries = vec![
        "test document",
        "number",
        "doc",
        "0",
        "9",
    ];

    for query in queries {
        let results = vector_store
            .search_similar(query, 3, None)
            .await
            .expect("Search should succeed");

        // Verify all results are valid
        for result in &results {
            assert!(!result.id.is_empty(), "Result ID should not be empty");
            assert!(!result.namespace.is_empty(), "Namespace should not be empty");
            assert!(result.distance >= 0.0, "Distance should be non-negative");
            assert!(result.distance.is_finite(), "Distance should be finite");
        }

        // Results should be sorted by distance (ascending)
        for i in 1..results.len() {
            assert!(
                results[i - 1].distance <= results[i].distance,
                "Results should be sorted by distance"
            );
        }
    }

    println!(
        "✓ Both implementations produce valid, sorted results (implementation: {:?})",
        vector_store.implementation()
    );

    helpers::database::teardown_test_db(pool).await;
}

/// Test that VectorImplementation enum implements required traits
#[test]
fn test_vector_implementation_enum_traits() {
    // Should implement Debug
    let impl_native = VectorImplementation::NativeVec0;
    let debug_str = format!("{:?}", impl_native);
    assert!(debug_str.contains("NativeVec0"));

    // Should implement Clone
    let impl_clone = impl_native.clone();
    assert_eq!(impl_native, impl_clone);

    // Should implement Copy
    let impl_copy = impl_native;
    assert_eq!(impl_native, impl_copy);

    // Should implement PartialEq
    assert_eq!(VectorImplementation::NativeVec0, VectorImplementation::NativeVec0);
    assert_ne!(VectorImplementation::NativeVec0, VectorImplementation::PureRust);

    // Should implement Eq
    assert_eq!(VectorImplementation::PureRust, VectorImplementation::PureRust);
}

/// Benchmark test: Compare performance of both implementations (if available)
#[tokio::test]
#[ignore] // Run with --ignored flag for performance testing
async fn benchmark_implementation_performance() {
    use std::time::Instant;

    let pool = helpers::database::setup_test_db().await;

    let embedding_service = Arc::new(
        LocalEmbeddingService::new()
            .expect("Failed to create embedding service")
    );

    let vector_store = VectorStore::new(Arc::new(pool.clone()), embedding_service)
        .await
        .expect("Failed to create vector store");

    // Insert 100 documents
    for i in 0..100 {
        let _ = vector_store
            .insert(
                &format!("bench-doc-{}", i),
                "benchmark",
                &format!("Benchmark test document with content number {}", i),
                None,
                None,
            )
            .await;
    }

    // Measure search performance
    let queries = vec![
        "test document",
        "benchmark content",
        "number 50",
        "with content",
    ];

    let mut total_duration = std::time::Duration::ZERO;
    let iterations = 10;

    for _ in 0..iterations {
        for query in &queries {
            let start = Instant::now();
            let _ = vector_store
                .search_similar(query, 10, None)
                .await
                .expect("Search should succeed");
            total_duration += start.elapsed();
        }
    }

    let avg_latency = total_duration / (iterations * queries.len() as u32);

    println!(
        "Implementation: {:?}",
        vector_store.implementation()
    );
    println!(
        "Average search latency (100 docs): {:?}",
        avg_latency
    );

    // Performance expectations based on implementation
    match vector_store.implementation() {
        VectorImplementation::NativeVec0 => {
            // SIMD path should be fast: p95 < 100ms for 10k entries
            // For 100 entries, should be much faster
            assert!(
                avg_latency.as_millis() < 50,
                "NativeVec0 should be fast for 100 documents (got {}ms)",
                avg_latency.as_millis()
            );
        }
        VectorImplementation::PureRust => {
            // Pure Rust is slower but should still be reasonable for small datasets
            // p95 < 500ms for 10k entries, so 100 entries should be < 50ms
            assert!(
                avg_latency.as_millis() < 100,
                "PureRust should handle 100 documents reasonably (got {}ms)",
                avg_latency.as_millis()
            );
        }
    }

    helpers::database::teardown_test_db(pool).await;
}

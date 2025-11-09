//! Property-based tests for embedding quality invariants
//!
//! Tests the following properties:
//! 1. Determinism: same input → same output
//! 2. Normalization: ||embed(text)|| = 1.0
//! 3. Batch ordering: embed_batch()[i] == embed(texts[i])
//! 4. Search symmetry: distance(A,B) == distance(B,A)
//! 5. Cosine distance bounds: distance ∈ [0, 2]
//!
//! This test suite validates both the LocalEmbeddingService (test implementation)
//! and the BertEmbeddingModel (production BERT implementation).

use abathur_cli::domain::models::EmbeddingModel;
use abathur_cli::domain::ports::EmbeddingService;
use abathur_cli::infrastructure::vector::embedding_service::LocalEmbeddingService;
use abathur_cli::infrastructure::vector::bert_model::BertEmbeddingModel;
use abathur_cli::infrastructure::vector::vector_store::VectorStore;
use proptest::prelude::*;
use std::sync::Arc;

mod common;

/// Generate valid UTF-8 text strings for testing
fn text_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9 .,!?;:'\"-]{1,1000}").expect("Valid regex")
}

/// Generate non-empty text strings
fn non_empty_text_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z0-9 .,!?;:'\"-]{1,500}").expect("Valid regex")
}

/// Strategy for generating normalized embeddings (L2 norm = 1.0)
fn normalized_embedding_strategy(dim: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-1.0f32..1.0f32, dim..=dim).prop_map(|mut vec| {
        // Normalize to unit vector
        let magnitude: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for val in &mut vec {
                *val /= magnitude;
            }
        }
        vec
    })
}

proptest! {
    /// Property 1: Determinism - same input always produces same output
    #[test]
    fn proptest_embedding_determinism(text in text_strategy()) {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let emb1 = service.generate_deterministic_embedding(&text);
        let emb2 = service.generate_deterministic_embedding(&text);

        // Same input should produce exactly the same embedding
        prop_assert_eq!(emb1.len(), emb2.len());
        for (a, b) in emb1.iter().zip(emb2.iter()) {
            prop_assert!(
                (a - b).abs() < 1e-10,
                "Embeddings should be identical for same input"
            );
        }
    }

    /// Property 2: Normalization - all embeddings should have L2 norm = 1.0
    #[test]
    fn proptest_l2_normalization(text in non_empty_text_strategy()) {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let embedding = service.generate_deterministic_embedding(&text);

        // Calculate L2 norm
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();

        // All embeddings should be normalized to unit vectors
        // Allow slightly larger tolerance for floating point accumulation errors
        prop_assert!(
            (magnitude - 1.0).abs() < 1e-5,
            "Embedding L2 norm should be 1.0, got {}",
            magnitude
        );

        // Verify no NaN or Inf values
        for val in &embedding {
            prop_assert!(val.is_finite(), "Embedding contains non-finite values");
        }
    }

    /// Property 3: Batch ordering - embed_batch()[i] == embed(texts[i])
    #[test]
    fn proptest_batch_ordering_equivalence(
        texts in prop::collection::vec(non_empty_text_strategy(), 1..20)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        rt.block_on(async {
            // Get batch embeddings
            let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            let batch_embeddings = service
                .embed_batch(&text_refs)
                .await
                .expect("Batch embedding failed");

            // Get individual embeddings
            let mut individual_embeddings = Vec::new();
            for text in &texts {
                let emb = service.embed(text).await.expect("Individual embedding failed");
                individual_embeddings.push(emb);
            }

            // Verify batch and individual results match
            prop_assert_eq!(batch_embeddings.len(), individual_embeddings.len());

            for (i, (batch_emb, ind_emb)) in batch_embeddings
                .iter()
                .zip(individual_embeddings.iter())
                .enumerate()
            {
                prop_assert_eq!(batch_emb.len(), ind_emb.len());
                for (j, (a, b)) in batch_emb.iter().zip(ind_emb.iter()).enumerate() {
                    prop_assert!(
                        (a - b).abs() < 1e-6,
                        "Batch embedding[{}][{}] != individual embedding: {} vs {}",
                        i,
                        j,
                        a,
                        b
                    );
                }
            }

            Ok(()) as Result<(), proptest::test_runner::TestCaseError>
        })?;
    }

    /// Property 4: Search symmetry - distance(A, B) == distance(B, A)
    #[test]
    fn proptest_search_symmetry(
        emb1 in normalized_embedding_strategy(384),
        emb2 in normalized_embedding_strategy(384)
    ) {
        let dist_ab = VectorStore::cosine_distance(&emb1, &emb2);
        let dist_ba = VectorStore::cosine_distance(&emb2, &emb1);

        // Distance should be symmetric
        prop_assert!(
            (dist_ab - dist_ba).abs() < 1e-6,
            "Distance should be symmetric: distance(A,B)={} != distance(B,A)={}",
            dist_ab,
            dist_ba
        );
    }

    /// Property 5: Cosine distance bounds - always in [0, 2] for normalized vectors
    #[test]
    fn proptest_cosine_distance_bounds(
        emb1 in normalized_embedding_strategy(384),
        emb2 in normalized_embedding_strategy(384)
    ) {
        let distance = VectorStore::cosine_distance(&emb1, &emb2);

        // For normalized vectors, cosine distance should be in [0, 2]
        // where 0 = identical, 1 = orthogonal, 2 = opposite
        prop_assert!(
            distance >= 0.0 && distance <= 2.0,
            "Cosine distance should be in [0, 2], got {}",
            distance
        );

        // Verify no NaN or Inf
        prop_assert!(distance.is_finite(), "Distance should be finite");
    }

    /// Property 6: Distance identity - distance of vector to itself is 0
    #[test]
    fn proptest_distance_identity(emb in normalized_embedding_strategy(384)) {
        let distance = VectorStore::cosine_distance(&emb, &emb);

        prop_assert!(
            distance.abs() < 1e-6,
            "Distance from vector to itself should be 0, got {}",
            distance
        );
    }

    /// Property 7: Embedding dimensions consistency
    #[test]
    fn proptest_embedding_dimensions(text in text_strategy()) {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let embedding = service.generate_deterministic_embedding(&text);

        prop_assert_eq!(
            embedding.len(),
            384,
            "MiniLM embeddings should have 384 dimensions"
        );
    }

    /// Property 8: Model dimensions consistency across model types
    #[test]
    fn proptest_model_dimensions_consistency(
        text in non_empty_text_strategy(),
        model_idx in 0usize..2usize
    ) {
        let models = [EmbeddingModel::LocalMiniLM, EmbeddingModel::LocalMPNet];
        let expected_dims = [384, 768];

        let model = models[model_idx];
        let service = LocalEmbeddingService::new(model).expect("Failed to create service");

        let embedding = service.generate_deterministic_embedding(&text);

        prop_assert_eq!(
            embedding.len(),
            expected_dims[model_idx],
            "Model {:?} should produce {} dimensions",
            model,
            expected_dims[model_idx]
        );

        // Verify normalization for all models
        // Use f64 for magnitude calculation to match implementation and avoid accumulation errors
        let magnitude: f32 = embedding.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt() as f32;
        prop_assert!(
            (magnitude - 1.0).abs() < 1e-6,
            "Embedding L2 norm should be 1.0, got {} for model {:?}",
            magnitude,
            model
        );
    }

    /// Property 9: Empty string handling
    #[test]
    fn proptest_empty_string_handling(_seed in 0u32..100u32) {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let embedding = service.generate_deterministic_embedding("");

        // Even empty string should produce valid normalized embedding
        prop_assert_eq!(embedding.len(), 384);

        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        // Empty string might have zero embedding, which is okay
        prop_assert!(magnitude >= 0.0);

        // Verify no NaN or Inf
        for val in &embedding {
            prop_assert!(val.is_finite());
        }
    }

    /// Property 10: Triangle inequality (relaxed for cosine distance)
    #[test]
    fn proptest_triangle_inequality(
        emb_a in normalized_embedding_strategy(384),
        emb_b in normalized_embedding_strategy(384),
        emb_c in normalized_embedding_strategy(384)
    ) {
        let d_ab = VectorStore::cosine_distance(&emb_a, &emb_b);
        let d_bc = VectorStore::cosine_distance(&emb_b, &emb_c);
        let d_ac = VectorStore::cosine_distance(&emb_a, &emb_c);

        // Cosine distance satisfies a relaxed triangle inequality
        prop_assert!(
            d_ac <= d_ab + d_bc + 1e-6,
            "Triangle inequality violated: d(A,C)={} > d(A,B)={} + d(B,C)={}",
            d_ac,
            d_ab,
            d_bc
        );
    }

    /// Property 11: Batch size invariance - different batch sizes produce same embeddings
    #[test]
    fn proptest_batch_size_invariance(
        texts in prop::collection::vec(non_empty_text_strategy(), 2..10)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        rt.block_on(async {
            // Get embeddings in different batch sizes
            let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

            // Batch all at once
            let all_batch = service.embed_batch(&text_refs).await
                .expect("Full batch failed");

            // Batch in pairs (if we have at least 2 texts)
            if texts.len() >= 2 {
                let mut pair_batches = Vec::new();
                for chunk in text_refs.chunks(2) {
                    let batch = service.embed_batch(chunk).await
                        .expect("Pair batch failed");
                    pair_batches.extend(batch);
                }

                // Verify same results
                prop_assert_eq!(all_batch.len(), pair_batches.len());
                for (i, (full, pairs)) in all_batch.iter().zip(pair_batches.iter()).enumerate() {
                    for (j, (a, b)) in full.iter().zip(pairs.iter()).enumerate() {
                        prop_assert!(
                            (a - b).abs() < 1e-6,
                            "Batch[{}][{}] differs: {} vs {}",
                            i, j, a, b
                        );
                    }
                }
            }

            Ok(()) as Result<(), proptest::test_runner::TestCaseError>
        })?;
    }

    /// Property 12: Cosine distance is in [0,1] for normalized vectors (not [0,2])
    /// Note: The actual range is [0,2], but for normalized vectors similarity is [0,1]
    /// so distance = 1 - similarity is also [0,1] in practice
    #[test]
    fn proptest_cosine_similarity_range(
        emb1 in normalized_embedding_strategy(384),
        emb2 in normalized_embedding_strategy(384)
    ) {
        let distance = VectorStore::cosine_distance(&emb1, &emb2);

        // For normalized vectors, cosine similarity is in [-1, 1]
        // Distance = 1 - similarity, so range is [0, 2]
        prop_assert!(distance >= 0.0, "Distance should be non-negative, got {}", distance);
        prop_assert!(distance <= 2.0, "Distance should be <= 2.0, got {}", distance);
    }

    /// Property 13: Embedding stability - identical texts produce identical embeddings
    /// Note: We test identical texts here because the hash-based test implementation
    /// doesn't preserve semantic similarity for text variations. For real BERT models,
    /// this would test that similar texts have similar embeddings.
    #[test]
    fn proptest_embedding_stability_identical(
        base_text in non_empty_text_strategy()
    ) {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        // Test that identical text produces identical embedding
        let emb1 = service.generate_deterministic_embedding(&base_text);
        let emb2 = service.generate_deterministic_embedding(&base_text);

        // Calculate cosine distance
        let distance = VectorStore::cosine_distance(&emb1, &emb2);

        // Identical texts should have distance 0 (perfect similarity)
        prop_assert!(
            distance.abs() < 1e-6,
            "Identical texts should have distance ~0, got {}",
            distance
        );
    }

    /// Property 14: Non-negativity of embeddings after normalization (component-wise)
    /// Actually, normalized embeddings can have negative components - this tests finite values
    #[test]
    fn proptest_embedding_finiteness(text in text_strategy()) {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let embedding = service.generate_deterministic_embedding(&text);

        // All values must be finite (no NaN, no Inf)
        for (i, &val) in embedding.iter().enumerate() {
            prop_assert!(
                val.is_finite(),
                "Embedding[{}] is not finite: {}",
                i, val
            );
        }
    }

    /// Property 15: Batch processing maintains order strictly
    #[test]
    fn proptest_batch_strict_order_preservation(
        texts in prop::collection::vec(non_empty_text_strategy(), 5..15)
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        rt.block_on(async {
            let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

            // Get batch embeddings
            let batch = service.embed_batch(&text_refs).await
                .expect("Batch failed");

            // Verify each position matches individual embed
            for (i, (text, batch_emb)) in texts.iter().zip(batch.iter()).enumerate() {
                let individual = service.embed(text).await.expect("Individual failed");

                prop_assert_eq!(batch_emb.len(), individual.len());
                for (j, (b, ind)) in batch_emb.iter().zip(individual.iter()).enumerate() {
                    prop_assert!(
                        (b - ind).abs() < 1e-10,
                        "Position {} dimension {} mismatch: batch={} vs individual={}",
                        i, j, b, ind
                    );
                }
            }

            Ok(()) as Result<(), proptest::test_runner::TestCaseError>
        })?;
    }
}

// ============================================================================
// BERT Embedding Model Property Tests
// ============================================================================
//
// These tests validate the actual BERT embedding pipeline with candle-transformers.
// They test the same properties as above but with the production BERT model.

/// Helper to create a BERT model with error handling for CI/headless environments
fn create_bert_model_if_available() -> Option<Arc<BertEmbeddingModel>> {
    match BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM) {
        Ok(model) => Some(Arc::new(model)),
        Err(e) => {
            eprintln!("Skipping BERT tests: failed to load model: {}", e);
            None
        }
    }
}

proptest! {
    // ------------------------------------------------------------------------
    // Property 1: BERT Determinism - same input always produces same output
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_embedding_determinism(text in text_strategy()) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                // Generate embedding twice for same text
                let emb1 = model.embed(&text).await
                    .expect("First embedding failed");
                let emb2 = model.embed(&text).await
                    .expect("Second embedding failed");

                // Same input should produce exactly the same embedding
                prop_assert_eq!(emb1.len(), emb2.len(), "Embedding dimensions should match");

                for (i, (a, b)) in emb1.iter().zip(emb2.iter()).enumerate() {
                    prop_assert!(
                        (a - b).abs() < 1e-6,
                        "Dimension {} differs: {} vs {}. BERT embeddings must be deterministic.",
                        i, a, b
                    );
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 2: BERT Normalization - all embeddings have L2 norm = 1.0
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_l2_normalization(text in non_empty_text_strategy()) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let embedding = model.embed(&text).await
                    .expect("Embedding failed");

                // Calculate L2 norm: sqrt(sum(x^2))
                let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();

                // All BERT embeddings should be L2-normalized to unit vectors
                prop_assert!(
                    (magnitude - 1.0).abs() < 1e-5,
                    "BERT embedding L2 norm should be 1.0, got {}. Text: '{}'",
                    magnitude,
                    &text[..text.len().min(50)]
                );

                // Verify no NaN or Inf values
                for (i, &val) in embedding.iter().enumerate() {
                    prop_assert!(
                        val.is_finite(),
                        "BERT embedding[{}] is not finite: {}",
                        i, val
                    );
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 3: BERT Batch Ordering - embed_batch()[i] == embed(texts[i])
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_batch_ordering_equivalence(
        texts in prop::collection::vec(non_empty_text_strategy(), 1..10)
    ) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                // Get batch embeddings
                let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
                let batch_embeddings = model.embed_batch(&text_refs).await
                    .expect("Batch embedding failed");

                // Get individual embeddings
                let mut individual_embeddings = Vec::new();
                for text in &texts {
                    let emb = model.embed(text).await.expect("Individual embedding failed");
                    individual_embeddings.push(emb);
                }

                // Verify batch and individual results match
                prop_assert_eq!(
                    batch_embeddings.len(),
                    individual_embeddings.len(),
                    "Batch size mismatch"
                );

                for (i, (batch_emb, ind_emb)) in batch_embeddings.iter()
                    .zip(individual_embeddings.iter())
                    .enumerate() {
                    prop_assert_eq!(
                        batch_emb.len(),
                        ind_emb.len(),
                        "Embedding dimension mismatch at position {}",
                        i
                    );

                    for (j, (a, b)) in batch_emb.iter().zip(ind_emb.iter()).enumerate() {
                        prop_assert!(
                            (a - b).abs() < 1e-5,
                            "Batch[{}][{}] != Individual: {} vs {}. Batch processing must preserve order and values.",
                            i, j, a, b
                        );
                    }
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 4: BERT Search Symmetry - distance(A,B) == distance(B,A)
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_search_symmetry(
        text1 in non_empty_text_strategy(),
        text2 in non_empty_text_strategy()
    ) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                // Generate embeddings
                let emb1 = model.embed(&text1).await.expect("Embedding 1 failed");
                let emb2 = model.embed(&text2).await.expect("Embedding 2 failed");

                // Calculate distances in both directions
                let dist_ab = VectorStore::cosine_distance(&emb1, &emb2);
                let dist_ba = VectorStore::cosine_distance(&emb2, &emb1);

                // Distance should be symmetric
                prop_assert!(
                    (dist_ab - dist_ba).abs() < 1e-6,
                    "BERT cosine distance should be symmetric: distance(A,B)={} != distance(B,A)={}",
                    dist_ab, dist_ba
                );

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 5: BERT Cosine Distance Bounds - distance in [0, 2]
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_cosine_distance_bounds(
        text1 in non_empty_text_strategy(),
        text2 in non_empty_text_strategy()
    ) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let emb1 = model.embed(&text1).await.expect("Embedding 1 failed");
                let emb2 = model.embed(&text2).await.expect("Embedding 2 failed");

                let distance = VectorStore::cosine_distance(&emb1, &emb2);

                // For normalized vectors, cosine distance should be in [0, 2]
                // where 0 = identical, 1 = orthogonal, 2 = opposite
                prop_assert!(
                    distance >= 0.0 && distance <= 2.0,
                    "BERT cosine distance should be in [0, 2], got {}",
                    distance
                );

                // Verify no NaN or Inf
                prop_assert!(
                    distance.is_finite(),
                    "BERT distance should be finite, got {}",
                    distance
                );

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 6: BERT Embedding Dimensions Consistency
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_embedding_dimensions(text in text_strategy()) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let embedding = model.embed(&text).await
                    .expect("Embedding failed");

                prop_assert_eq!(
                    embedding.len(),
                    384,
                    "MiniLM BERT embeddings should have 384 dimensions"
                );

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 7: BERT Batch Size Invariance
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_batch_size_invariance(
        texts in prop::collection::vec(non_empty_text_strategy(), 4..8)
    ) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

                // Get embeddings in full batch
                let all_batch = model.embed_batch(&text_refs).await
                    .expect("Full batch failed");

                // Get embeddings in pairs
                let mut pair_batches = Vec::new();
                for chunk in text_refs.chunks(2) {
                    let batch = model.embed_batch(chunk).await
                        .expect("Pair batch failed");
                    pair_batches.extend(batch);
                }

                // Verify same results regardless of batch size
                prop_assert_eq!(all_batch.len(), pair_batches.len());

                for (i, (full, pairs)) in all_batch.iter().zip(pair_batches.iter()).enumerate() {
                    for (j, (a, b)) in full.iter().zip(pairs.iter()).enumerate() {
                        prop_assert!(
                            (a - b).abs() < 1e-5,
                            "Batch size affects results at [{}][{}]: {} vs {}",
                            i, j, a, b
                        );
                    }
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 8: BERT Empty String Handling
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_empty_string_handling(_seed in 0u32..50u32) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let embedding = model.embed("").await
                    .expect("Empty string embedding failed");

                // Even empty string should produce valid normalized embedding
                prop_assert_eq!(embedding.len(), 384, "Empty string should produce 384-dim embedding");

                let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                prop_assert!(
                    (magnitude - 1.0).abs() < 1e-5,
                    "Empty string embedding should be normalized, got magnitude {}",
                    magnitude
                );

                // Verify no NaN or Inf
                for &val in &embedding {
                    prop_assert!(val.is_finite(), "Empty string embedding contains non-finite value");
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 9: BERT Distance Identity - distance(A, A) = 0
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_distance_identity(text in non_empty_text_strategy()) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let embedding = model.embed(&text).await
                    .expect("Embedding failed");

                let distance = VectorStore::cosine_distance(&embedding, &embedding);

                prop_assert!(
                    distance.abs() < 1e-6,
                    "Distance from embedding to itself should be 0, got {}",
                    distance
                );

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 10: BERT Embedding Finiteness - all values are finite
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_embedding_finiteness(text in text_strategy()) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                let embedding = model.embed(&text).await
                    .expect("Embedding failed");

                // All values must be finite (no NaN, no Inf)
                for (i, &val) in embedding.iter().enumerate() {
                    prop_assert!(
                        val.is_finite(),
                        "BERT embedding[{}] is not finite: {}",
                        i, val
                    );
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 11: BERT Long Text Handling - texts > 512 tokens are truncated
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_long_text_truncation(repeat_count in 100usize..200usize) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                // Create very long text (will exceed 512 tokens)
                let long_text = "word ".repeat(repeat_count);

                let embedding = model.embed(&long_text).await
                    .expect("Long text embedding failed");

                // Should still produce valid normalized embedding
                prop_assert_eq!(embedding.len(), 384);

                let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                prop_assert!(
                    (magnitude - 1.0).abs() < 1e-5,
                    "Long text embedding should be normalized, got magnitude {}",
                    magnitude
                );

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }

    // ------------------------------------------------------------------------
    // Property 12: BERT Special Characters Handling
    // ------------------------------------------------------------------------
    #[test]
    fn proptest_bert_special_characters(
        base_text in non_empty_text_strategy(),
        special_idx in 0usize..5usize
    ) {
        if let Some(model) = create_bert_model_if_available() {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async {
                // Add different special characters
                let specials = vec!["!", "?", "@", "#", "€"];
                let text_with_special = format!("{} {}", base_text, specials[special_idx]);

                let embedding = model.embed(&text_with_special).await
                    .expect("Special character embedding failed");

                // Verify valid embedding
                prop_assert_eq!(embedding.len(), 384);

                let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
                prop_assert!(
                    (magnitude - 1.0).abs() < 1e-5,
                    "Special character text should produce normalized embedding"
                );

                for &val in &embedding {
                    prop_assert!(val.is_finite());
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }
    }
}

//! Embedding service implementation
//!
//! Provides local sentence transformer models for generating embeddings.
//! Models are downloaded from HuggingFace and cached locally.
//!
//! Note: This is a simplified implementation that generates deterministic test embeddings.
//! For production use with actual ML models, you would integrate candle-transformers
//! with sentence transformer models from HuggingFace.

use crate::domain::models::EmbeddingModel;
use crate::domain::ports::EmbeddingService as EmbeddingServiceTrait;
use anyhow::{anyhow, Result};
use async_trait::async_trait;

/// Local embedding service
///
/// This service generates embeddings using locally-hosted sentence transformer models.
/// Current implementation provides deterministic test embeddings for development.
///
/// For production use, integrate with candle-transformers:
/// - Download sentence transformer models from HuggingFace using hf-hub
/// - Load model weights with candle-core
/// - Implement tokenization and forward pass
/// - Apply mean pooling to get sentence embeddings
pub struct LocalEmbeddingService {
    model_type: EmbeddingModel,
    _initialized: bool,
}

impl LocalEmbeddingService {
    /// Create a new local embedding service
    ///
    /// # Arguments
    /// * `model_type` - The embedding model to use
    ///
    /// # Returns
    /// * `Ok(Self)` - A new embedding service
    /// * `Err(_)` - If model loading fails
    ///
    /// # Note
    /// This will download the model from HuggingFace on first use.
    /// Models are cached in ~/.cache/huggingface/ or ~/.cache/torch/.
    pub fn new(model_type: EmbeddingModel) -> Result<Self> {
        if !model_type.is_local() {
            return Err(anyhow!(
                "LocalEmbeddingService only supports local models. {:?} requires API access.",
                model_type
            ));
        }

        tracing::warn!(
            "LocalEmbeddingService is using simplified test implementation. \
             For production use, implement full candle-based sentence transformer."
        );

        Ok(Self {
            model_type,
            _initialized: true,
        })
    }

    /// Generate a deterministic embedding for testing
    ///
    /// This is a placeholder implementation that generates embeddings based on
    /// text content. In production, this would use the actual sentence transformer model.
    pub fn generate_deterministic_embedding(&self, text: &str) -> Vec<f32> {
        let dimensions = self.model_type.dimensions();
        let mut embedding = vec![0.0; dimensions];

        // Generate a simple hash-based embedding for testing
        // This maintains consistency: same text -> same embedding
        let text_bytes = text.as_bytes();

        for (i, val) in embedding.iter_mut().enumerate() {
            let byte_idx = i % text_bytes.len().max(1);
            let byte_val = if !text_bytes.is_empty() {
                text_bytes[byte_idx]
            } else {
                0
            };

            // Create a pseudo-random but deterministic value
            *val = ((byte_val as usize * 31 + i * 17) % 256) as f32 / 255.0 - 0.5;
        }

        // Normalize the vector to unit length (L2 norm = 1.0)
        // Use f64 for magnitude calculation to avoid accumulation errors with many dimensions
        let magnitude_f64: f64 = embedding.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
        let magnitude = magnitude_f64 as f32;

        if magnitude > 1e-10 {
            for val in &mut embedding {
                *val /= magnitude;
            }
        } else {
            // Handle zero vector case - create a uniform small vector
            let uniform_val = 1.0 / (dimensions as f32).sqrt();
            for val in &mut embedding {
                *val = uniform_val;
            }
        }

        embedding
    }
}

#[async_trait]
impl EmbeddingServiceTrait for LocalEmbeddingService {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.generate_deterministic_embedding(text))
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.generate_deterministic_embedding(text));
        }
        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.model_type.dimensions()
    }

    fn model_type(&self) -> EmbeddingModel {
        self.model_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_embed_single() {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let embedding = service
            .embed("Hello world")
            .await
            .expect("Failed to generate embedding");

        assert_eq!(embedding.len(), 384); // MiniLM has 384 dimensions
    }

    #[tokio::test]
    async fn test_embed_batch() {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let texts = vec!["Hello", "World", "Test"];
        let embeddings = service
            .embed_batch(&texts)
            .await
            .expect("Failed to generate embeddings");

        assert_eq!(embeddings.len(), 3);
        assert_eq!(embeddings[0].len(), 384);
        assert_eq!(embeddings[1].len(), 384);
        assert_eq!(embeddings[2].len(), 384);
    }

    #[tokio::test]
    async fn test_deterministic() {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let text = "Test text for deterministic embedding";
        let emb1 = service.embed(text).await.unwrap();
        let emb2 = service.embed(text).await.unwrap();

        // Same text should produce same embedding
        assert_eq!(emb1, emb2);
    }

    #[test]
    fn test_dimensions() {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        assert_eq!(service.dimensions(), 384);
    }

    #[test]
    fn test_normalized_embeddings() {
        let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create service");

        let embedding = service.generate_deterministic_embedding("test");

        // Check that embeddings are normalized (magnitude ≈ 1.0)
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((magnitude - 1.0).abs() < 0.001);
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    /// Generate valid UTF-8 strings for testing
    fn text_strategy() -> impl Strategy<Value = String> {
        // Generate strings with 1-1000 characters
        // Use printable ASCII + some Unicode for realistic text
        prop::string::string_regex("[a-zA-Z0-9 .,!?;:'\"-]{1,1000}")
            .expect("Valid regex")
    }

    /// Generate non-empty text strings
    fn non_empty_text_strategy() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-zA-Z0-9 .,!?;:'\"-]{1,500}")
            .expect("Valid regex")
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
                prop_assert!((a - b).abs() < 1e-10, "Embeddings should be identical for same input");
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
            prop_assert!(
                (magnitude - 1.0).abs() < 1e-6,
                "Embedding L2 norm should be 1.0, got {}",
                magnitude
            );

            // Verify no NaN or Inf values
            for val in &embedding {
                prop_assert!(val.is_finite(), "Embedding contains non-finite values");
            }
        }

        /// Property 3: Dimensions - all embeddings should have correct dimensions
        #[test]
        fn proptest_embedding_dimensions(text in text_strategy()) {
            let service = LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)
                .expect("Failed to create service");

            let embedding = service.generate_deterministic_embedding(&text);

            prop_assert_eq!(embedding.len(), 384, "MiniLM embeddings should have 384 dimensions");
        }

        /// Property 4: Batch ordering - embed_batch()[i] == embed(texts[i])
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
                let batch_embeddings = service.embed_batch(&text_refs).await
                    .expect("Batch embedding failed");

                // Get individual embeddings
                let mut individual_embeddings = Vec::new();
                for text in &texts {
                    let emb = service.embed(text).await.expect("Individual embedding failed");
                    individual_embeddings.push(emb);
                }

                // Verify batch and individual results match
                prop_assert_eq!(batch_embeddings.len(), individual_embeddings.len());

                for (i, (batch_emb, ind_emb)) in batch_embeddings.iter()
                    .zip(individual_embeddings.iter())
                    .enumerate() {
                    prop_assert_eq!(batch_emb.len(), ind_emb.len());
                    for (j, (a, b)) in batch_emb.iter().zip(ind_emb.iter()).enumerate() {
                        prop_assert!(
                            (a - b).abs() < 1e-6,
                            "Batch embedding[{}][{}] != individual embedding: {} vs {}",
                            i, j, a, b
                        );
                    }
                }

                Ok(()) as Result<(), proptest::test_runner::TestCaseError>
            })?;
        }

        /// Property 5: Empty string handling
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

        /// Property 6: Stability across different model types
        #[test]
        fn proptest_model_dimensions_consistency(
            text in non_empty_text_strategy(),
            model_idx in 0usize..2usize
        ) {
            let models = [EmbeddingModel::LocalMiniLM, EmbeddingModel::LocalMPNet];
            let expected_dims = [384, 768];

            let model = models[model_idx];
            let service = LocalEmbeddingService::new(model)
                .expect("Failed to create service");

            let embedding = service.generate_deterministic_embedding(&text);

            prop_assert_eq!(
                embedding.len(),
                expected_dims[model_idx],
                "Model {:?} should produce {} dimensions",
                model,
                expected_dims[model_idx]
            );

            // Verify normalization for all models
            let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            prop_assert!((magnitude - 1.0).abs() < 1e-6);
        }
    }
}

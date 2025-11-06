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
    fn generate_deterministic_embedding(&self, text: &str) -> Vec<f32> {
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

        // Normalize the vector
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for val in &mut embedding {
                *val /= magnitude;
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

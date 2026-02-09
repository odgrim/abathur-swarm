//! Embedding provider port for semantic vector generation.
//!
//! Defines the trait for embedding providers that convert text into
//! dense vector representations for semantic similarity search.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;

/// A single embedding request item.
#[derive(Debug, Clone)]
pub struct EmbeddingInput {
    /// Unique client-side ID for correlation.
    pub id: String,
    /// Text to embed.
    pub text: String,
}

/// A single embedding result.
#[derive(Debug, Clone)]
pub struct EmbeddingOutput {
    /// Correlation ID matching the input.
    pub id: String,
    /// The embedding vector.
    pub vector: Vec<f32>,
}

/// Trait for embedding providers (real-time and batch).
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Provider name (e.g., "openai", "voyage", "null").
    fn name(&self) -> &'static str;

    /// Embedding dimension for this provider/model.
    fn dimension(&self) -> usize;

    /// Generate an embedding for a single text.
    async fn embed(&self, text: &str) -> DomainResult<Vec<f32>>;

    /// Generate embeddings for multiple texts in a single API call.
    ///
    /// Implementations should handle chunking if the provider has per-request limits.
    async fn embed_batch(&self, inputs: &[EmbeddingInput]) -> DomainResult<Vec<EmbeddingOutput>>;

    /// Maximum number of texts per single API call.
    fn max_batch_size(&self) -> usize;
}

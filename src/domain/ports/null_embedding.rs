//! Null embedding provider implementation.
//!
//! Used when embedding features are not needed but the type system
//! requires an EmbeddingProvider implementation.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use super::embedding::{EmbeddingInput, EmbeddingOutput, EmbeddingProvider};

/// A no-op embedding provider that returns empty vectors.
#[derive(Debug, Clone, Default)]
pub struct NullEmbeddingProvider;

impl NullEmbeddingProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EmbeddingProvider for NullEmbeddingProvider {
    fn name(&self) -> &'static str {
        "null"
    }

    fn dimension(&self) -> usize {
        0
    }

    async fn embed(&self, _text: &str) -> DomainResult<Vec<f32>> {
        Ok(Vec::new())
    }

    async fn embed_batch(&self, inputs: &[EmbeddingInput]) -> DomainResult<Vec<EmbeddingOutput>> {
        Ok(inputs
            .iter()
            .map(|input| EmbeddingOutput {
                id: input.id.clone(),
                vector: Vec::new(),
            })
            .collect())
    }

    fn max_batch_size(&self) -> usize {
        0
    }
}

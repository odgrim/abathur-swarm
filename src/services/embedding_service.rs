//! Embedding service for batch and real-time vector generation.
//!
//! Orchestrates embedding generation across single and batch operations,
//! automatically chunking large batches and providing cost-efficient
//! embedding for bulk memory indexing (e.g., cold start).

use std::sync::Arc;

use crate::domain::errors::DomainResult;
use crate::domain::ports::embedding::{EmbeddingInput, EmbeddingOutput, EmbeddingProvider};

/// Configuration for the embedding service.
#[derive(Debug, Clone)]
pub struct EmbeddingServiceConfig {
    /// Minimum number of items to trigger batch mode instead of individual calls.
    /// Below this threshold, items are embedded individually with concurrency.
    pub batch_threshold: usize,
    /// Maximum concurrent individual embedding calls (when below batch_threshold).
    pub max_concurrency: usize,
}

impl Default for EmbeddingServiceConfig {
    fn default() -> Self {
        Self {
            batch_threshold: 5,
            max_concurrency: 10,
        }
    }
}

/// Report from a batch embedding operation.
#[derive(Debug, Clone, Default)]
pub struct BatchEmbeddingReport {
    /// Total items processed.
    pub total_items: usize,
    /// Items successfully embedded.
    pub succeeded: usize,
    /// Items that failed.
    pub failed: usize,
    /// Number of API calls made.
    pub api_calls: usize,
}

/// Embedding service that orchestrates embedding generation.
pub struct EmbeddingService {
    provider: Arc<dyn EmbeddingProvider>,
    config: EmbeddingServiceConfig,
}

impl EmbeddingService {
    pub fn new(provider: Arc<dyn EmbeddingProvider>, config: EmbeddingServiceConfig) -> Self {
        Self { provider, config }
    }

    pub fn with_defaults(provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self::new(provider, EmbeddingServiceConfig::default())
    }

    /// Provider name for diagnostics.
    pub fn provider_name(&self) -> &'static str {
        self.provider.name()
    }

    /// Embedding dimension.
    pub fn dimension(&self) -> usize {
        self.provider.dimension()
    }

    /// Embed a single text.
    pub async fn embed_single(&self, text: &str) -> DomainResult<Vec<f32>> {
        self.provider.embed(text).await
    }

    /// Embed multiple texts efficiently.
    ///
    /// For small batches (below `batch_threshold`), embeds individually.
    /// For larger batches, uses the provider's batch API with auto-chunking.
    pub async fn embed_many(
        &self,
        inputs: &[EmbeddingInput],
    ) -> DomainResult<Vec<EmbeddingOutput>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        if inputs.len() < self.config.batch_threshold {
            // Small batch: embed individually
            let mut outputs = Vec::with_capacity(inputs.len());
            for input in inputs {
                let vector = self.provider.embed(&input.text).await?;
                outputs.push(EmbeddingOutput {
                    id: input.id.clone(),
                    vector,
                });
            }
            return Ok(outputs);
        }

        // Large batch: use provider's batch API (handles chunking internally)
        let max_size = self.provider.max_batch_size();
        if max_size == 0 {
            // Provider doesn't support batch, fall back to individual
            let mut outputs = Vec::with_capacity(inputs.len());
            for input in inputs {
                let vector = self.provider.embed(&input.text).await?;
                outputs.push(EmbeddingOutput {
                    id: input.id.clone(),
                    vector,
                });
            }
            return Ok(outputs);
        }

        let mut all_outputs = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(max_size) {
            let chunk_outputs = self.provider.embed_batch(chunk).await?;
            all_outputs.extend(chunk_outputs);
        }

        Ok(all_outputs)
    }

    /// Embed multiple texts and return a report with statistics.
    pub async fn embed_many_with_report(
        &self,
        inputs: &[EmbeddingInput],
    ) -> (Vec<EmbeddingOutput>, BatchEmbeddingReport) {
        let total = inputs.len();
        let max_size = self.provider.max_batch_size().max(1);
        let api_calls = if inputs.len() < self.config.batch_threshold {
            inputs.len()
        } else {
            inputs.len().div_ceil(max_size)
        };

        match self.embed_many(inputs).await {
            Ok(outputs) => {
                let report = BatchEmbeddingReport {
                    total_items: total,
                    succeeded: outputs.len(),
                    failed: total.saturating_sub(outputs.len()),
                    api_calls,
                };
                (outputs, report)
            }
            Err(_) => {
                let report = BatchEmbeddingReport {
                    total_items: total,
                    succeeded: 0,
                    failed: total,
                    api_calls,
                };
                (Vec::new(), report)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::null_embedding::NullEmbeddingProvider;

    #[tokio::test]
    async fn test_embed_single_null_provider() {
        let provider = Arc::new(NullEmbeddingProvider::new());
        let service = EmbeddingService::with_defaults(provider);

        let result = service.embed_single("test text").await.unwrap();
        assert!(result.is_empty()); // Null provider returns empty vectors
    }

    #[tokio::test]
    async fn test_embed_many_empty() {
        let provider = Arc::new(NullEmbeddingProvider::new());
        let service = EmbeddingService::with_defaults(provider);

        let result = service.embed_many(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_embed_many_small_batch() {
        let provider = Arc::new(NullEmbeddingProvider::new());
        let service = EmbeddingService::with_defaults(provider);

        let inputs = vec![
            EmbeddingInput { id: "1".to_string(), text: "hello".to_string() },
            EmbeddingInput { id: "2".to_string(), text: "world".to_string() },
        ];

        let result = service.embed_many(&inputs).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "1");
        assert_eq!(result[1].id, "2");
    }

    #[tokio::test]
    async fn test_embed_many_with_report() {
        let provider = Arc::new(NullEmbeddingProvider::new());
        let service = EmbeddingService::with_defaults(provider);

        let inputs = vec![
            EmbeddingInput { id: "1".to_string(), text: "hello".to_string() },
            EmbeddingInput { id: "2".to_string(), text: "world".to_string() },
        ];

        let (outputs, report) = service.embed_many_with_report(&inputs).await;
        assert_eq!(outputs.len(), 2);
        assert_eq!(report.total_items, 2);
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.failed, 0);
    }

    #[tokio::test]
    async fn test_provider_name() {
        let provider = Arc::new(NullEmbeddingProvider::new());
        let service = EmbeddingService::with_defaults(provider);
        assert_eq!(service.provider_name(), "null");
    }

    #[tokio::test]
    async fn test_dimension() {
        let provider = Arc::new(NullEmbeddingProvider::new());
        let service = EmbeddingService::with_defaults(provider);
        assert_eq!(service.dimension(), 0);
    }

    // -- Mock provider for testing batch behavior --

    struct MockEmbeddingProvider {
        dimension: usize,
        max_batch: usize,
    }

    impl MockEmbeddingProvider {
        fn new(dimension: usize, max_batch: usize) -> Self {
            Self { dimension, max_batch }
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        fn name(&self) -> &'static str { "mock" }
        fn dimension(&self) -> usize { self.dimension }

        async fn embed(&self, _text: &str) -> DomainResult<Vec<f32>> {
            Ok(vec![0.1; self.dimension])
        }

        async fn embed_batch(&self, inputs: &[EmbeddingInput]) -> DomainResult<Vec<EmbeddingOutput>> {
            Ok(inputs.iter().map(|i| EmbeddingOutput {
                id: i.id.clone(),
                vector: vec![0.1; self.dimension],
            }).collect())
        }

        fn max_batch_size(&self) -> usize { self.max_batch }
    }

    #[tokio::test]
    async fn test_batch_threshold_triggers_batch_api() {
        let provider = Arc::new(MockEmbeddingProvider::new(4, 100));
        let service = EmbeddingService::new(provider, EmbeddingServiceConfig {
            batch_threshold: 3,
            max_concurrency: 5,
        });

        // 5 items >= threshold of 3, should use batch API
        let inputs: Vec<EmbeddingInput> = (0..5)
            .map(|i| EmbeddingInput { id: i.to_string(), text: format!("text {}", i) })
            .collect();

        let result = service.embed_many(&inputs).await.unwrap();
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].vector.len(), 4);
    }

    #[tokio::test]
    async fn test_auto_chunking_large_batch() {
        let provider = Arc::new(MockEmbeddingProvider::new(4, 3));
        let service = EmbeddingService::new(provider, EmbeddingServiceConfig {
            batch_threshold: 2,
            max_concurrency: 5,
        });

        // 7 items with max_batch_size=3: should chunk into 3+3+1
        let inputs: Vec<EmbeddingInput> = (0..7)
            .map(|i| EmbeddingInput { id: i.to_string(), text: format!("text {}", i) })
            .collect();

        let result = service.embed_many(&inputs).await.unwrap();
        assert_eq!(result.len(), 7);
    }

    #[tokio::test]
    async fn test_embed_many_with_report_counts_api_calls() {
        let provider = Arc::new(MockEmbeddingProvider::new(4, 3));
        let service = EmbeddingService::new(provider, EmbeddingServiceConfig {
            batch_threshold: 2,
            max_concurrency: 5,
        });

        let inputs: Vec<EmbeddingInput> = (0..7)
            .map(|i| EmbeddingInput { id: i.to_string(), text: format!("text {}", i) })
            .collect();

        let (outputs, report) = service.embed_many_with_report(&inputs).await;
        assert_eq!(outputs.len(), 7);
        assert_eq!(report.total_items, 7);
        assert_eq!(report.succeeded, 7);
        assert_eq!(report.api_calls, 3); // ceil(7/3) = 3 batch API calls
    }
}

//! OpenAI embedding provider adapter.
//!
//! Supports both real-time and batch embedding generation via the
//! OpenAI `/v1/embeddings` endpoint. Compatible with any OpenAI-compatible
//! embedding API (e.g., Azure OpenAI, local servers).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ports::embedding::{EmbeddingInput, EmbeddingOutput, EmbeddingProvider};

/// Configuration for the OpenAI embedding provider.
#[derive(Debug, Clone)]
pub struct OpenAiEmbeddingConfig {
    /// API key. Falls back to `OPENAI_API_KEY` env var.
    pub api_key: Option<String>,
    /// Base URL for the API. Default: `https://api.openai.com/v1`.
    pub base_url: String,
    /// Embedding model. Default: `text-embedding-3-small`.
    pub model: String,
    /// Expected embedding dimension. Default: 1536.
    pub dimension: usize,
    /// Request timeout in seconds. Default: 30.
    pub timeout_secs: u64,
    /// Maximum texts per single API request. Default: 2048.
    pub max_batch_size: usize,
}

impl Default for OpenAiEmbeddingConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
            timeout_secs: 30,
            max_batch_size: 2048,
        }
    }
}

impl OpenAiEmbeddingConfig {
    fn get_api_key(&self) -> DomainResult<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                DomainError::ExecutionFailed(
                    "OpenAI API key not set. Set OPENAI_API_KEY env var or configure api_key."
                        .to_string(),
                )
            })
    }
}

/// OpenAI embedding provider.
pub struct OpenAiEmbeddingProvider {
    config: OpenAiEmbeddingConfig,
    client: Arc<reqwest::Client>,
}

impl OpenAiEmbeddingProvider {
    pub fn new(config: OpenAiEmbeddingConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");
        Self {
            config,
            client: Arc::new(client),
        }
    }

    async fn call_embeddings_api(&self, texts: Vec<String>) -> DomainResult<Vec<Vec<f32>>> {
        let api_key = self.config.get_api_key()?;
        let url = format!("{}/embeddings", self.config.base_url);

        let request_body = EmbeddingsRequest {
            model: self.config.model.clone(),
            input: texts,
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| DomainError::ExecutionFailed(format!("Embedding API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unable to read response body".to_string());
            return Err(DomainError::ExecutionFailed(format!(
                "Embedding API returned {}: {}",
                status, body
            )));
        }

        let result: EmbeddingsResponse = response
            .json()
            .await
            .map_err(|e| DomainError::SerializationError(format!("Failed to parse embedding response: {}", e)))?;

        // Sort by index to maintain input order
        let mut data = result.data;
        data.sort_by_key(|d| d.index);

        Ok(data.into_iter().map(|d| d.embedding).collect())
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn dimension(&self) -> usize {
        self.config.dimension
    }

    async fn embed(&self, text: &str) -> DomainResult<Vec<f32>> {
        let results = self.call_embeddings_api(vec![text.to_string()]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| DomainError::ExecutionFailed("Empty embedding response".to_string()))
    }

    async fn embed_batch(&self, inputs: &[EmbeddingInput]) -> DomainResult<Vec<EmbeddingOutput>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let texts: Vec<String> = inputs.iter().map(|i| i.text.clone()).collect();
        let mut all_outputs = Vec::with_capacity(inputs.len());

        // Chunk by max_batch_size
        for chunk_start in (0..texts.len()).step_by(self.config.max_batch_size) {
            let chunk_end = (chunk_start + self.config.max_batch_size).min(texts.len());
            let chunk_texts = texts[chunk_start..chunk_end].to_vec();
            let chunk_inputs = &inputs[chunk_start..chunk_end];

            let vectors = self.call_embeddings_api(chunk_texts).await?;

            for (input, vector) in chunk_inputs.iter().zip(vectors) {
                all_outputs.push(EmbeddingOutput {
                    id: input.id.clone(),
                    vector,
                });
            }
        }

        Ok(all_outputs)
    }

    fn max_batch_size(&self) -> usize {
        self.config.max_batch_size
    }
}

// -- OpenAI API request/response types --

#[derive(Debug, Serialize)]
struct EmbeddingsRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = OpenAiEmbeddingConfig::default();
        assert_eq!(config.model, "text-embedding-3-small");
        assert_eq!(config.dimension, 1536);
        assert_eq!(config.max_batch_size, 2048);
        assert_eq!(config.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn test_api_key_from_config() {
        let config = OpenAiEmbeddingConfig {
            api_key: Some("test-key".to_string()),
            ..Default::default()
        };
        assert_eq!(config.get_api_key().unwrap(), "test-key");
    }

    #[test]
    fn test_api_key_missing() {
        // Clear env var for this test
        let config = OpenAiEmbeddingConfig {
            api_key: None,
            ..Default::default()
        };
        // This will succeed if OPENAI_API_KEY is set in env, fail otherwise
        // We just verify it doesn't panic
        let _ = config.get_api_key();
    }
}

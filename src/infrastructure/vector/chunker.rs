//! Text chunking service implementation
//!
//! Provides token-aware text chunking using tiktoken for optimal embedding quality.

use crate::domain::models::{Chunk, ChunkMetadata, ChunkingConfig};
use crate::domain::ports::ChunkingService as ChunkingServiceTrait;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tiktoken_rs::CoreBPE;

/// Token-aware text chunking service
///
/// Splits text into chunks while respecting token boundaries and optionally
/// sentence boundaries for better semantic coherence.
pub struct Chunker {
    config: ChunkingConfig,
    tokenizer: CoreBPE,
}

impl Chunker {
    /// Create a new chunker with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(ChunkingConfig::default())
    }

    /// Create a new chunker with custom configuration
    pub fn with_config(config: ChunkingConfig) -> Result<Self> {
        // Validate configuration
        config
            .validate()
            .map_err(|e| anyhow!("Invalid chunking config: {}", e))?;

        // Load cl100k_base tokenizer (used by GPT-4 and most embedding models)
        let tokenizer = tiktoken_rs::cl100k_base()
            .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

        Ok(Self { config, tokenizer })
    }

    /// Chunk text into pieces suitable for embedding
    fn chunk_impl(&self, text: &str, parent_id: &str) -> Result<Vec<Chunk>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        // Tokenize the entire text
        let tokens = self.tokenizer.encode_with_special_tokens(text);

        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = Vec::new();
        let mut start = 0;
        let mut chunk_index = 0;
        let mut char_offset = 0;

        while start < tokens.len() {
            let end = (start + self.config.chunk_size).min(tokens.len());
            let chunk_tokens = &tokens[start..end];

            // Decode tokens back to text
            let mut chunk_text = self
                .tokenizer
                .decode(chunk_tokens.to_vec())
                .map_err(|e| anyhow!("Failed to decode tokens: {}", e))?;

            let mut truncated = false;

            // Respect sentence boundaries if configured and not at end
            if self.config.respect_boundaries && end < tokens.len() {
                if let Some(boundary_text) = self.snap_to_boundary(&chunk_text) {
                    chunk_text = boundary_text;
                    truncated = true;
                }
            }

            let chunk_len = chunk_text.len();

            // Create metadata
            let metadata = ChunkMetadata::with_offsets(char_offset, char_offset + chunk_len);
            let metadata = if truncated {
                metadata.mark_truncated()
            } else {
                metadata
            };

            // Create chunk
            let chunk = Chunk::new(
                parent_id.to_string(),
                chunk_text,
                chunk_index,
                chunk_tokens.len(),
            )
            .with_metadata(metadata);

            chunks.push(chunk);

            // Move to next chunk with overlap
            if end >= tokens.len() {
                break;
            }

            start = if self.config.chunk_overlap > 0 {
                end.saturating_sub(self.config.chunk_overlap)
            } else {
                end
            };

            chunk_index += 1;
            char_offset += chunk_len;
        }

        Ok(chunks)
    }

    /// Snap chunk text to nearest sentence boundary
    fn snap_to_boundary(&self, text: &str) -> Option<String> {
        // Find last sentence boundary (., !, ?, \n)
        let boundaries = ['.', '!', '?', '\n'];

        let mut last_boundary = None;

        for (i, c) in text.char_indices().rev() {
            if boundaries.contains(&c) {
                last_boundary = Some(i + c.len_utf8());
                break;
            }
        }

        last_boundary.map(|pos| text[..pos].to_string())
    }

    /// Count tokens in text
    fn count_tokens_impl(&self, text: &str) -> Result<usize> {
        let tokens = self.tokenizer.encode_with_special_tokens(text);
        Ok(tokens.len())
    }
}

impl Default for Chunker {
    fn default() -> Self {
        Self::new().expect("Failed to create default chunker")
    }
}

#[async_trait]
impl ChunkingServiceTrait for Chunker {
    async fn chunk(&self, text: &str, parent_id: &str) -> Result<Vec<Chunk>> {
        self.chunk_impl(text, parent_id)
    }

    async fn count_tokens(&self, text: &str) -> Result<usize> {
        self.count_tokens_impl(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_chunker() {
        let chunker = Chunker::new();
        assert!(chunker.is_ok());
    }

    #[test]
    fn test_with_config() {
        let config = ChunkingConfig {
            chunk_size: 256,
            chunk_overlap: 32,
            separator: "\n\n".to_string(),
            respect_boundaries: true,
        };

        let chunker = Chunker::with_config(config);
        assert!(chunker.is_ok());
    }

    #[test]
    fn test_invalid_config() {
        let config = ChunkingConfig {
            chunk_size: 100,
            chunk_overlap: 150, // Invalid: overlap > chunk_size
            separator: "\n\n".to_string(),
            respect_boundaries: true,
        };

        let chunker = Chunker::with_config(config);
        assert!(chunker.is_err());
    }

    #[tokio::test]
    async fn test_chunk_empty_text() {
        let chunker = Chunker::new().unwrap();
        let chunks = chunker.chunk("", "test-doc").await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_chunk_short_text() {
        let chunker = Chunker::new().unwrap();
        let text = "This is a short text.";
        let chunks = chunker.chunk(text, "test-doc").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].parent_id, "test-doc");
        assert_eq!(chunks[0].chunk_index, 0);
        assert!(chunks[0].is_first());
    }

    #[tokio::test]
    async fn test_chunk_long_text() {
        let chunker = Chunker::with_config(ChunkingConfig {
            chunk_size: 50,  // Small size for testing
            chunk_overlap: 5, // Small overlap
            separator: "\n\n".to_string(),
            respect_boundaries: true,
        })
        .unwrap();

        // Create text longer than chunk_size
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(20);
        let chunks = chunker.chunk(&text, "test-doc").await.unwrap();

        // Should have multiple chunks
        assert!(chunks.len() > 1);

        // Check chunk IDs
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
            assert_eq!(chunk.id, format!("test-doc:chunk:{}", i));
            assert_eq!(chunk.parent_id, "test-doc");
        }
    }

    #[tokio::test]
    async fn test_count_tokens() {
        let chunker = Chunker::new().unwrap();

        let text = "Hello world";
        let count = chunker.count_tokens(text).await.unwrap();

        // "Hello world" should be 2 tokens
        assert!(count >= 2);
    }

    #[test]
    fn test_snap_to_boundary() {
        let chunker = Chunker::new().unwrap();

        let text = "This is a sentence. This is another one";
        let snapped = chunker.snap_to_boundary(text);

        assert!(snapped.is_some());
        assert_eq!(snapped.unwrap(), "This is a sentence.");
    }

    #[test]
    fn test_snap_to_boundary_no_boundary() {
        let chunker = Chunker::new().unwrap();

        let text = "This text has no sentence ending";
        let snapped = chunker.snap_to_boundary(text);

        assert!(snapped.is_none());
    }

    #[tokio::test]
    async fn test_chunk_with_overlap() {
        let chunker = Chunker::with_config(ChunkingConfig {
            chunk_size: 20,
            chunk_overlap: 5,
            separator: "\n\n".to_string(),
            respect_boundaries: false,
        })
        .unwrap();

        let text = "word ".repeat(30); // 30 words
        let chunks = chunker.chunk(&text, "test-doc").await.unwrap();

        // Should have multiple chunks due to overlap
        assert!(chunks.len() > 1);

        // Each chunk should have reasonable token count
        for chunk in &chunks {
            assert!(chunk.token_count > 0);
        }
    }

    #[tokio::test]
    async fn test_chunk_metadata() {
        let chunker = Chunker::new().unwrap();
        let text = "This is a test.";
        let chunks = chunker.chunk(text, "test-doc").await.unwrap();

        assert_eq!(chunks.len(), 1);

        let metadata = &chunks[0].metadata;
        assert_eq!(metadata.start_offset, Some(0));
        assert!(metadata.end_offset.is_some());
        assert!(!metadata.truncated_at_boundary);
    }
}

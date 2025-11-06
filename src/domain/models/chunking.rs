//! Text chunking domain models
//!
//! Models for splitting documents into chunks for embedding.
//! Token-aware chunking ensures optimal embedding quality.

use serde::{Deserialize, Serialize};

/// Configuration for document chunking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// Maximum size of each chunk in tokens
    pub chunk_size: usize,

    /// Overlap between chunks in tokens (for context preservation)
    pub chunk_overlap: usize,

    /// Separator to use for splitting (e.g., "\n\n" for paragraphs)
    pub separator: String,

    /// Whether to respect sentence boundaries when chunking
    /// (prevents splitting mid-sentence for better semantic coherence)
    pub respect_boundaries: bool,
}

impl ChunkingConfig {
    /// Create default chunking configuration
    /// - 512 tokens per chunk (optimal for most embedding models)
    /// - 50 tokens overlap (preserves context across chunks)
    /// - Paragraph separator
    /// - Respect sentence boundaries
    pub fn default() -> Self {
        Self {
            chunk_size: 512,
            chunk_overlap: 50,
            separator: "\n\n".to_string(),
            respect_boundaries: true,
        }
    }

    /// Create configuration for small chunks (better for precise retrieval)
    pub fn small() -> Self {
        Self {
            chunk_size: 256,
            chunk_overlap: 32,
            separator: "\n\n".to_string(),
            respect_boundaries: true,
        }
    }

    /// Create configuration for large chunks (better for broad context)
    pub fn large() -> Self {
        Self {
            chunk_size: 1024,
            chunk_overlap: 100,
            separator: "\n\n".to_string(),
            respect_boundaries: true,
        }
    }

    /// Validate the chunking configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.chunk_size == 0 {
            return Err("chunk_size must be greater than 0".to_string());
        }

        if self.chunk_overlap >= self.chunk_size {
            return Err("chunk_overlap must be less than chunk_size".to_string());
        }

        if self.separator.is_empty() {
            return Err("separator cannot be empty".to_string());
        }

        Ok(())
    }
}

/// A chunk of text extracted from a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier for this chunk
    pub id: String,

    /// ID of the parent document
    pub parent_id: String,

    /// The text content of this chunk
    pub content: String,

    /// Index of this chunk within the parent document (0-based)
    pub chunk_index: usize,

    /// Number of tokens in this chunk
    pub token_count: usize,

    /// Metadata about this chunk
    pub metadata: ChunkMetadata,
}

impl Chunk {
    /// Create a new chunk
    pub fn new(
        parent_id: String,
        content: String,
        chunk_index: usize,
        token_count: usize,
    ) -> Self {
        let id = format!("{}:chunk:{}", parent_id, chunk_index);

        Self {
            id,
            parent_id,
            content,
            chunk_index,
            token_count,
            metadata: ChunkMetadata::default(),
        }
    }

    /// Set metadata for this chunk
    pub fn with_metadata(mut self, metadata: ChunkMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Returns true if this is the first chunk
    pub fn is_first(&self) -> bool {
        self.chunk_index == 0
    }

    /// Get a preview of the content (first 100 chars)
    pub fn preview(&self) -> String {
        if self.content.len() <= 100 {
            self.content.clone()
        } else {
            format!("{}...", &self.content[..100])
        }
    }
}

/// Metadata about a chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    /// Start position in the original document (character offset)
    pub start_offset: Option<usize>,

    /// End position in the original document (character offset)
    pub end_offset: Option<usize>,

    /// Whether this chunk was truncated at a boundary
    pub truncated_at_boundary: bool,

    /// Language of the content (if detected)
    pub language: Option<String>,

    /// Additional custom metadata
    pub custom: serde_json::Value,
}

impl Default for ChunkMetadata {
    fn default() -> Self {
        Self {
            start_offset: None,
            end_offset: None,
            truncated_at_boundary: false,
            language: None,
            custom: serde_json::json!({}),
        }
    }
}

impl ChunkMetadata {
    /// Create metadata with offsets
    pub fn with_offsets(start: usize, end: usize) -> Self {
        Self {
            start_offset: Some(start),
            end_offset: Some(end),
            truncated_at_boundary: false,
            language: None,
            custom: serde_json::json!({}),
        }
    }

    /// Mark as truncated at a boundary
    pub fn mark_truncated(mut self) -> Self {
        self.truncated_at_boundary = true;
        self
    }

    /// Set the language
    pub fn with_language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    /// Add custom metadata
    pub fn with_custom(mut self, key: String, value: serde_json::Value) -> Self {
        if let Some(obj) = self.custom.as_object_mut() {
            obj.insert(key, value);
        }
        self
    }
}

/// Strategy for handling overlapping chunks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapStrategy {
    /// Fixed overlap size (specified in config)
    Fixed,

    /// Adaptive overlap based on content (finds natural break points)
    Adaptive,

    /// No overlap between chunks
    None,
}

impl Default for OverlapStrategy {
    fn default() -> Self {
        Self::Fixed
    }
}

/// Result of a chunking operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingResult {
    /// The chunks created
    pub chunks: Vec<Chunk>,

    /// Total number of chunks created
    pub total_chunks: usize,

    /// Total tokens across all chunks (with overlap)
    pub total_tokens: usize,

    /// Original document size in characters
    pub original_size: usize,

    /// Configuration used for chunking
    pub config: ChunkingConfig,
}

impl ChunkingResult {
    /// Create a new chunking result
    pub fn new(chunks: Vec<Chunk>, original_size: usize, config: ChunkingConfig) -> Self {
        let total_chunks = chunks.len();
        let total_tokens = chunks.iter().map(|c| c.token_count).sum();

        Self {
            chunks,
            total_chunks,
            total_tokens,
            original_size,
            config,
        }
    }

    /// Calculate the average chunk size in tokens
    pub fn average_chunk_size(&self) -> f64 {
        if self.total_chunks == 0 {
            0.0
        } else {
            self.total_tokens as f64 / self.total_chunks as f64
        }
    }

    /// Calculate the compression ratio (total_tokens / original_size)
    /// This shows how much expansion happened due to overlap
    pub fn expansion_ratio(&self) -> f64 {
        if self.original_size == 0 {
            0.0
        } else {
            self.total_tokens as f64 / self.original_size as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunking_config_default() {
        let config = ChunkingConfig::default();
        assert_eq!(config.chunk_size, 512);
        assert_eq!(config.chunk_overlap, 50);
        assert_eq!(config.separator, "\n\n");
        assert!(config.respect_boundaries);
    }

    #[test]
    fn test_chunking_config_validation() {
        let valid = ChunkingConfig::default();
        assert!(valid.validate().is_ok());

        let invalid_size = ChunkingConfig {
            chunk_size: 0,
            ..ChunkingConfig::default()
        };
        assert!(invalid_size.validate().is_err());

        let invalid_overlap = ChunkingConfig {
            chunk_overlap: 600,
            ..ChunkingConfig::default()
        };
        assert!(invalid_overlap.validate().is_err());
    }

    #[test]
    fn test_chunk_new() {
        let chunk = Chunk::new(
            "doc-123".to_string(),
            "test content".to_string(),
            0,
            10,
        );

        assert_eq!(chunk.id, "doc-123:chunk:0");
        assert_eq!(chunk.parent_id, "doc-123");
        assert_eq!(chunk.content, "test content");
        assert_eq!(chunk.chunk_index, 0);
        assert_eq!(chunk.token_count, 10);
        assert!(chunk.is_first());
    }

    #[test]
    fn test_chunk_preview() {
        let short = Chunk::new(
            "doc".to_string(),
            "short".to_string(),
            0,
            1,
        );
        assert_eq!(short.preview(), "short");

        let long = Chunk::new(
            "doc".to_string(),
            "a".repeat(200),
            0,
            200,
        );
        assert_eq!(long.preview().len(), 103); // 100 chars + "..."
    }

    #[test]
    fn test_chunk_metadata() {
        let metadata = ChunkMetadata::with_offsets(0, 100)
            .mark_truncated()
            .with_language("rust".to_string())
            .with_custom("author".to_string(), serde_json::json!("test"));

        assert_eq!(metadata.start_offset, Some(0));
        assert_eq!(metadata.end_offset, Some(100));
        assert!(metadata.truncated_at_boundary);
        assert_eq!(metadata.language, Some("rust".to_string()));
    }

    #[test]
    fn test_chunking_result() {
        let config = ChunkingConfig::default();
        let chunks = vec![
            Chunk::new("doc".to_string(), "chunk1".to_string(), 0, 100),
            Chunk::new("doc".to_string(), "chunk2".to_string(), 1, 100),
        ];

        let result = ChunkingResult::new(chunks, 1000, config);

        assert_eq!(result.total_chunks, 2);
        assert_eq!(result.total_tokens, 200);
        assert_eq!(result.average_chunk_size(), 100.0);
        assert_eq!(result.expansion_ratio(), 0.2);
    }
}

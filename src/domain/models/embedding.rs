//! Embedding domain models
//!
//! Models for vector embeddings and RAG (Retrieval-Augmented Generation).
//! These models define the core embedding functionality in a framework-agnostic way.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Embedding model types supported by the system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingModel {
    /// Local MiniLM model (all-MiniLM-L6-v2)
    /// - Dimensions: 384
    /// - Size: ~80MB
    /// - Speed: ~50ms per embedding (CPU)
    /// - Quality: Good for most use cases
    LocalMiniLM,

    /// Local MPNet model (all-mpnet-base-v2)
    /// - Dimensions: 768
    /// - Size: ~420MB
    /// - Speed: ~100ms per embedding (CPU)
    /// - Quality: Better than MiniLM
    LocalMPNet,

    /// OpenAI Ada-002 model (cloud-based, opt-in only)
    /// - Dimensions: 1536
    /// - Size: N/A (API-based)
    /// - Speed: ~200ms per embedding (includes network)
    /// - Quality: Best (but not private)
    OpenAIAda002,
}

impl EmbeddingModel {
    /// Returns the vector dimensions for this model
    pub fn dimensions(&self) -> usize {
        match self {
            Self::LocalMiniLM => 384,
            Self::LocalMPNet => 768,
            Self::OpenAIAda002 => 1536,
        }
    }

    /// Returns the HuggingFace model name or identifier
    pub fn model_name(&self) -> &str {
        match self {
            Self::LocalMiniLM => "sentence-transformers/all-MiniLM-L6-v2",
            Self::LocalMPNet => "sentence-transformers/all-mpnet-base-v2",
            Self::OpenAIAda002 => "text-embedding-ada-002",
        }
    }

    /// Returns true if this is a local model (privacy-preserving)
    pub fn is_local(&self) -> bool {
        matches!(self, Self::LocalMiniLM | Self::LocalMPNet)
    }

    /// Returns true if this model requires API credentials
    pub fn requires_api_key(&self) -> bool {
        matches!(self, Self::OpenAIAda002)
    }

    /// Returns the default model (LocalMiniLM for best balance)
    pub fn default() -> Self {
        Self::LocalMiniLM
    }
}

impl std::fmt::Display for EmbeddingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalMiniLM => write!(f, "all-MiniLM-L6-v2 (local)"),
            Self::LocalMPNet => write!(f, "all-mpnet-base-v2 (local)"),
            Self::OpenAIAda002 => write!(f, "text-embedding-ada-002 (OpenAI)"),
        }
    }
}

/// Citation information for vector memory sources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Citation {
    /// Source identifier (file path, URL, or document ID)
    pub source: String,

    /// Page number (if applicable)
    pub page: Option<u32>,

    /// URL (if from web)
    pub url: Option<String>,

    /// Timestamp of when this was retrieved/created
    pub timestamp: DateTime<Utc>,
}

impl Citation {
    /// Create a new citation from a file path
    pub fn from_file(path: String) -> Self {
        Self {
            source: path,
            page: None,
            url: None,
            timestamp: Utc::now(),
        }
    }

    /// Create a new citation from a URL
    pub fn from_url(url: String) -> Self {
        Self {
            source: url.clone(),
            page: None,
            url: Some(url),
            timestamp: Utc::now(),
        }
    }

    /// Create a new citation from a document with page number
    pub fn from_document(doc_id: String, page: u32) -> Self {
        Self {
            source: doc_id,
            page: Some(page),
            url: None,
            timestamp: Utc::now(),
        }
    }
}

/// Vector memory entry for semantic search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMemory {
    /// Unique identifier
    pub id: String,

    /// Namespace for organizing memories (e.g., "agent:memory", "docs:api")
    pub namespace: String,

    /// The actual text content
    pub content: String,

    /// Vector embedding (dimensions depend on model)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embedding: Vec<f32>,

    /// Additional metadata as JSON
    pub metadata: serde_json::Value,

    /// Source citation (optional)
    pub source_citation: Option<Citation>,

    /// When this memory was created
    pub created_at: DateTime<Utc>,

    /// When this memory was last updated
    pub updated_at: DateTime<Utc>,

    /// Who created this memory
    pub created_by: String,
}

impl VectorMemory {
    /// Create a new vector memory (embedding will be generated later)
    pub fn new(
        id: String,
        namespace: String,
        content: String,
        metadata: serde_json::Value,
        citation: Option<Citation>,
        created_by: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            namespace,
            content,
            embedding: Vec::new(),
            metadata,
            source_citation: citation,
            created_at: now,
            updated_at: now,
            created_by,
        }
    }

    /// Set the embedding vector
    pub fn set_embedding(&mut self, embedding: Vec<f32>) {
        self.embedding = embedding;
        self.updated_at = Utc::now();
    }

    /// Get the embedding dimensions (0 if not yet embedded)
    pub fn embedding_dimensions(&self) -> usize {
        self.embedding.len()
    }

    /// Returns true if this memory has been embedded
    pub fn is_embedded(&self) -> bool {
        !self.embedding.is_empty()
    }
}

/// Search result from vector similarity search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Memory ID
    pub id: String,

    /// Namespace
    pub namespace: String,

    /// Content
    pub content: String,

    /// Distance metric (lower is better, 0 = identical)
    pub distance: f32,

    /// Normalized similarity score (0-1, higher is better)
    pub score: f32,

    /// Metadata
    pub metadata: serde_json::Value,

    /// Citation (if available)
    pub citation: Option<Citation>,
}

impl SearchResult {
    /// Create a new search result
    pub fn new(
        id: String,
        namespace: String,
        content: String,
        distance: f32,
        metadata: serde_json::Value,
        citation: Option<Citation>,
    ) -> Self {
        // Convert distance to similarity score: score = 1 / (1 + distance)
        let score = 1.0 / (1.0 + distance);

        Self {
            id,
            namespace,
            content,
            distance,
            score,
            metadata,
            citation,
        }
    }

    /// Returns true if this is a high-quality match (score > 0.7)
    pub fn is_high_quality(&self) -> bool {
        self.score > 0.7
    }

    /// Returns true if this is a moderate match (score > 0.5)
    pub fn is_moderate_quality(&self) -> bool {
        self.score > 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_model_dimensions() {
        assert_eq!(EmbeddingModel::LocalMiniLM.dimensions(), 384);
        assert_eq!(EmbeddingModel::LocalMPNet.dimensions(), 768);
        assert_eq!(EmbeddingModel::OpenAIAda002.dimensions(), 1536);
    }

    #[test]
    fn test_embedding_model_is_local() {
        assert!(EmbeddingModel::LocalMiniLM.is_local());
        assert!(EmbeddingModel::LocalMPNet.is_local());
        assert!(!EmbeddingModel::OpenAIAda002.is_local());
    }

    #[test]
    fn test_embedding_model_requires_api_key() {
        assert!(!EmbeddingModel::LocalMiniLM.requires_api_key());
        assert!(!EmbeddingModel::LocalMPNet.requires_api_key());
        assert!(EmbeddingModel::OpenAIAda002.requires_api_key());
    }

    #[test]
    fn test_citation_from_file() {
        let citation = Citation::from_file("test.txt".to_string());
        assert_eq!(citation.source, "test.txt");
        assert_eq!(citation.page, None);
        assert_eq!(citation.url, None);
    }

    #[test]
    fn test_citation_from_url() {
        let url = "https://example.com/doc".to_string();
        let citation = Citation::from_url(url.clone());
        assert_eq!(citation.source, url);
        assert_eq!(citation.url, Some(url));
        assert_eq!(citation.page, None);
    }

    #[test]
    fn test_vector_memory_new() {
        let memory = VectorMemory::new(
            "test-id".to_string(),
            "test:namespace".to_string(),
            "test content".to_string(),
            serde_json::json!({"key": "value"}),
            None,
            "test-agent".to_string(),
        );

        assert_eq!(memory.id, "test-id");
        assert_eq!(memory.namespace, "test:namespace");
        assert_eq!(memory.content, "test content");
        assert!(!memory.is_embedded());
        assert_eq!(memory.embedding_dimensions(), 0);
    }

    #[test]
    fn test_vector_memory_set_embedding() {
        let mut memory = VectorMemory::new(
            "test-id".to_string(),
            "test:namespace".to_string(),
            "test content".to_string(),
            serde_json::json!({}),
            None,
            "test-agent".to_string(),
        );

        let embedding = vec![0.1, 0.2, 0.3];
        memory.set_embedding(embedding.clone());

        assert!(memory.is_embedded());
        assert_eq!(memory.embedding_dimensions(), 3);
        assert_eq!(memory.embedding, embedding);
    }

    #[test]
    fn test_search_result_score_calculation() {
        let result = SearchResult::new(
            "id".to_string(),
            "ns".to_string(),
            "content".to_string(),
            0.0, // distance = 0 means identical
            serde_json::json!({}),
            None,
        );

        // score = 1 / (1 + 0) = 1.0
        assert_eq!(result.score, 1.0);
        assert!(result.is_high_quality());
    }

    #[test]
    fn test_search_result_quality_thresholds() {
        let high = SearchResult::new(
            "id".to_string(),
            "ns".to_string(),
            "content".to_string(),
            0.3, // score ~= 0.77
            serde_json::json!({}),
            None,
        );
        assert!(high.is_high_quality());

        let moderate = SearchResult::new(
            "id".to_string(),
            "ns".to_string(),
            "content".to_string(),
            0.8, // score ~= 0.56
            serde_json::json!({}),
            None,
        );
        assert!(moderate.is_moderate_quality());
        assert!(!moderate.is_high_quality());
    }
}

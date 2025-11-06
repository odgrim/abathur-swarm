use crate::domain::models::{
    Citation, Chunk, EmbeddingModel, SearchResult, VectorMemory,
};
use anyhow::Result;
use async_trait::async_trait;

/// Repository interface for embedding and vector storage operations
///
/// Provides operations for storing and retrieving vector embeddings for semantic search.
/// Implementations should handle embedding generation, vector storage, and similarity search.
#[async_trait]
pub trait EmbeddingRepository: Send + Sync {
    /// Insert a vector memory entry with automatic embedding generation
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this memory
    /// * `namespace` - Hierarchical namespace for organization (e.g., "agent:memory", "docs:api")
    /// * `content` - The text content to embed
    /// * `metadata` - Optional JSON metadata
    /// * `citation` - Optional citation information
    ///
    /// # Returns
    /// * `Ok(i64)` - The database rowid of the inserted entry
    /// * `Err(_)` - If embedding generation or insertion fails
    async fn insert(
        &self,
        id: &str,
        namespace: &str,
        content: &str,
        metadata: Option<serde_json::Value>,
        citation: Option<Citation>,
    ) -> Result<i64>;

    /// Insert multiple vector memories in a batch (more efficient)
    ///
    /// # Arguments
    /// * `memories` - Vector of (id, namespace, content) tuples
    ///
    /// # Returns
    /// * `Ok(Vec<i64>)` - The database rowids of all inserted entries
    /// * `Err(_)` - If any insertion fails
    async fn insert_batch(
        &self,
        memories: Vec<(String, String, String)>,
    ) -> Result<Vec<i64>>;

    /// Semantic search for similar content
    ///
    /// # Arguments
    /// * `query` - The search query text
    /// * `limit` - Maximum number of results to return
    /// * `namespace_filter` - Optional namespace to filter results (e.g., "docs:*")
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Ordered by similarity (most similar first)
    /// * `Err(_)` - If embedding generation or search fails
    async fn search_similar(
        &self,
        query: &str,
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>>;

    /// Hybrid search: vector similarity + keyword matching
    ///
    /// # Arguments
    /// * `query` - The search query text
    /// * `limit` - Maximum number of results to return
    /// * `alpha` - Weight for vector vs keyword (0=keyword only, 1=vector only)
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Results using reciprocal rank fusion
    /// * `Err(_)` - If search fails
    async fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        alpha: f32,
    ) -> Result<Vec<SearchResult>>;

    /// Get a vector memory by ID
    ///
    /// # Arguments
    /// * `id` - The unique identifier
    ///
    /// # Returns
    /// * `Ok(Some(VectorMemory))` - The memory if found
    /// * `Ok(None)` - If not found
    /// * `Err(_)` - If query fails
    async fn get(&self, id: &str) -> Result<Option<VectorMemory>>;

    /// Delete a vector memory
    ///
    /// # Arguments
    /// * `id` - The unique identifier
    ///
    /// # Returns
    /// * `Ok(())` - If successfully deleted
    /// * `Err(_)` - If deletion fails
    async fn delete(&self, id: &str) -> Result<()>;

    /// Count vector memories in a namespace
    ///
    /// # Arguments
    /// * `namespace_prefix` - The namespace prefix to match
    ///
    /// # Returns
    /// * `Ok(usize)` - Count of matching memories
    /// * `Err(_)` - If query fails
    async fn count(&self, namespace_prefix: &str) -> Result<usize>;

    /// List all namespaces with their memory counts
    ///
    /// # Returns
    /// * `Ok(Vec<(String, usize)>)` - List of (namespace, count) tuples
    /// * `Err(_)` - If query fails
    async fn list_namespaces(&self) -> Result<Vec<(String, usize)>>;
}

/// Service interface for embedding generation
///
/// Abstracts the embedding model implementation (local or cloud-based).
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Generate embedding for a single text
    ///
    /// # Arguments
    /// * `text` - The input text to embed
    ///
    /// # Returns
    /// * `Ok(Vec<f32>)` - The embedding vector
    /// * `Err(_)` - If embedding generation fails
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple texts (more efficient)
    ///
    /// # Arguments
    /// * `texts` - The input texts to embed
    ///
    /// # Returns
    /// * `Ok(Vec<Vec<f32>>)` - The embedding vectors
    /// * `Err(_)` - If embedding generation fails
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Get the embedding dimensions for this model
    ///
    /// # Returns
    /// * The number of dimensions in the embedding vectors
    fn dimensions(&self) -> usize;

    /// Get the embedding model type
    ///
    /// # Returns
    /// * The model type being used
    fn model_type(&self) -> EmbeddingModel;
}

/// Service interface for text chunking
///
/// Splits documents into chunks suitable for embedding.
#[async_trait]
pub trait ChunkingService: Send + Sync {
    /// Chunk a document into smaller pieces
    ///
    /// # Arguments
    /// * `text` - The input text to chunk
    /// * `parent_id` - The parent document ID
    ///
    /// # Returns
    /// * `Ok(Vec<Chunk>)` - The chunks
    /// * `Err(_)` - If chunking fails
    async fn chunk(&self, text: &str, parent_id: &str) -> Result<Vec<Chunk>>;

    /// Get the token count for a text
    ///
    /// # Arguments
    /// * `text` - The input text
    ///
    /// # Returns
    /// * `Ok(usize)` - The number of tokens
    /// * `Err(_)` - If tokenization fails
    async fn count_tokens(&self, text: &str) -> Result<usize>;
}

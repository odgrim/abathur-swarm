//! RAG (Retrieval-Augmented Generation) service
//!
//! High-level orchestration for semantic search and document indexing.
//! Coordinates chunking, embedding, and vector storage operations.

use crate::domain::models::{Citation, Chunk, SearchResult};
use crate::domain::ports::{ChunkingService, EmbeddingRepository};
use anyhow::Result;
use std::sync::Arc;
use uuid::Uuid;

/// RAG service for document indexing and semantic search
///
/// This service provides high-level operations for:
/// - Adding documents with automatic chunking and embedding
/// - Retrieving relevant context for queries
/// - Building augmented prompts with context
pub struct RagService {
    vector_store: Arc<dyn EmbeddingRepository>,
    chunker: Arc<dyn ChunkingService>,
}

impl RagService {
    /// Create a new RAG service
    ///
    /// # Arguments
    /// * `vector_store` - Vector storage and search implementation
    /// * `chunker` - Text chunking implementation
    pub fn new(
        vector_store: Arc<dyn EmbeddingRepository>,
        chunker: Arc<dyn ChunkingService>,
    ) -> Self {
        Self {
            vector_store,
            chunker,
        }
    }

    /// Add a document with automatic chunking and embedding
    ///
    /// # Arguments
    /// * `namespace` - Namespace for organizing the document (e.g., "docs:api", "agent:memory")
    /// * `content` - The document content
    /// * `citation` - Optional citation information
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of chunk IDs that were created
    /// * `Err(_)` - If chunking or embedding fails
    pub async fn add_document(
        &self,
        namespace: &str,
        content: &str,
        citation: Option<Citation>,
    ) -> Result<Vec<String>> {
        // Generate a parent document ID
        let parent_id = Uuid::new_v4().to_string();

        // Chunk the document
        let chunks = self.chunker.chunk(content, &parent_id).await?;

        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        tracing::info!(
            "Chunked document into {} chunks for namespace '{}'",
            chunks.len(),
            namespace
        );

        // Insert all chunks with embeddings
        let chunk_ids = self
            .insert_chunks(namespace, &chunks, citation.clone())
            .await?;

        tracing::info!(
            "Successfully indexed {} chunks for namespace '{}'",
            chunk_ids.len(),
            namespace
        );

        Ok(chunk_ids)
    }

    /// Add multiple documents in batch
    ///
    /// # Arguments
    /// * `documents` - List of (namespace, content, citation) tuples
    ///
    /// # Returns
    /// * `Ok(Vec<Vec<String>>)` - List of chunk ID lists for each document
    /// * `Err(_)` - If any operation fails
    pub async fn add_documents_batch(
        &self,
        documents: Vec<(String, String, Option<Citation>)>,
    ) -> Result<Vec<Vec<String>>> {
        let mut all_chunk_ids = Vec::new();

        for (namespace, content, citation) in documents {
            let chunk_ids = self.add_document(&namespace, &content, citation).await?;
            all_chunk_ids.push(chunk_ids);
        }

        Ok(all_chunk_ids)
    }

    /// Retrieve relevant context for a query
    ///
    /// # Arguments
    /// * `query` - The search query
    /// * `limit` - Maximum number of results to return
    /// * `namespace` - Optional namespace filter (e.g., "docs:*")
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Ordered by relevance (most relevant first)
    /// * `Err(_)` - If search fails
    pub async fn retrieve_context(
        &self,
        query: &str,
        limit: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        tracing::debug!("Retrieving context for query: {}", query);

        let results = self
            .vector_store
            .search_similar(query, limit, namespace)
            .await?;

        tracing::debug!("Found {} relevant results", results.len());

        Ok(results)
    }

    /// Build an augmented prompt with retrieved context
    ///
    /// # Arguments
    /// * `original_prompt` - The original user prompt/question
    /// * `context` - Retrieved context from semantic search
    ///
    /// # Returns
    /// * Augmented prompt with context injected
    pub fn build_augmented_prompt(
        &self,
        original_prompt: &str,
        context: &[SearchResult],
    ) -> String {
        if context.is_empty() {
            return original_prompt.to_string();
        }

        let context_str = context
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let citation_info = r
                    .citation
                    .as_ref()
                    .map(|c| format!(" (Source: {})", c.source))
                    .unwrap_or_default();

                format!(
                    "[Context {}]{}\n{}\n",
                    i + 1,
                    citation_info,
                    r.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"Use the following context to help answer the question. If the context doesn't contain relevant information, you can answer based on your general knowledge but mention that the context didn't contain specific information.

## Context

{}

## Question

{}

## Instructions

Answer based on the context provided above when relevant. Cite sources using the context numbers ([Context 1], [Context 2], etc.) when referencing specific information."#,
            context_str, original_prompt
        )
    }

    /// Search for similar content across all namespaces
    ///
    /// # Arguments
    /// * `query` - The search query
    /// * `limit` - Maximum number of results
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Search results
    /// * `Err(_)` - If search fails
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.retrieve_context(query, limit, None).await
    }

    /// Get vector memory count for a namespace
    ///
    /// # Arguments
    /// * `namespace_prefix` - The namespace prefix
    ///
    /// # Returns
    /// * `Ok(usize)` - Count of memories in namespace
    /// * `Err(_)` - If query fails
    pub async fn count(&self, namespace_prefix: &str) -> Result<usize> {
        self.vector_store.count(namespace_prefix).await
    }

    /// List all namespaces with their memory counts
    ///
    /// # Returns
    /// * `Ok(Vec<(String, usize)>)` - List of (namespace, count) tuples
    /// * `Err(_)` - If query fails
    pub async fn list_namespaces(&self) -> Result<Vec<(String, usize)>> {
        self.vector_store.list_namespaces().await
    }

    /// Delete a vector memory by ID
    ///
    /// # Arguments
    /// * `id` - The memory ID
    ///
    /// # Returns
    /// * `Ok(())` - If deletion succeeds
    /// * `Err(_)` - If deletion fails
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.vector_store.delete(id).await
    }

    /// Insert chunks into vector store (internal helper)
    async fn insert_chunks(
        &self,
        namespace: &str,
        chunks: &[Chunk],
        citation: Option<Citation>,
    ) -> Result<Vec<String>> {
        let mut chunk_ids = Vec::new();

        for chunk in chunks {
            let chunk_id = format!("{}:chunk:{}", namespace, chunk.chunk_index);

            let metadata = serde_json::json!({
                "chunk_index": chunk.chunk_index,
                "parent_id": chunk.parent_id,
                "token_count": chunk.token_count,
            });

            self.vector_store
                .insert(&chunk_id, namespace, &chunk.content, Some(metadata), citation.clone())
                .await?;

            chunk_ids.push(chunk_id);
        }

        Ok(chunk_ids)
    }

    /// Migrate existing memories to vector storage
    ///
    /// This is useful for migrating from the old prefix-based memory system
    /// to the new vector-based semantic search system.
    ///
    /// # Arguments
    /// * `namespace` - Namespace to migrate
    /// * `memories` - List of (key, content) tuples
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of memories migrated
    /// * `Err(_)` - If migration fails
    pub async fn migrate_memories(
        &self,
        namespace: &str,
        memories: Vec<(String, String)>,
    ) -> Result<usize> {
        if memories.is_empty() {
            return Ok(0);
        }

        tracing::info!("Migrating {} memories to namespace '{}'", memories.len(), namespace);

        let batch: Vec<(String, String, String)> = memories
            .into_iter()
            .map(|(key, content)| {
                let id = format!("{}:{}", namespace, key);
                (id, namespace.to_string(), content)
            })
            .collect();

        let rowids = self.vector_store.insert_batch(batch).await?;

        tracing::info!("Successfully migrated {} memories", rowids.len());

        Ok(rowids.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{ChunkingConfig, EmbeddingModel};
    use crate::infrastructure::vector::{Chunker, LocalEmbeddingService, VectorStore};
    use sqlx::SqlitePool;
    use tempfile::TempDir;

    async fn setup_test_service() -> Result<(RagService, TempDir)> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        let pool = SqlitePool::connect(&format!("sqlite://{}?mode=rwc", db_path.display())).await?;

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        let embedding_service = Arc::new(LocalEmbeddingService::new(EmbeddingModel::LocalMiniLM)?);
        let vector_store = Arc::new(VectorStore::new(Arc::new(pool), embedding_service)?);
        let chunker = Arc::new(Chunker::with_config(ChunkingConfig::small())?);

        let service = RagService::new(vector_store, chunker);

        Ok((service, temp_dir))
    }

    #[tokio::test]
    #[ignore] // Ignore by default as it requires model download
    async fn test_add_document() {
        let (service, _temp) = setup_test_service().await.unwrap();

        let content = "This is a test document about RAG. RAG stands for Retrieval-Augmented Generation.";
        let citation = Citation::from_file("test.txt".to_string());

        let chunk_ids = service
            .add_document("test:docs", content, Some(citation))
            .await
            .unwrap();

        assert!(!chunk_ids.is_empty());
    }

    #[tokio::test]
    #[ignore] // Ignore by default as it requires model download
    async fn test_retrieve_context() {
        let (service, _temp) = setup_test_service().await.unwrap();

        let content = "Rust is a systems programming language focused on safety and performance.";
        service
            .add_document("test:docs", content, None)
            .await
            .unwrap();

        let results = service
            .retrieve_context("What is Rust?", 5, Some("test:"))
            .await
            .unwrap();

        assert!(!results.is_empty());
    }

    #[test]
    fn test_build_augmented_prompt() {
        let vector_store = Arc::new(MockVectorStore);
        let chunker = Arc::new(MockChunker);
        let service = RagService::new(vector_store, chunker);

        let results = vec![SearchResult::new(
            "id1".to_string(),
            "ns".to_string(),
            "Some relevant context".to_string(),
            0.1,
            serde_json::json!({}),
            None,
        )];

        let prompt = service.build_augmented_prompt("What is Rust?", &results);

        assert!(prompt.contains("Context"));
        assert!(prompt.contains("Some relevant context"));
        assert!(prompt.contains("What is Rust?"));
    }

    // Mock implementations for testing
    struct MockVectorStore;

    #[async_trait]
    impl EmbeddingRepository for MockVectorStore {
        async fn insert(&self, _: &str, _: &str, _: &str, _: Option<serde_json::Value>, _: Option<Citation>) -> Result<i64> {
            Ok(1)
        }
        async fn insert_batch(&self, _: Vec<(String, String, String)>) -> Result<Vec<i64>> {
            Ok(vec![])
        }
        async fn search_similar(&self, _: &str, _: usize, _: Option<&str>) -> Result<Vec<SearchResult>> {
            Ok(vec![])
        }
        async fn hybrid_search(&self, _: &str, _: usize, _: f32) -> Result<Vec<SearchResult>> {
            Ok(vec![])
        }
        async fn get(&self, _: &str) -> Result<Option<crate::domain::models::VectorMemory>> {
            Ok(None)
        }
        async fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn count(&self, _: &str) -> Result<usize> {
            Ok(0)
        }
        async fn list_namespaces(&self) -> Result<Vec<(String, usize)>> {
            Ok(vec![])
        }
    }

    struct MockChunker;

    #[async_trait]
    impl ChunkingService for MockChunker {
        async fn chunk(&self, _: &str, _: &str) -> Result<Vec<Chunk>> {
            Ok(vec![])
        }
        async fn count_tokens(&self, _: &str) -> Result<usize> {
            Ok(0)
        }
    }
}

use crate::domain::models::{Citation, Memory, MemoryType, SearchResult};
use crate::domain::ports::{ChunkingService, EmbeddingRepository, MemoryRepository};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde_json::Value;
use std::sync::Arc;
use tracing::{error, info, instrument};
use uuid::Uuid;

/// Service for managing memory operations
///
/// Coordinates memory CRUD operations with the repository layer, providing
/// business logic for versioning, soft deletes, namespace management, and
/// optional vector search capabilities.
///
/// # Examples
///
/// ```no_run
/// use abathur::services::MemoryService;
/// use abathur::domain::models::{Memory, MemoryType};
/// use std::sync::Arc;
/// use serde_json::json;
///
/// # async fn example(repo: Arc<dyn abathur::domain::ports::MemoryRepository>) -> anyhow::Result<()> {
/// let service = MemoryService::new(repo, None, None);
///
/// // Add a new memory
/// let memory = Memory::new(
///     "user:alice".to_string(),
///     "preferences".to_string(),
///     json!({"theme": "dark"}),
///     MemoryType::Semantic,
///     "alice".to_string(),
/// );
/// service.add(memory).await?;
///
/// // Get the latest version
/// let retrieved = service.get("user:alice", "preferences").await?;
/// # Ok(())
/// # }
/// ```
pub struct MemoryService {
    repo: Arc<dyn MemoryRepository>,
    vector_store: Option<Arc<dyn EmbeddingRepository>>,
    chunker: Option<Arc<dyn ChunkingService>>,
}

impl MemoryService {
    /// Create a new `MemoryService` with the given repository and optional vector capabilities
    ///
    /// # Arguments
    /// * `repo` - Arc-wrapped trait object implementing `MemoryRepository`
    /// * `vector_store` - Optional vector storage for semantic search
    /// * `chunker` - Optional text chunking service
    ///
    /// Vector search features are only available if both `vector_store` and `chunker` are provided.
    pub fn new(
        repo: Arc<dyn MemoryRepository>,
        vector_store: Option<Arc<dyn EmbeddingRepository>>,
        chunker: Option<Arc<dyn ChunkingService>>,
    ) -> Self {
        if vector_store.is_some() && chunker.is_some() {
            info!("MemoryService initialized with vector search capabilities");
        } else {
            info!("MemoryService initialized without vector search (graceful degradation)");
        }
        Self {
            repo,
            vector_store,
            chunker,
        }
    }

    /// Add a new memory entry
    ///
    /// Validates the memory and inserts it into the repository. The memory
    /// will be assigned version 1 automatically. If vector search capabilities
    /// are enabled, the memory will also be indexed for semantic search.
    ///
    /// # Arguments
    /// * `memory` - The memory entry to add
    ///
    /// # Returns
    /// * `Ok(i64)` - The database ID of the inserted memory
    /// * `Err(_)` - If validation or insertion fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - Memory already exists (namespace + key combination)
    /// - Repository insert operation fails
    ///
    /// # Note
    /// Vector indexing errors are logged but don't fail the operation.
    /// Traditional memory storage always succeeds even if vector indexing fails.
    #[instrument(skip(self, memory), fields(namespace = %memory.namespace, key = %memory.key), err)]
    pub async fn add(&self, memory: Memory) -> Result<i64> {
        // Validate memory doesn't already exist
        if let Some(existing) = self
            .repo
            .get(&memory.namespace, &memory.key)
            .await
            .context("Failed to check for existing memory")?
            && existing.is_active()
        {
            return Err(anyhow!(
                "Memory already exists at {}:{}. Use update() to modify it.",
                memory.namespace,
                memory.key
            ));
        }

        let namespace = memory.namespace.clone();
        let key = memory.key.clone();
        let value = memory.value.clone();
        let created_by = memory.created_by.clone();

        // 1. Insert into traditional memory
        let id = self
            .repo
            .insert(memory)
            .await
            .context("Failed to insert memory")?;

        info!("Memory added successfully with ID: {}", id);

        // 2. ALSO index in vector storage for semantic search (graceful degradation)
        if let (Some(vector_store), Some(chunker)) = (&self.vector_store, &self.chunker) {
            // Extract text content from the JSON value
            let text_content = match &value {
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other)
                    .unwrap_or_else(|_| other.to_string()),
            };

            // Create citation with namespace:key for traceability
            let citation = Citation {
                source: format!("memory:{}:{} (created by {})", namespace, key, created_by),
                page: None,
                url: None,
                timestamp: Utc::now(),
            };

            // Generate a parent document ID for chunking
            let parent_id = Uuid::new_v4().to_string();

            // Chunk and index (errors don't fail the overall operation)
            match chunker.chunk(&text_content, &parent_id).await {
                Ok(chunks) if !chunks.is_empty() => {
                    // Insert all chunks with embeddings
                    for chunk in chunks {
                        let chunk_metadata = serde_json::json!({
                            "parent_id": parent_id,
                            "chunk_index": chunk.chunk_index,
                            "token_count": chunk.token_count,
                        });

                        match vector_store
                            .insert(
                                &chunk.id,
                                &namespace,
                                &chunk.content,
                                Some(chunk_metadata),
                                Some(citation.clone()),
                            )
                            .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                error!("Failed to insert chunk to vector storage: {}", e);
                            }
                        }
                    }
                    info!(
                        "Memory also indexed for semantic search: namespace={}, chunks={}",
                        namespace,
                        parent_id
                    );
                }
                Ok(_) => {
                    // Empty chunks, skip vector storage
                }
                Err(e) => {
                    // Log but don't fail - traditional memory still works
                    error!("Failed to chunk memory for vector storage: {}", e);
                }
            }
        }

        Ok(id)
    }

    /// Get the latest version of a memory
    ///
    /// Retrieves the most recent version of a memory entry by namespace and key.
    /// Returns None if the memory doesn't exist or has been soft deleted.
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    ///
    /// # Returns
    /// * `Ok(Some(Memory))` - The latest version if found and active
    /// * `Ok(None)` - If not found or deleted
    /// * `Err(_)` - If query fails
    #[instrument(skip(self), err)]
    pub async fn get(&self, namespace: &str, key: &str) -> Result<Option<Memory>> {
        self.repo
            .get(namespace, key)
            .await
            .context("Failed to retrieve memory")
    }


    /// Search memories by namespace prefix and optional type
    ///
    /// Returns the latest version of each memory matching the criteria,
    /// excluding soft-deleted entries.
    ///
    /// # Arguments
    /// * `namespace_prefix` - Prefix to match (e.g., "user:alice" matches "user:alice:*")
    /// * `memory_type` - Optional filter by memory type
    /// * `limit` - Maximum number of results (defaults to 50)
    ///
    /// # Returns
    /// * `Ok(Vec<Memory>)` - List of matching memories
    /// * `Err(_)` - If query fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use abathur::services::MemoryService;
    /// # use abathur::domain::models::MemoryType;
    /// # use std::sync::Arc;
    /// # async fn example(service: &MemoryService) -> anyhow::Result<()> {
    /// // Search all semantic memories for user alice
    /// let memories = service.search(
    ///     "user:alice",
    ///     Some(MemoryType::Semantic),
    ///     Some(100)
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), err)]
    pub async fn search(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
        limit: Option<usize>,
    ) -> Result<Vec<Memory>> {
        let limit = limit.unwrap_or(50);

        self.repo
            .search(namespace_prefix, memory_type, limit)
            .await
            .context("Failed to search memories")
    }

    /// Update a memory
    ///
    /// Updates the memory with a new value.
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    /// * `value` - The new value
    /// * `updated_by` - Identifier of who is updating
    ///
    /// # Returns
    /// * `Ok(())` - If update succeeds
    /// * `Err(_)` - If update fails or memory not found
    ///
    /// # Errors
    /// Returns an error if:
    /// - Memory doesn't exist
    /// - Memory has been soft deleted
    /// - Repository update operation fails
    #[instrument(skip(self, value), err)]
    pub async fn update(
        &self,
        namespace: &str,
        key: &str,
        value: Value,
        updated_by: &str,
    ) -> Result<()> {
        // Verify memory exists and is active
        let existing = self
            .repo
            .get(namespace, key)
            .await
            .context("Failed to check existing memory")?
            .ok_or_else(|| anyhow!("Memory not found at {namespace}:{key}"))?;

        if !existing.is_active() {
            return Err(anyhow!("Cannot update deleted memory at {namespace}:{key}"));
        }

        // Update via repository
        self.repo
            .update(namespace, key, value, updated_by)
            .await
            .context("Failed to update memory")
    }

    /// Soft delete a memory
    ///
    /// Marks the memory as deleted without physically removing it from storage.
    /// Deleted memories won't appear in `get()` or `search()` results.
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    ///
    /// # Returns
    /// * `Ok(())` - If successfully deleted
    /// * `Err(_)` - If deletion fails or memory not found
    ///
    /// # Errors
    /// Returns an error if:
    /// - Memory doesn't exist
    /// - Repository delete operation fails
    #[instrument(skip(self), err)]
    pub async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        // Verify memory exists
        self.repo
            .get(namespace, key)
            .await
            .context("Failed to check existing memory")?
            .ok_or_else(|| anyhow!("Memory not found at {namespace}:{key}"))?;

        // Soft delete
        self.repo
            .delete(namespace, key)
            .await
            .context("Failed to delete memory")
    }

    /// Count memories matching criteria
    ///
    /// # Arguments
    /// * `namespace_prefix` - Prefix to match
    /// * `memory_type` - Optional filter by type
    ///
    /// # Returns
    /// * `Ok(usize)` - Count of matching memories (excluding deleted)
    /// * `Err(_)` - If query fails
    #[instrument(skip(self), err)]
    pub async fn count(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
    ) -> Result<usize> {
        self.repo
            .count(namespace_prefix, memory_type)
            .await
            .context("Failed to count memories")
    }

    /// Semantic search across vector memories using natural language queries
    ///
    /// Performs similarity search across all memories that have been indexed
    /// for vector search. Returns memories ordered by semantic relevance.
    ///
    /// # Arguments
    /// * `query` - Natural language search query
    /// * `limit` - Maximum number of results to return
    /// * `namespace_filter` - Optional namespace prefix to filter results
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Ordered by relevance (most relevant first)
    /// * `Err(_)` - If search fails or vector search not available
    ///
    /// # Errors
    /// Returns an error if:
    /// - Vector search capabilities not initialized
    /// - Search operation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use abathur::services::MemoryService;
    /// # use std::sync::Arc;
    /// # async fn example(service: &MemoryService) -> anyhow::Result<()> {
    /// // Search for authentication-related memories
    /// let results = service.vector_search(
    ///     "authentication and authorization implementation",
    ///     10,
    ///     Some("docs:")
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), err)]
    pub async fn vector_search(
        &self,
        query: &str,
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let vector_store = self
            .vector_store
            .as_ref()
            .ok_or_else(|| anyhow!("Vector search not available. Initialize MemoryService with vector_store and chunker."))?;

        info!("Performing vector search: query='{}', limit={}", query, limit);

        vector_store
            .search_similar(query, limit, namespace_filter)
            .await
            .context("Vector search failed")
    }

    /// List all vector memory namespaces with their document counts
    ///
    /// Returns a list of namespaces that have memories indexed for vector search,
    /// along with the count of documents in each namespace.
    ///
    /// # Returns
    /// * `Ok(Vec<(String, usize)>)` - List of (namespace, count) tuples
    /// * `Err(_)` - If operation fails or vector search not available
    ///
    /// # Errors
    /// Returns an error if:
    /// - Vector search capabilities not initialized
    /// - List operation fails
    #[instrument(skip(self), err)]
    pub async fn list_vector_namespaces(&self) -> Result<Vec<(String, usize)>> {
        let vector_store = self
            .vector_store
            .as_ref()
            .ok_or_else(|| anyhow!("Vector search not available. Initialize MemoryService with vector_store and chunker."))?;

        vector_store
            .list_namespaces()
            .await
            .context("Failed to list vector namespaces")
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    use mockall::predicate::*;
    use serde_json::json;

    mock! {
        MemoryRepo {}

        #[async_trait::async_trait]
        impl MemoryRepository for MemoryRepo {
            async fn insert(&self, memory: Memory) -> Result<i64>;
            async fn get(&self, namespace: &str, key: &str) -> Result<Option<Memory>>;
            async fn search(
                &self,
                namespace_prefix: &str,
                memory_type: Option<MemoryType>,
                limit: usize,
            ) -> Result<Vec<Memory>>;
            async fn update(
                &self,
                namespace: &str,
                key: &str,
                value: Value,
                updated_by: &str,
            ) -> Result<()>;
            async fn delete(&self, namespace: &str, key: &str) -> Result<()>;
            async fn count(
                &self,
                namespace_prefix: &str,
                memory_type: Option<MemoryType>,
            ) -> Result<usize>;
        }
    }

    fn create_test_memory() -> Memory {
        Memory::new(
            "test:namespace".to_string(),
            "key1".to_string(),
            json!({"data": "value"}),
            MemoryType::Semantic,
            "test_user".to_string(),
        )
    }

    #[tokio::test]
    async fn test_add_new_memory() {
        let mut mock_repo = MockMemoryRepo::new();
        let memory = create_test_memory();

        // Expect get to return None (doesn't exist)
        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(|_, _| Ok(None));

        // Expect insert to succeed
        mock_repo.expect_insert().times(1).returning(|_| Ok(42));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service.add(memory).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_add_existing_memory_fails() {
        let mut mock_repo = MockMemoryRepo::new();
        let memory = create_test_memory();
        let existing = create_test_memory();

        // Expect get to return existing memory
        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(move |_, _| Ok(Some(existing.clone())));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service.add(memory).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn test_get_memory() {
        let mut mock_repo = MockMemoryRepo::new();
        let expected = create_test_memory();

        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(move |_, _| Ok(Some(expected.clone())));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service.get("test:namespace", "key1").await;

        assert!(result.is_ok());
        let memory = result.unwrap();
        assert!(memory.is_some());
        assert_eq!(memory.unwrap().namespace, "test:namespace");
    }

    #[tokio::test]
    async fn test_get_nonexistent_memory() {
        let mut mock_repo = MockMemoryRepo::new();

        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(|_, _| Ok(None));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service.get("test:namespace", "key1").await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_search_memories() {
        let mut mock_repo = MockMemoryRepo::new();
        let memory1 = create_test_memory();
        let memory2 = Memory::new(
            "test:namespace".to_string(),
            "key2".to_string(),
            json!({"data": "value2"}),
            MemoryType::Semantic,
            "test_user".to_string(),
        );

        mock_repo
            .expect_search()
            .with(eq("test:namespace"), eq(Some(MemoryType::Semantic)), eq(50))
            .times(1)
            .returning(move |_, _, _| Ok(vec![memory1.clone(), memory2.clone()]));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service
            .search("test:namespace", Some(MemoryType::Semantic), None)
            .await;

        assert!(result.is_ok());
        let memories = result.unwrap();
        assert_eq!(memories.len(), 2);
    }

    #[tokio::test]
    async fn test_update_memory() {
        let mut mock_repo = MockMemoryRepo::new();
        let existing = create_test_memory();
        let new_value = json!({"data": "updated"});

        // Expect get to return existing memory
        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(move |_, _| Ok(Some(existing.clone())));

        // Expect update to succeed
        mock_repo
            .expect_update()
            .with(
                eq("test:namespace"),
                eq("key1"),
                eq(new_value.clone()),
                eq("updater"),
            )
            .times(1)
            .returning(|_, _, _, _| Ok(()));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service
            .update("test:namespace", "key1", new_value, "updater")
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_nonexistent_memory_fails() {
        let mut mock_repo = MockMemoryRepo::new();

        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(|_, _| Ok(None));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service
            .update("test:namespace", "key1", json!({}), "updater")
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_update_deleted_memory_fails() {
        let mut mock_repo = MockMemoryRepo::new();
        let mut deleted = create_test_memory();
        deleted.mark_deleted();

        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(move |_, _| Ok(Some(deleted.clone())));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service
            .update("test:namespace", "key1", json!({}), "updater")
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("deleted"));
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let mut mock_repo = MockMemoryRepo::new();
        let existing = create_test_memory();

        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(move |_, _| Ok(Some(existing.clone())));

        mock_repo
            .expect_delete()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(|_, _| Ok(()));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service.delete("test:namespace", "key1").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_memory_fails() {
        let mut mock_repo = MockMemoryRepo::new();

        mock_repo
            .expect_get()
            .with(eq("test:namespace"), eq("key1"))
            .times(1)
            .returning(|_, _| Ok(None));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service.delete("test:namespace", "key1").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_count_memories() {
        let mut mock_repo = MockMemoryRepo::new();

        mock_repo
            .expect_count()
            .with(eq("test:namespace"), eq(Some(MemoryType::Semantic)))
            .times(1)
            .returning(|_, _| Ok(5));

        let service = MemoryService::new(Arc::new(mock_repo), None, None);
        let result = service
            .count("test:namespace", Some(MemoryType::Semantic))
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5);
    }

}

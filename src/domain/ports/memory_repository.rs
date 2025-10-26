use crate::domain::models::{Memory, MemoryType};
use anyhow::Result;
use async_trait::async_trait;

/// Repository interface for memory storage operations
///
/// Provides CRUD operations for memories with versioning and soft delete support.
/// Implementations should handle database-specific details while maintaining
/// the interface contract.
#[async_trait]
pub trait MemoryRepository: Send + Sync {
    /// Insert a new memory entry
    ///
    /// # Arguments
    /// * `memory` - The memory entry to insert
    ///
    /// # Returns
    /// * `Ok(i64)` - The database ID of the inserted memory
    /// * `Err(_)` - If insertion fails
    async fn insert(&self, memory: Memory) -> Result<i64>;

    /// Get the latest version of a memory by namespace and key
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    ///
    /// # Returns
    /// * `Ok(Some(Memory))` - The latest version if found and not deleted
    /// * `Ok(None)` - If not found or soft deleted
    /// * `Err(_)` - If query fails
    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Memory>>;

    /// Get a specific version of a memory
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    /// * `version` - The version number to retrieve
    ///
    /// # Returns
    /// * `Ok(Some(Memory))` - The specific version if found
    /// * `Ok(None)` - If not found
    /// * `Err(_)` - If query fails
    async fn get_version(&self, namespace: &str, key: &str, version: u32) -> Result<Option<Memory>>;

    /// Search memories by namespace prefix and optional type filter
    ///
    /// # Arguments
    /// * `namespace_prefix` - The namespace prefix to match (e.g., "user:alice" matches "user:alice:*")
    /// * `memory_type` - Optional filter by memory type
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    /// * `Ok(Vec<Memory>)` - List of matching memories (latest versions only, excluding deleted)
    /// * `Err(_)` - If query fails
    async fn search(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
        limit: usize,
    ) -> Result<Vec<Memory>>;

    /// Update a memory (creates a new version)
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    /// * `value` - The new value
    /// * `updated_by` - The identifier of who is updating
    ///
    /// # Returns
    /// * `Ok(u32)` - The new version number
    /// * `Err(_)` - If update fails or memory not found
    async fn update(
        &self,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
        updated_by: &str,
    ) -> Result<u32>;

    /// Soft delete a memory (marks as deleted)
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    ///
    /// # Returns
    /// * `Ok(())` - If successfully marked as deleted
    /// * `Err(_)` - If deletion fails or memory not found
    async fn delete(&self, namespace: &str, key: &str) -> Result<()>;

    /// Count memories matching criteria
    ///
    /// # Arguments
    /// * `namespace_prefix` - The namespace prefix to match
    /// * `memory_type` - Optional filter by memory type
    ///
    /// # Returns
    /// * `Ok(usize)` - Count of matching memories (excluding deleted)
    /// * `Err(_)` - If query fails
    async fn count(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
    ) -> Result<usize>;

    /// List all versions of a memory
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The key within the namespace
    ///
    /// # Returns
    /// * `Ok(Vec<Memory>)` - All versions sorted by version number
    /// * `Err(_)` - If query fails
    async fn list_versions(&self, namespace: &str, key: &str) -> Result<Vec<Memory>>;
}

use crate::domain::models::{Memory, MemoryType};
use anyhow::Result;
use async_trait::async_trait;

/// Repository trait for memory storage operations
///
/// Provides async methods for CRUD operations on memories with support for:
/// - Hierarchical namespaces
/// - Versioning on updates
/// - Soft deletes
/// - Namespace prefix searching
/// - Memory type filtering
#[async_trait]
pub trait MemoryRepository: Send + Sync {
    /// Add a new memory entry
    ///
    /// # Arguments
    /// * `memory` - The memory to add
    ///
    /// # Errors
    /// Returns error if:
    /// - Namespace+key already exists
    /// - Database operation fails
    async fn add(&self, memory: Memory) -> Result<()>;

    /// Get a memory by namespace and key
    ///
    /// Returns None if:
    /// - Memory doesn't exist
    /// - Memory is soft-deleted
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The unique key within the namespace
    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Memory>>;

    /// Search memories by namespace prefix and optional memory type
    ///
    /// Supports hierarchical namespace queries like "user:alice" to find all
    /// memories under "user:alice:*".
    ///
    /// Does NOT return soft-deleted memories.
    ///
    /// # Arguments
    /// * `namespace_prefix` - Namespace prefix to match (e.g., "user:alice")
    /// * `memory_type` - Optional memory type filter
    async fn search(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
    ) -> Result<Vec<Memory>>;

    /// Update a memory value and increment version
    ///
    /// This method:
    /// - Updates the value field
    /// - Increments the version number
    /// - Updates the updated_at timestamp
    /// - Sets updated_by
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The unique key within the namespace
    /// * `value` - The new JSON value
    /// * `updated_by` - User or agent performing the update
    ///
    /// # Errors
    /// Returns error if:
    /// - Memory doesn't exist
    /// - Memory is soft-deleted
    /// - Database operation fails
    async fn update(
        &self,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
        updated_by: &str,
    ) -> Result<()>;

    /// Soft-delete a memory
    ///
    /// Sets is_deleted=1 instead of removing the row.
    /// Soft-deleted memories are excluded from get() and search() results.
    ///
    /// # Arguments
    /// * `namespace` - The hierarchical namespace
    /// * `key` - The unique key within the namespace
    async fn delete(&self, namespace: &str, key: &str) -> Result<()>;
}

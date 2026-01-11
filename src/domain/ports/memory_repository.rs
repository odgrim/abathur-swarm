//! Memory repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Memory, MemoryQuery, MemoryTier};

/// Repository interface for Memory persistence.
#[async_trait]
pub trait MemoryRepository: Send + Sync {
    /// Store a memory entry.
    async fn store(&self, memory: &Memory) -> DomainResult<()>;

    /// Get a memory by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<Memory>>;

    /// Get a memory by key and namespace.
    async fn get_by_key(&self, key: &str, namespace: &str) -> DomainResult<Option<Memory>>;

    /// Update an existing memory.
    async fn update(&self, memory: &Memory) -> DomainResult<()>;

    /// Delete a memory by ID.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// Query memories with filters.
    async fn query(&self, query: MemoryQuery) -> DomainResult<Vec<Memory>>;

    /// Full-text search in memory content.
    async fn search(&self, query: &str, namespace: Option<&str>, limit: usize) -> DomainResult<Vec<Memory>>;

    /// Get memories by tier.
    async fn list_by_tier(&self, tier: MemoryTier) -> DomainResult<Vec<Memory>>;

    /// Get memories by namespace.
    async fn list_by_namespace(&self, namespace: &str) -> DomainResult<Vec<Memory>>;

    /// Get expired memories.
    async fn get_expired(&self) -> DomainResult<Vec<Memory>>;

    /// Delete expired memories.
    async fn prune_expired(&self) -> DomainResult<u64>;

    /// Get memories with decay factor below threshold.
    async fn get_decayed(&self, threshold: f32) -> DomainResult<Vec<Memory>>;

    /// Get memories for a specific task.
    async fn get_for_task(&self, task_id: Uuid) -> DomainResult<Vec<Memory>>;

    /// Get memories for a specific goal.
    async fn get_for_goal(&self, goal_id: Uuid) -> DomainResult<Vec<Memory>>;

    /// Count memories by tier.
    async fn count_by_tier(&self) -> DomainResult<std::collections::HashMap<MemoryTier, u64>>;
}

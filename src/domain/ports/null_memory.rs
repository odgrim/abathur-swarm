//! Null memory repository implementation.
//!
//! Used when memory features are not needed but the type system
//! requires a MemoryRepository implementation.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Memory, MemoryQuery, MemoryTier};
use super::MemoryRepository;

/// A no-op memory repository that stores nothing.
///
/// Use this when memory features are disabled or not needed.
#[derive(Debug, Clone, Default)]
pub struct NullMemoryRepository;

impl NullMemoryRepository {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MemoryRepository for NullMemoryRepository {
    async fn store(&self, _memory: &Memory) -> DomainResult<()> {
        Ok(())
    }

    async fn get(&self, _id: Uuid) -> DomainResult<Option<Memory>> {
        Ok(None)
    }

    async fn get_by_key(&self, _key: &str, _namespace: &str) -> DomainResult<Option<Memory>> {
        Ok(None)
    }

    async fn update(&self, _memory: &Memory) -> DomainResult<()> {
        Ok(())
    }

    async fn delete(&self, _id: Uuid) -> DomainResult<()> {
        Ok(())
    }

    async fn query(&self, _query: MemoryQuery) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn search(&self, _query: &str, _namespace: Option<&str>, _limit: usize) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn list_by_tier(&self, _tier: MemoryTier) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn list_by_namespace(&self, _namespace: &str) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn get_expired(&self) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn prune_expired(&self) -> DomainResult<u64> {
        Ok(0)
    }

    async fn get_decayed(&self, _threshold: f32) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn get_for_task(&self, _task_id: Uuid) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn get_for_goal(&self, _goal_id: Uuid) -> DomainResult<Vec<Memory>> {
        Ok(Vec::new())
    }

    async fn count_by_tier(&self) -> DomainResult<std::collections::HashMap<MemoryTier, u64>> {
        Ok(std::collections::HashMap::new())
    }
}

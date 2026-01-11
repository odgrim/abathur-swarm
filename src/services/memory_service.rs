//! Memory service implementing business logic with decay management.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Memory, MemoryMetadata, MemoryQuery, MemoryTier, MemoryType};
use crate::domain::ports::MemoryRepository;

/// Configuration for memory decay thresholds.
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// Decay threshold below which working memories are pruned
    pub working_prune_threshold: f32,
    /// Decay threshold below which episodic memories are pruned
    pub episodic_prune_threshold: f32,
    /// Access count threshold for promotion to episodic
    pub promote_to_episodic_threshold: u32,
    /// Access count threshold for promotion to semantic
    pub promote_to_semantic_threshold: u32,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            working_prune_threshold: 0.1,
            episodic_prune_threshold: 0.05,
            promote_to_episodic_threshold: 5,
            promote_to_semantic_threshold: 20,
        }
    }
}

pub struct MemoryService<R: MemoryRepository> {
    repository: Arc<R>,
    decay_config: DecayConfig,
}

impl<R: MemoryRepository> MemoryService<R> {
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            decay_config: DecayConfig::default(),
        }
    }

    pub fn with_decay_config(mut self, config: DecayConfig) -> Self {
        self.decay_config = config;
        self
    }

    /// Store a new memory.
    pub async fn store(
        &self,
        key: String,
        content: String,
        namespace: String,
        tier: MemoryTier,
        memory_type: MemoryType,
        metadata: Option<MemoryMetadata>,
    ) -> DomainResult<Memory> {
        let mut memory = match tier {
            MemoryTier::Working => Memory::working(key, content),
            MemoryTier::Episodic => Memory::episodic(key, content),
            MemoryTier::Semantic => Memory::semantic(key, content),
        };

        memory = memory.with_namespace(namespace).with_type(memory_type);

        if let Some(meta) = metadata {
            memory.metadata = meta;
        }

        memory.validate().map_err(DomainError::ValidationFailed)?;
        self.repository.store(&memory).await?;

        Ok(memory)
    }

    /// Store a working memory (convenience method).
    pub async fn remember(
        &self,
        key: String,
        content: String,
        namespace: &str,
    ) -> DomainResult<Memory> {
        self.store(
            key,
            content,
            namespace.to_string(),
            MemoryTier::Working,
            MemoryType::Fact,
            None,
        ).await
    }

    /// Store a semantic memory (long-term).
    pub async fn learn(
        &self,
        key: String,
        content: String,
        namespace: &str,
    ) -> DomainResult<Memory> {
        self.store(
            key,
            content,
            namespace.to_string(),
            MemoryTier::Semantic,
            MemoryType::Pattern,
            None,
        ).await
    }

    /// Get a memory by ID and record the access.
    pub async fn recall(&self, id: Uuid) -> DomainResult<Option<Memory>> {
        let memory = self.repository.get(id).await?;

        if let Some(mut mem) = memory {
            mem.record_access();
            self.repository.update(&mem).await?;

            // Check if should be promoted
            self.check_promotion(&mut mem).await?;

            Ok(Some(mem))
        } else {
            Ok(None)
        }
    }

    /// Get a memory by key and namespace.
    pub async fn recall_by_key(&self, key: &str, namespace: &str) -> DomainResult<Option<Memory>> {
        let memory = self.repository.get_by_key(key, namespace).await?;

        if let Some(mut mem) = memory {
            mem.record_access();
            self.repository.update(&mem).await?;
            self.check_promotion(&mut mem).await?;
            Ok(Some(mem))
        } else {
            Ok(None)
        }
    }

    /// Query memories without recording access.
    pub async fn query(&self, query: MemoryQuery) -> DomainResult<Vec<Memory>> {
        self.repository.query(query).await
    }

    /// Full-text search in memories.
    pub async fn search(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> DomainResult<Vec<Memory>> {
        self.repository.search(query, namespace, limit).await
    }

    /// Get memories for a specific task.
    pub async fn get_task_context(&self, task_id: Uuid) -> DomainResult<Vec<Memory>> {
        self.repository.get_for_task(task_id).await
    }

    /// Get memories for a specific goal.
    pub async fn get_goal_context(&self, goal_id: Uuid) -> DomainResult<Vec<Memory>> {
        self.repository.get_for_goal(goal_id).await
    }

    /// Delete a memory.
    pub async fn forget(&self, id: Uuid) -> DomainResult<()> {
        self.repository.delete(id).await
    }

    /// Prune expired memories.
    pub async fn prune_expired(&self) -> DomainResult<u64> {
        self.repository.prune_expired().await
    }

    /// Prune decayed memories (below threshold).
    pub async fn prune_decayed(&self) -> DomainResult<u64> {
        let mut count = 0;

        // Prune working memories
        let decayed = self.repository.get_decayed(self.decay_config.working_prune_threshold).await?;
        for mem in decayed {
            if mem.tier == MemoryTier::Working {
                self.repository.delete(mem.id).await?;
                count += 1;
            }
        }

        // Prune episodic memories
        let decayed = self.repository.get_decayed(self.decay_config.episodic_prune_threshold).await?;
        for mem in decayed {
            if mem.tier == MemoryTier::Episodic {
                self.repository.delete(mem.id).await?;
                count += 1;
            }
        }

        Ok(count)
    }

    /// Run full maintenance: prune expired and decayed.
    pub async fn run_maintenance(&self) -> DomainResult<MaintenanceReport> {
        let expired = self.prune_expired().await?;
        let decayed = self.prune_decayed().await?;

        // Check for promotion candidates
        let promoted = self.check_all_promotions().await?;

        Ok(MaintenanceReport {
            expired_pruned: expired,
            decayed_pruned: decayed,
            promoted,
        })
    }

    /// Check if a memory should be promoted based on access patterns.
    async fn check_promotion(&self, memory: &mut Memory) -> DomainResult<bool> {
        let should_promote = match memory.tier {
            MemoryTier::Working => {
                memory.access_count >= self.decay_config.promote_to_episodic_threshold
            }
            MemoryTier::Episodic => {
                memory.access_count >= self.decay_config.promote_to_semantic_threshold
            }
            MemoryTier::Semantic => false,
        };

        if should_promote {
            memory.promote().map_err(DomainError::ValidationFailed)?;
            self.repository.update(memory).await?;
            return Ok(true);
        }

        Ok(false)
    }

    /// Check all non-semantic memories for promotion.
    async fn check_all_promotions(&self) -> DomainResult<u64> {
        let mut promoted = 0;

        // Check working memories
        let working = self.repository.list_by_tier(MemoryTier::Working).await?;
        for mut mem in working {
            if mem.access_count >= self.decay_config.promote_to_episodic_threshold
                && self.check_promotion(&mut mem).await? {
                    promoted += 1;
                }
        }

        // Check episodic memories
        let episodic = self.repository.list_by_tier(MemoryTier::Episodic).await?;
        for mut mem in episodic {
            if mem.access_count >= self.decay_config.promote_to_semantic_threshold
                && self.check_promotion(&mut mem).await? {
                    promoted += 1;
                }
        }

        Ok(promoted)
    }

    /// Get memory statistics.
    pub async fn get_stats(&self) -> DomainResult<MemoryStats> {
        let counts = self.repository.count_by_tier().await?;

        Ok(MemoryStats {
            working_count: *counts.get(&MemoryTier::Working).unwrap_or(&0),
            episodic_count: *counts.get(&MemoryTier::Episodic).unwrap_or(&0),
            semantic_count: *counts.get(&MemoryTier::Semantic).unwrap_or(&0),
        })
    }
}

/// Report from maintenance run.
#[derive(Debug, Clone)]
pub struct MaintenanceReport {
    pub expired_pruned: u64,
    pub decayed_pruned: u64,
    pub promoted: u64,
}

/// Memory statistics.
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub working_count: u64,
    pub episodic_count: u64,
    pub semantic_count: u64,
}

impl MemoryStats {
    pub fn total(&self) -> u64 {
        self.working_count + self.episodic_count + self.semantic_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, SqliteMemoryRepository, Migrator, all_embedded_migrations};

    async fn setup_service() -> MemoryService<SqliteMemoryRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
        let repo = Arc::new(SqliteMemoryRepository::new(pool));
        MemoryService::new(repo)
    }

    #[tokio::test]
    async fn test_remember_and_recall() {
        let service = setup_service().await;

        let memory = service.remember(
            "test_key".to_string(),
            "test content".to_string(),
            "test",
        ).await.unwrap();

        assert_eq!(memory.tier, MemoryTier::Working);

        let recalled = service.recall(memory.id).await.unwrap().unwrap();
        assert_eq!(recalled.access_count, 1);
    }

    #[tokio::test]
    async fn test_learn_semantic() {
        let service = setup_service().await;

        let memory = service.learn(
            "pattern_key".to_string(),
            "learned pattern".to_string(),
            "patterns",
        ).await.unwrap();

        assert_eq!(memory.tier, MemoryTier::Semantic);
        assert!(memory.expires_at.is_none());
    }

    #[tokio::test]
    async fn test_recall_by_key() {
        let service = setup_service().await;

        service.remember(
            "lookup".to_string(),
            "value to find".to_string(),
            "test",
        ).await.unwrap();

        let found = service.recall_by_key("lookup", "test").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().content, "value to find");
    }

    #[tokio::test]
    async fn test_stats() {
        let service = setup_service().await;

        service.remember("w1".to_string(), "content".to_string(), "test").await.unwrap();
        service.remember("w2".to_string(), "content".to_string(), "test").await.unwrap();
        service.learn("s1".to_string(), "content".to_string(), "test").await.unwrap();

        let stats = service.get_stats().await.unwrap();
        assert_eq!(stats.working_count, 2);
        assert_eq!(stats.semantic_count, 1);
        assert_eq!(stats.total(), 3);
    }

    #[tokio::test]
    async fn test_promotion_on_access() {
        let service = setup_service().await
            .with_decay_config(DecayConfig {
                promote_to_episodic_threshold: 3,
                ..Default::default()
            });

        let memory = service.remember(
            "promote_me".to_string(),
            "content".to_string(),
            "test",
        ).await.unwrap();

        // Access multiple times to trigger promotion
        service.recall(memory.id).await.unwrap();
        service.recall(memory.id).await.unwrap();
        let promoted = service.recall(memory.id).await.unwrap().unwrap();

        assert_eq!(promoted.tier, MemoryTier::Episodic);
    }

    #[tokio::test]
    async fn test_forget() {
        let service = setup_service().await;

        let memory = service.remember(
            "forget_me".to_string(),
            "content".to_string(),
            "test",
        ).await.unwrap();

        service.forget(memory.id).await.unwrap();

        let recalled = service.recall(memory.id).await.unwrap();
        assert!(recalled.is_none());
    }
}

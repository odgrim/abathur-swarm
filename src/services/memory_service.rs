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

/// Represents a potential conflict between memories.
#[derive(Debug, Clone)]
pub struct MemoryConflict {
    /// First memory in the conflict.
    pub memory_a: Uuid,
    /// Second memory in the conflict.
    pub memory_b: Uuid,
    /// Key that both memories relate to.
    pub key: String,
    /// Similarity score indicating how related the memories are (0.0-1.0).
    pub similarity: f64,
    /// Whether the conflict has been automatically resolved.
    pub resolved: bool,
    /// Resolution strategy applied, if any.
    pub resolution: Option<ConflictResolution>,
}

/// Resolution strategy for memory conflicts.
#[derive(Debug, Clone)]
pub enum ConflictResolution {
    /// Kept the newer memory, deprecated the older one.
    PreferNewer { kept_id: Uuid, deprecated_id: Uuid },
    /// Kept the memory with higher confidence.
    PreferHigherConfidence { kept_id: Uuid, deprecated_id: Uuid },
    /// Merged content from both memories.
    SoftMerge { merged_id: Uuid, merged_content: String },
    /// Flagged for human review (no automatic resolution).
    FlaggedForReview,
}

/// Result of a query with conflict information.
#[derive(Debug, Clone)]
pub struct QueryResultWithConflicts {
    /// The query results.
    pub memories: Vec<Memory>,
    /// Any detected conflicts among the results.
    pub conflicts: Vec<MemoryConflict>,
}

impl<R: MemoryRepository> MemoryService<R> {
    /// Query memories and detect any conflicts among results.
    ///
    /// This method performs a standard query but additionally analyzes
    /// the returned memories for potential contradictions.
    pub async fn query_with_conflict_detection(
        &self,
        query: MemoryQuery,
    ) -> DomainResult<QueryResultWithConflicts> {
        let memories = self.repository.query(query).await?;
        let conflicts = self.detect_conflicts(&memories);

        Ok(QueryResultWithConflicts { memories, conflicts })
    }

    /// Search with conflict detection.
    pub async fn search_with_conflict_detection(
        &self,
        query: &str,
        namespace: Option<&str>,
        limit: usize,
    ) -> DomainResult<QueryResultWithConflicts> {
        let memories = self.repository.search(query, namespace, limit).await?;
        let conflicts = self.detect_conflicts(&memories);

        Ok(QueryResultWithConflicts { memories, conflicts })
    }

    /// Detect conflicts among a set of memories.
    ///
    /// Conflict detection works by:
    /// 1. Grouping memories by key (same key = potential conflict)
    /// 2. Checking if grouped memories have divergent content
    /// 3. Flagging memories with the same namespace and key but different content
    pub fn detect_conflicts(&self, memories: &[Memory]) -> Vec<MemoryConflict> {
        use std::collections::HashMap;

        let mut conflicts = Vec::new();

        // Group by (namespace, key)
        let mut grouped: HashMap<(String, String), Vec<&Memory>> = HashMap::new();
        for mem in memories {
            let key = (mem.namespace.clone(), mem.key.clone());
            grouped.entry(key).or_default().push(mem);
        }

        // Check each group for conflicts
        for ((namespace, key), group) in grouped {
            if group.len() < 2 {
                continue;
            }

            // Compare all pairs in the group
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let mem_a = group[i];
                    let mem_b = group[j];

                    // Check if content differs significantly
                    let similarity = self.compute_content_similarity(&mem_a.content, &mem_b.content);

                    // If content is different (low similarity), it's a potential conflict
                    // High similarity (>0.9) means they're essentially the same
                    if similarity < 0.9 {
                        let resolution = self.suggest_resolution(mem_a, mem_b, similarity);
                        conflicts.push(MemoryConflict {
                            memory_a: mem_a.id,
                            memory_b: mem_b.id,
                            key: format!("{}:{}", namespace, key),
                            similarity,
                            resolved: resolution.is_some(),
                            resolution,
                        });
                    }
                }
            }
        }

        conflicts
    }

    /// Compute similarity between two pieces of content.
    /// Returns a value between 0.0 (completely different) and 1.0 (identical).
    fn compute_content_similarity(&self, content_a: &str, content_b: &str) -> f64 {
        if content_a == content_b {
            return 1.0;
        }

        // Simple word-overlap based similarity (Jaccard coefficient)
        let lowercase_a = content_a.to_lowercase();
        let lowercase_b = content_b.to_lowercase();
        let words_a: std::collections::HashSet<&str> =
            lowercase_a.split_whitespace().collect();
        let words_b: std::collections::HashSet<&str> =
            lowercase_b.split_whitespace().collect();

        if words_a.is_empty() && words_b.is_empty() {
            return 1.0;
        }

        let intersection = words_a.intersection(&words_b).count() as f64;
        let union = words_a.union(&words_b).count() as f64;

        if union == 0.0 {
            return 1.0;
        }

        intersection / union
    }

    /// Suggest a resolution strategy for a conflict.
    fn suggest_resolution(
        &self,
        mem_a: &Memory,
        mem_b: &Memory,
        similarity: f64,
    ) -> Option<ConflictResolution> {
        // If very low similarity, needs human review
        if similarity < 0.3 {
            return Some(ConflictResolution::FlaggedForReview);
        }

        // Prefer higher tier memory (semantic > episodic > working)
        let tier_order = |tier: &MemoryTier| match tier {
            MemoryTier::Semantic => 3,
            MemoryTier::Episodic => 2,
            MemoryTier::Working => 1,
        };

        if tier_order(&mem_a.tier) != tier_order(&mem_b.tier) {
            let (kept, deprecated) = if tier_order(&mem_a.tier) > tier_order(&mem_b.tier) {
                (mem_a.id, mem_b.id)
            } else {
                (mem_b.id, mem_a.id)
            };
            return Some(ConflictResolution::PreferHigherConfidence {
                kept_id: kept,
                deprecated_id: deprecated,
            });
        }

        // Same tier - prefer newer memory
        let (newer, older) = if mem_a.created_at > mem_b.created_at {
            (mem_a.id, mem_b.id)
        } else {
            (mem_b.id, mem_a.id)
        };

        Some(ConflictResolution::PreferNewer {
            kept_id: newer,
            deprecated_id: older,
        })
    }

    /// Apply a conflict resolution.
    pub async fn resolve_conflict(
        &self,
        conflict: &MemoryConflict,
    ) -> DomainResult<()> {
        match &conflict.resolution {
            Some(ConflictResolution::PreferNewer { deprecated_id, .. })
            | Some(ConflictResolution::PreferHigherConfidence { deprecated_id, .. }) => {
                // Mark the deprecated memory as superseded (we could delete or just flag)
                if let Some(mut deprecated) = self.repository.get(*deprecated_id).await? {
                    // Add superseded flag to metadata
                    deprecated.metadata.tags.push("superseded".to_string());
                    self.repository.update(&deprecated).await?;
                }
            }
            Some(ConflictResolution::SoftMerge { merged_id, merged_content }) => {
                // Update the merged memory with combined content
                if let Some(mut merged) = self.repository.get(*merged_id).await? {
                    merged.content = merged_content.clone();
                    merged.metadata.tags.push("merged".to_string());
                    self.repository.update(&merged).await?;
                }

                // Mark the other memory as merged-into
                let other_id = if conflict.memory_a == *merged_id {
                    conflict.memory_b
                } else {
                    conflict.memory_a
                };
                if let Some(mut other) = self.repository.get(other_id).await? {
                    other.metadata.tags.push("merged-into".to_string());
                    other.metadata.tags.push(format!("merged-into:{}", merged_id));
                    self.repository.update(&other).await?;
                }
            }
            Some(ConflictResolution::FlaggedForReview) | None => {
                // Just mark both memories as needing review
                for id in [conflict.memory_a, conflict.memory_b] {
                    if let Some(mut mem) = self.repository.get(id).await? {
                        if !mem.metadata.tags.contains(&"needs-review".to_string()) {
                            mem.metadata.tags.push("needs-review".to_string());
                            self.repository.update(&mem).await?;
                        }
                    }
                }
            }
        }

        Ok(())
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

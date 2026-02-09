//! Memory service implementing business logic with decay management.

use std::sync::Arc;
use uuid::Uuid;

use async_trait::async_trait;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    Memory, MemoryMetadata, MemoryQuery, MemoryTier, MemoryType,
    RelevanceWeights, ScoredMemory,
};
use crate::domain::ports::MemoryRepository;
use crate::services::command_bus::{CommandError, CommandResult, MemoryCommand, MemoryCommandHandler};
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};

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
    event_bus: Arc<EventBus>,
}

impl<R: MemoryRepository> MemoryService<R> {
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            decay_config: DecayConfig::default(),
            event_bus: Arc::new(EventBus::new(crate::services::event_bus::EventBusConfig::default())),
        }
    }

    pub fn new_with_event_bus(repository: Arc<R>, event_bus: Arc<EventBus>) -> Self {
        Self {
            repository,
            decay_config: DecayConfig::default(),
            event_bus,
        }
    }

    pub fn with_decay_config(mut self, config: DecayConfig) -> Self {
        self.decay_config = config;
        self
    }

    async fn emit(&self, event: UnifiedEvent) {
        self.event_bus.publish(event).await;
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

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Memory,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::MemoryStored {
                memory_id: memory.id,
                key: memory.key.clone(),
                namespace: memory.namespace.clone(),
                tier: memory.tier.as_str().to_string(),
                memory_type: memory.memory_type.as_str().to_string(),
            },
        }).await;

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

            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Debug,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::MemoryAccessed {
                    memory_id: mem.id,
                    key: mem.key.clone(),
                    access_count: mem.access_count,
                },
            }).await;

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

            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Debug,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::MemoryAccessed {
                    memory_id: mem.id,
                    key: mem.key.clone(),
                    access_count: mem.access_count,
                },
            }).await;

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

    /// Ranked search: search memories and return results scored by multi-factor relevance.
    ///
    /// Implements the research-recommended approach from DynTaskMAS (ICAPS 2025):
    ///   score = w_semantic * text_match + w_decay * recency + w_importance * access_pattern
    ///
    /// Results are sorted by composite score (highest first) and filtered by minimum threshold.
    pub async fn ranked_search(
        &self,
        query: &str,
        namespace: Option<&str>,
        weights: RelevanceWeights,
        limit: usize,
        min_score: f32,
    ) -> DomainResult<Vec<ScoredMemory>> {
        // First, get candidate memories via full-text search
        // We fetch more than needed since scoring will re-rank them
        let fetch_limit = (limit * 3).max(50);
        let candidates = self.repository.search(query, namespace, fetch_limit).await?;

        // Score each candidate using multi-factor relevance
        let mut scored: Vec<ScoredMemory> = candidates
            .into_iter()
            .map(|mem| mem.relevance_score(query, &weights))
            .filter(|scored| scored.score >= min_score)
            .collect();

        // Sort by composite score (highest first)
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Truncate to requested limit
        scored.truncate(limit);

        Ok(scored)
    }

    /// Load context for a task with token budget management.
    ///
    /// Inspired by Manus AI's context engineering approach:
    /// - Select relevant memories using multi-factor scoring
    /// - Fit within a token budget to avoid context window overflow
    /// - Prioritize high-relevance memories over low-relevance ones
    ///
    /// Returns memories that fit within the token budget, sorted by relevance.
    pub async fn load_context_with_budget(
        &self,
        query: &str,
        namespace: Option<&str>,
        token_budget: usize,
        weights: RelevanceWeights,
    ) -> DomainResult<Vec<ScoredMemory>> {
        // Get scored candidates
        let scored = self.ranked_search(query, namespace, weights, 100, 0.1).await?;

        // Greedily fill the token budget with highest-scored memories
        let mut selected = Vec::new();
        let mut tokens_used = 0;

        for entry in scored {
            let entry_tokens = entry.memory.estimated_tokens();
            if tokens_used + entry_tokens <= token_budget {
                tokens_used += entry_tokens;
                selected.push(entry);
            }
            // Don't break early - later entries might be smaller and still fit
        }

        Ok(selected)
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
        // Fetch memory info before deleting for the event
        let memory = self.repository.get(id).await?;
        self.repository.delete(id).await?;

        if let Some(mem) = memory {
            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Debug,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::MemoryDeleted {
                    memory_id: id,
                    key: mem.key,
                    namespace: mem.namespace,
                },
            }).await;
        }

        Ok(())
    }

    /// Prune expired memories.
    pub async fn prune_expired(&self) -> DomainResult<u64> {
        let count = self.repository.prune_expired().await?;
        if count > 0 {
            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Debug,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::MemoryPruned {
                    count,
                    reason: "expired".to_string(),
                },
            }).await;
        }
        Ok(count)
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

    /// Run full maintenance: prune expired and decayed, resolve conflicts.
    pub async fn run_maintenance(&self) -> DomainResult<MaintenanceReport> {
        let expired = self.prune_expired().await?;
        let decayed = self.prune_decayed().await?;

        // Check for promotion candidates
        let promoted = self.check_all_promotions().await?;

        // Detect and auto-resolve conflicts
        let conflicts_resolved = self.auto_resolve_conflicts().await?;

        Ok(MaintenanceReport {
            expired_pruned: expired,
            decayed_pruned: decayed,
            promoted,
            conflicts_resolved,
        })
    }

    /// Automatically detect and resolve memory conflicts.
    ///
    /// This method scans all memories for conflicts and applies automatic
    /// resolution strategies (soft merge, prefer newer/higher confidence).
    /// Conflicts that cannot be automatically resolved are flagged for review.
    pub async fn auto_resolve_conflicts(&self) -> DomainResult<u64> {
        let mut resolved_count = 0;

        // Get all namespaces by querying distinct values
        // For efficiency, we'll scan working and episodic tiers (semantic is long-term stable)
        let working_memories = self.repository.list_by_tier(MemoryTier::Working).await?;
        let episodic_memories = self.repository.list_by_tier(MemoryTier::Episodic).await?;

        let all_memories: Vec<Memory> = working_memories
            .into_iter()
            .chain(episodic_memories.into_iter())
            .collect();

        // Detect conflicts
        let conflicts = self.detect_conflicts(&all_memories);

        // Resolve each conflict that has an automatic resolution
        for conflict in conflicts {
            // Emit conflict detection event
            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Warning,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::MemoryConflictDetected {
                    memory_a: conflict.memory_a,
                    memory_b: conflict.memory_b,
                    key: conflict.key.clone(),
                    similarity: conflict.similarity,
                },
            }).await;

            if matches!(
                &conflict.resolution,
                Some(ConflictResolution::PreferNewer { .. })
                    | Some(ConflictResolution::PreferHigherConfidence { .. })
                    | Some(ConflictResolution::SoftMerge { .. })
            ) {
                if self.resolve_conflict(&conflict).await.is_ok() {
                    resolved_count += 1;
                }
            } else if matches!(&conflict.resolution, Some(ConflictResolution::FlaggedForReview)) {
                // Just flag these for review, count as "processed"
                if self.resolve_conflict(&conflict).await.is_ok() {
                    // Don't count flagged as "resolved", but still process them
                }
            }
        }

        Ok(resolved_count)
    }

    /// Get all memories flagged for review due to unresolved conflicts.
    pub async fn get_memories_needing_review(&self) -> DomainResult<Vec<Memory>> {
        let query = MemoryQuery {
            tags: vec!["needs-review".to_string()],
            ..Default::default()
        };
        self.repository.query(query).await
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
            let from_tier = memory.tier.as_str().to_string();
            memory.promote().map_err(DomainError::ValidationFailed)?;
            let to_tier = memory.tier.as_str().to_string();
            self.repository.update(memory).await?;

            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::MemoryPromoted {
                    memory_id: memory.id,
                    key: memory.key.clone(),
                    from_tier,
                    to_tier,
                },
            }).await;

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
    pub conflicts_resolved: u64,
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

        // Medium similarity (0.3-0.7) with same tier: try semantic merge
        // This combines information from both memories when they're complementary
        if similarity >= 0.3 && similarity < 0.7 {
            let merged_content = self.create_merged_content(mem_a, mem_b);
            // Use the newer memory as the base to merge into
            let merged_id = if mem_a.created_at > mem_b.created_at {
                mem_a.id
            } else {
                mem_b.id
            };
            return Some(ConflictResolution::SoftMerge {
                merged_id,
                merged_content,
            });
        }

        // High similarity (0.7-0.9) with same tier - prefer newer memory
        // (above 0.9 we don't report conflicts at all)
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

    /// Create merged content from two memories using semantic synthesis.
    ///
    /// This method attempts to combine information from both memories,
    /// preserving unique information from each while avoiding duplication.
    fn create_merged_content(&self, mem_a: &Memory, mem_b: &Memory) -> String {
        // Determine which memory is newer (will be the base)
        let (newer, older) = if mem_a.created_at > mem_b.created_at {
            (mem_a, mem_b)
        } else {
            (mem_b, mem_a)
        };

        // Extract sentences/paragraphs from each
        let newer_parts: Vec<&str> = newer.content
            .split(|c| c == '.' || c == '\n')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let older_parts: Vec<&str> = older.content
            .split(|c| c == '.' || c == '\n')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        // Find unique content from older memory not substantially covered in newer
        let mut unique_from_older: Vec<&str> = Vec::new();
        for old_part in &older_parts {
            let is_covered = newer_parts.iter().any(|new_part| {
                self.compute_content_similarity(old_part, new_part) > 0.6
            });
            if !is_covered && !old_part.is_empty() {
                unique_from_older.push(old_part);
            }
        }

        // Build merged content: newer content + unique older content
        let mut merged = newer.content.clone();

        if !unique_from_older.is_empty() {
            merged.push_str("\n\n[Additional context from previous memory:]\n");
            for part in unique_from_older {
                merged.push_str(part);
                if !part.ends_with('.') && !part.ends_with('!') && !part.ends_with('?') {
                    merged.push('.');
                }
                merged.push(' ');
            }
        }

        merged.trim().to_string()
    }

    /// Apply a conflict resolution.
    pub async fn resolve_conflict(
        &self,
        conflict: &MemoryConflict,
    ) -> DomainResult<()> {
        let resolution_type = match &conflict.resolution {
            Some(ConflictResolution::PreferNewer { .. }) => "prefer_newer",
            Some(ConflictResolution::PreferHigherConfidence { .. }) => "prefer_higher_confidence",
            Some(ConflictResolution::SoftMerge { .. }) => "soft_merge",
            Some(ConflictResolution::FlaggedForReview) => "flagged_for_review",
            None => "none",
        };

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

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Memory,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::MemoryConflictResolved {
                memory_a: conflict.memory_a,
                memory_b: conflict.memory_b,
                resolution_type: resolution_type.to_string(),
            },
        }).await;

        Ok(())
    }
}

#[async_trait]
impl<R: MemoryRepository + 'static> MemoryCommandHandler for MemoryService<R> {
    async fn handle(&self, cmd: MemoryCommand) -> Result<CommandResult, CommandError> {
        match cmd {
            MemoryCommand::Store {
                key,
                content,
                namespace,
                tier,
                memory_type,
                metadata,
            } => {
                let memory = self
                    .store(key, content, namespace, tier, memory_type, metadata)
                    .await?;
                Ok(CommandResult::Memory(memory))
            }
            MemoryCommand::Recall { id } => {
                let memory = self.recall(id).await?;
                Ok(CommandResult::MemoryOpt(memory))
            }
            MemoryCommand::RecallByKey { key, namespace } => {
                let memory = self.recall_by_key(&key, &namespace).await?;
                Ok(CommandResult::MemoryOpt(memory))
            }
            MemoryCommand::Forget { id } => {
                self.forget(id).await?;
                Ok(CommandResult::Unit)
            }
            MemoryCommand::PruneExpired => {
                let count = self.prune_expired().await?;
                Ok(CommandResult::PruneCount(count))
            }
            MemoryCommand::RunMaintenance => {
                let report = self.run_maintenance().await?;
                Ok(CommandResult::MaintenanceReport(report))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_migrated_test_pool, SqliteMemoryRepository};

    async fn setup_service() -> MemoryService<SqliteMemoryRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
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
    async fn test_ranked_search() {
        let service = setup_service().await;

        // Store some memories with different content
        service.store(
            "rust_patterns".to_string(),
            "Rust programming patterns include iterators closures and traits".to_string(),
            "code".to_string(),
            MemoryTier::Semantic,
            MemoryType::Pattern,
            None,
        ).await.unwrap();

        service.store(
            "python_basics".to_string(),
            "Python is a dynamic language with list comprehensions".to_string(),
            "code".to_string(),
            MemoryTier::Working,
            MemoryType::Fact,
            None,
        ).await.unwrap();

        service.store(
            "rust_errors".to_string(),
            "Error handling in Rust uses Result and Option types for safety".to_string(),
            "code".to_string(),
            MemoryTier::Episodic,
            MemoryType::Pattern,
            None,
        ).await.unwrap();

        // Search for "Rust" - should rank Rust-related memories higher
        let results = service.ranked_search(
            "Rust patterns",
            Some("code"),
            RelevanceWeights::semantic_biased(),
            10,
            0.0,
        ).await.unwrap();

        assert!(!results.is_empty(), "Should find some results");

        // Verify results are sorted by score (descending)
        for i in 1..results.len() {
            assert!(results[i - 1].score >= results[i].score,
                "Results should be sorted by score: {} >= {}",
                results[i - 1].score, results[i].score);
        }
    }

    #[tokio::test]
    async fn test_load_context_with_budget() {
        let service = setup_service().await;

        // Store memories with varying sizes
        service.store(
            "short".to_string(),
            "Short memory.".to_string(),
            "test".to_string(),
            MemoryTier::Working,
            MemoryType::Fact,
            None,
        ).await.unwrap();

        service.store(
            "medium".to_string(),
            "This is a medium-length memory entry that contains some useful information about the project architecture and design decisions that were made.".to_string(),
            "test".to_string(),
            MemoryTier::Episodic,
            MemoryType::Decision,
            None,
        ).await.unwrap();

        // Load with a tight budget - should only include what fits
        let results = service.load_context_with_budget(
            "memory project",
            Some("test"),
            50, // ~50 tokens budget
            RelevanceWeights::default(),
        ).await.unwrap();

        // Should have results but limited by budget
        let total_tokens: usize = results.iter()
            .map(|r| r.memory.estimated_tokens())
            .sum();
        assert!(total_tokens <= 50, "Total tokens {} should be within budget of 50", total_tokens);
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

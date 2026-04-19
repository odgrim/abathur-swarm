//! Memory decay service: prunes expired/decayed memories and resolves conflicts.
//!
//! Split out of [`MemoryService`] to keep the core CRUD service focused on
//! store/recall/search. The decay service owns the low-level maintenance
//! operations that reshape the memory store over time:
//!
//! - [`MemoryDecayService::prune_expired`] — drop memories past their TTL
//! - [`MemoryDecayService::prune_decayed`] — drop memories below tier-specific
//!   decay thresholds
//! - [`MemoryDecayService::auto_resolve_conflicts`] — detect duplicate/contradicting
//!   memories and apply automatic resolution strategies
//!
//! Conflict *detection* and *resolution* primitives (`detect_conflicts`,
//! `resolve_conflict`, etc.) continue to live on [`MemoryService`] because
//! they are also consumed by query-side helpers. The decay service holds an
//! `Arc<MemoryService<R>>` and delegates to those.

use std::sync::Arc;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Memory, MemoryTier};
use crate::domain::ports::MemoryRepository;
use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity, UnifiedEvent};
use crate::services::event_factory;
use crate::services::memory_service::{ConflictResolution, MemoryService};

/// Decay and conflict-resolution operations for the memory subsystem.
///
/// This service is cheap to clone (it holds `Arc`s). Construct it once per
/// wiring site alongside [`MemoryService`] and share it across call sites
/// that need decay/pruning/conflict-resolution behavior.
#[derive(Clone)]
pub struct MemoryDecayService<R: MemoryRepository> {
    memory_service: Arc<MemoryService<R>>,
}

impl<R: MemoryRepository> MemoryDecayService<R> {
    /// Create a new decay service backed by the given memory service.
    pub fn new(memory_service: Arc<MemoryService<R>>) -> Self {
        Self { memory_service }
    }

    /// Access the wrapped memory service (e.g. for conflict-detection helpers).
    pub fn memory_service(&self) -> &Arc<MemoryService<R>> {
        &self.memory_service
    }

    fn repository(&self) -> &Arc<R> {
        self.memory_service.repository()
    }

    fn make_event(
        severity: EventSeverity,
        category: EventCategory,
        payload: EventPayload,
    ) -> UnifiedEvent {
        event_factory::make_event(severity, category, None, None, payload)
    }

    /// Prune expired memories. Returns the count and events to be journaled.
    pub async fn prune_expired(&self) -> DomainResult<(u64, Vec<UnifiedEvent>)> {
        let count = self.repository().prune_expired().await?;
        let mut events = Vec::new();
        if count > 0 {
            events.push(Self::make_event(
                EventSeverity::Debug,
                EventCategory::Memory,
                EventPayload::MemoryPruned {
                    count,
                    reason: "expired".to_string(),
                },
            ));
        }
        Ok((count, events))
    }

    /// Prune decayed memories (below threshold). Returns count and events.
    pub async fn prune_decayed(&self) -> DomainResult<(u64, Vec<UnifiedEvent>)> {
        let mut count = 0;
        let mut events = Vec::new();

        let decay_config = self.memory_service.decay_config();

        // Prune working memories
        let decayed = self
            .repository()
            .get_decayed(decay_config.working_prune_threshold)
            .await?;
        for mem in decayed {
            if mem.tier == MemoryTier::Working {
                self.repository().delete(mem.id).await?;
                count += 1;
            }
        }

        // Prune episodic memories
        let decayed = self
            .repository()
            .get_decayed(decay_config.episodic_prune_threshold)
            .await?;
        for mem in decayed {
            if mem.tier == MemoryTier::Episodic {
                self.repository().delete(mem.id).await?;
                count += 1;
            }
        }

        if count > 0 {
            events.push(Self::make_event(
                EventSeverity::Debug,
                EventCategory::Memory,
                EventPayload::MemoryPruned {
                    count,
                    reason: "decayed".to_string(),
                },
            ));
        }

        Ok((count, events))
    }

    /// Automatically detect and resolve memory conflicts.
    ///
    /// This method scans all memories for conflicts and applies automatic
    /// resolution strategies (soft merge, prefer newer/higher confidence).
    /// Conflicts that cannot be automatically resolved are flagged for review.
    /// Returns the count and all accumulated events.
    pub async fn auto_resolve_conflicts(&self) -> DomainResult<(u64, Vec<UnifiedEvent>)> {
        let mut resolved_count = 0;
        let mut all_events = Vec::new();

        // Get all namespaces by querying distinct values
        // For efficiency, we'll scan working and episodic tiers (semantic is long-term stable)
        let working_memories = self.repository().list_by_tier(MemoryTier::Working).await?;
        let episodic_memories = self.repository().list_by_tier(MemoryTier::Episodic).await?;

        let all_memories: Vec<Memory> = working_memories
            .into_iter()
            .chain(episodic_memories.into_iter())
            .collect();

        // Detect conflicts (logic lives on MemoryService so query-side helpers can reuse it).
        let conflicts = self.memory_service.detect_conflicts(&all_memories);

        // Resolve each conflict that has an automatic resolution
        for conflict in conflicts {
            // Collect conflict detection event
            all_events.push(Self::make_event(
                EventSeverity::Warning,
                EventCategory::Memory,
                EventPayload::MemoryConflictDetected {
                    memory_a: conflict.memory_a,
                    memory_b: conflict.memory_b,
                    key: conflict.key.clone(),
                    similarity: conflict.similarity,
                },
            ));

            if matches!(
                &conflict.resolution,
                Some(ConflictResolution::PreferNewer { .. })
                    | Some(ConflictResolution::PreferHigherConfidence { .. })
                    | Some(ConflictResolution::SoftMerge { .. })
            ) {
                if let Ok(events) = self.memory_service.resolve_conflict(&conflict).await {
                    all_events.extend(events);
                    resolved_count += 1;
                }
            } else if matches!(&conflict.resolution, Some(ConflictResolution::FlaggedForReview)) {
                // Just flag these for review, count as "processed"
                if let Ok(events) = self.memory_service.resolve_conflict(&conflict).await {
                    all_events.extend(events);
                    // Don't count flagged as "resolved", but still process them
                }
            }
        }

        Ok((resolved_count, all_events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::test_support;
    use crate::adapters::sqlite::SqliteMemoryRepository;
    use crate::domain::models::{MemoryTier, MemoryType};

    async fn setup() -> (
        Arc<MemoryService<SqliteMemoryRepository>>,
        MemoryDecayService<SqliteMemoryRepository>,
    ) {
        let memory_service = Arc::new(test_support::setup_memory_service().await);
        let decay_service = MemoryDecayService::new(memory_service.clone());
        (memory_service, decay_service)
    }

    #[tokio::test]
    async fn test_auto_resolve_conflicts_end_to_end() {
        let (service, decay) = setup().await;

        // Create two Working-tier memories with same key and ~80% overlap (PreferNewer)
        let (older, _) = service
            .store(
                "e2e_key".to_string(),
                "the quick brown fox jumps over the lazy dog".to_string(),
                "ns".to_string(),
                MemoryTier::Working,
                MemoryType::Fact,
                None,
            )
            .await
            .unwrap();

        let (_newer, _) = service
            .store(
                "e2e_key".to_string(),
                "the quick brown fox jumps over the lazy cat today".to_string(),
                "ns".to_string(),
                MemoryTier::Working,
                MemoryType::Fact,
                None,
            )
            .await
            .unwrap();

        // Run auto-resolve
        let (resolved_count, events) = decay.auto_resolve_conflicts().await.unwrap();

        assert!(resolved_count >= 1, "Should resolve at least 1 conflict");
        assert!(
            !events.is_empty(),
            "Should emit events for conflict detection and resolution"
        );

        // The older memory should now be tagged as "superseded"
        let deprecated = service.repository().get(older.id).await.unwrap().unwrap();
        assert!(
            deprecated.metadata.tags.contains(&"superseded".to_string()),
            "Deprecated memory should be tagged 'superseded', got tags: {:?}",
            deprecated.metadata.tags
        );
    }

    #[tokio::test]
    async fn test_prune_expired_no_expired_returns_zero() {
        let (_service, decay) = setup().await;
        let (count, events) = decay.prune_expired().await.unwrap();
        assert_eq!(count, 0);
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn test_prune_decayed_no_decayed_returns_zero() {
        let (_service, decay) = setup().await;
        let (count, events) = decay.prune_decayed().await.unwrap();
        assert_eq!(count, 0);
        assert!(events.is_empty());
    }
}

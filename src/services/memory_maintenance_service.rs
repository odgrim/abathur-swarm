//! Memory maintenance orchestration.
//!
//! Wraps [`MemoryService`] and [`MemoryDecayService`] and exposes the
//! top-level `run_maintenance` orchestration that ties expiration,
//! decay-pruning, promotion, and conflict resolution together.
//!
//! The memory decay daemon holds an `Arc<MemoryMaintenanceService>` and
//! invokes [`MemoryMaintenanceService::run_maintenance`] on its scheduled
//! cadence.

use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Memory, MemoryQuery};
use crate::domain::ports::MemoryRepository;
use crate::services::command_bus::{
    CommandError, CommandOutcome, CommandResult, MemoryCommand, MemoryCommandHandler,
};
use crate::services::event_bus::UnifiedEvent;
use crate::services::memory_decay_service::MemoryDecayService;
use crate::services::memory_service::{MaintenanceReport, MemoryService};

/// Orchestrates full memory maintenance by delegating to the underlying
/// CRUD service (promotions, review lookup) and decay service
/// (expiration, pruning, conflict resolution).
#[derive(Clone)]
pub struct MemoryMaintenanceService<R: MemoryRepository> {
    memory_service: Arc<MemoryService<R>>,
    decay_service: Arc<MemoryDecayService<R>>,
}

impl<R: MemoryRepository> MemoryMaintenanceService<R> {
    /// Create a new maintenance service from a memory service and decay service.
    ///
    /// Both services are expected to share the same underlying repository.
    pub fn new(
        memory_service: Arc<MemoryService<R>>,
        decay_service: Arc<MemoryDecayService<R>>,
    ) -> Self {
        Self {
            memory_service,
            decay_service,
        }
    }

    /// Convenience: build a maintenance service from just a memory service.
    /// The decay service is created fresh and wrapped in an `Arc`.
    pub fn from_memory_service(memory_service: Arc<MemoryService<R>>) -> Self {
        let decay_service = Arc::new(MemoryDecayService::new(memory_service.clone()));
        Self::new(memory_service, decay_service)
    }

    /// Access the underlying memory service.
    pub fn memory_service(&self) -> &Arc<MemoryService<R>> {
        &self.memory_service
    }

    /// Access the underlying decay service.
    pub fn decay_service(&self) -> &Arc<MemoryDecayService<R>> {
        &self.decay_service
    }

    /// Run full maintenance: prune expired and decayed, promote candidates,
    /// and auto-resolve conflicts. Returns the report and all accumulated
    /// events.
    pub async fn run_maintenance(&self) -> DomainResult<(MaintenanceReport, Vec<UnifiedEvent>)> {
        let mut all_events = Vec::new();

        let (expired, events) = self.decay_service.prune_expired().await?;
        all_events.extend(events);

        let (decayed, events) = self.decay_service.prune_decayed().await?;
        all_events.extend(events);

        // Check for promotion candidates
        let (promoted, events) = self.memory_service.check_all_promotions().await?;
        all_events.extend(events);

        // Detect and auto-resolve conflicts
        let (conflicts_resolved, events) = self.decay_service.auto_resolve_conflicts().await?;
        all_events.extend(events);

        Ok((
            MaintenanceReport {
                expired_pruned: expired,
                decayed_pruned: decayed,
                promoted,
                conflicts_resolved,
            },
            all_events,
        ))
    }

    /// Get all memories flagged for review due to unresolved conflicts.
    pub async fn get_memories_needing_review(&self) -> DomainResult<Vec<Memory>> {
        let query = MemoryQuery {
            tags: vec!["needs-review".to_string()],
            ..Default::default()
        };
        self.memory_service.repository().query(query).await
    }
}

/// `MemoryCommandHandler` implementation that dispatches CRUD commands to
/// [`MemoryService`] and maintenance commands (`PruneExpired`, `RunMaintenance`)
/// to the decay/maintenance services.
///
/// Wiring sites should register this as the bus's memory handler instead of
/// `Arc<MemoryService>` directly, so the full command surface is honored.
#[async_trait]
impl<R: MemoryRepository + 'static> MemoryCommandHandler for MemoryMaintenanceService<R> {
    async fn handle(&self, cmd: MemoryCommand) -> Result<CommandOutcome, CommandError> {
        match cmd {
            MemoryCommand::Store {
                key,
                content,
                namespace,
                tier,
                memory_type,
                metadata,
            } => {
                let (memory, events) = self
                    .memory_service
                    .store(key, content, namespace, tier, memory_type, metadata)
                    .await?;
                Ok(CommandOutcome {
                    result: CommandResult::Memory(memory),
                    events,
                })
            }
            MemoryCommand::Recall { id, accessor } => {
                let (memory, events) = self.memory_service.recall(id, accessor).await?;
                Ok(CommandOutcome {
                    result: CommandResult::MemoryOpt(memory),
                    events,
                })
            }
            MemoryCommand::RecallByKey {
                key,
                namespace,
                accessor,
            } => {
                let (memory, events) = self
                    .memory_service
                    .recall_by_key(&key, &namespace, accessor)
                    .await?;
                Ok(CommandOutcome {
                    result: CommandResult::MemoryOpt(memory),
                    events,
                })
            }
            MemoryCommand::Update {
                id,
                content,
                namespace,
                tier,
            } => {
                let (memory, events) = self
                    .memory_service
                    .update_memory(id, content, namespace, tier)
                    .await?;
                Ok(CommandOutcome {
                    result: CommandResult::Memory(memory),
                    events,
                })
            }
            MemoryCommand::Forget { id } => {
                let events = self.memory_service.forget(id).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Unit,
                    events,
                })
            }
            MemoryCommand::PruneExpired => {
                let (count, events) = self.decay_service.prune_expired().await?;
                Ok(CommandOutcome {
                    result: CommandResult::PruneCount(count),
                    events,
                })
            }
            MemoryCommand::RunMaintenance => {
                let (report, events) = self.run_maintenance().await?;
                Ok(CommandOutcome {
                    result: CommandResult::MaintenanceReport(report),
                    events,
                })
            }
        }
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
        MemoryMaintenanceService<SqliteMemoryRepository>,
    ) {
        let memory_service = Arc::new(test_support::setup_memory_service().await);
        let maintenance = MemoryMaintenanceService::from_memory_service(memory_service.clone());
        (memory_service, maintenance)
    }

    #[tokio::test]
    async fn test_run_maintenance_on_empty_store_succeeds() {
        let (_service, maintenance) = setup().await;
        let (report, _events) = maintenance.run_maintenance().await.unwrap();
        assert_eq!(report.expired_pruned, 0);
        assert_eq!(report.decayed_pruned, 0);
        assert_eq!(report.promoted, 0);
        assert_eq!(report.conflicts_resolved, 0);
    }

    #[tokio::test]
    async fn test_run_maintenance_resolves_conflicts() {
        let (service, maintenance) = setup().await;

        // Seed a PreferNewer-style conflict (two working memories, same key, ~80% overlap)
        service
            .store(
                "maint_key".to_string(),
                "the quick brown fox jumps over the lazy dog".to_string(),
                "ns".to_string(),
                MemoryTier::Working,
                MemoryType::Fact,
                None,
            )
            .await
            .unwrap();
        service
            .store(
                "maint_key".to_string(),
                "the quick brown fox jumps over the lazy cat today".to_string(),
                "ns".to_string(),
                MemoryTier::Working,
                MemoryType::Fact,
                None,
            )
            .await
            .unwrap();

        let (report, _events) = maintenance.run_maintenance().await.unwrap();
        assert!(
            report.conflicts_resolved >= 1,
            "Expected at least one conflict resolved, got report: {:?}",
            report
        );
    }

    #[tokio::test]
    async fn test_get_memories_needing_review_filters_by_tag() {
        let (service, maintenance) = setup().await;

        // Low-similarity conflict triggers FlaggedForReview, which tags both memories with "needs-review".
        service
            .store(
                "review_key".to_string(),
                "alpha bravo charlie delta echo foxtrot golf".to_string(),
                "ns".to_string(),
                MemoryTier::Working,
                MemoryType::Fact,
                None,
            )
            .await
            .unwrap();
        service
            .store(
                "review_key".to_string(),
                "one two three four five six seven eight nine ten".to_string(),
                "ns".to_string(),
                MemoryTier::Working,
                MemoryType::Fact,
                None,
            )
            .await
            .unwrap();

        // Run maintenance so FlaggedForReview resolution tags the memories.
        maintenance.run_maintenance().await.unwrap();

        let flagged = maintenance.get_memories_needing_review().await.unwrap();
        assert!(
            !flagged.is_empty(),
            "Expected at least one memory flagged for review"
        );
        for mem in &flagged {
            assert!(
                mem.metadata.tags.contains(&"needs-review".to_string()),
                "Returned memory should be tagged 'needs-review', got tags: {:?}",
                mem.metadata.tags
            );
        }
    }
}

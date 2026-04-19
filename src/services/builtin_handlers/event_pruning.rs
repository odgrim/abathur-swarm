//! Built-in reactive event handler.
//!
//! All handlers are **idempotent** — safe to run even if the poll loop already
//! handled the same state change. They check current state before acting.

#![allow(unused_imports)]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::domain::errors::DomainError;
use crate::domain::models::adapter::IngestionItemKind;
use crate::domain::models::convergence::{AmendmentSource, SpecificationAmendment};
use crate::domain::models::task_schedule::*;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{Goal, HumanEscalationEvent, Task, TaskSource, TaskStatus};
use crate::domain::ports::{
    GoalRepository, MemoryRepository, TaskRepository, TaskScheduleRepository, TrajectoryRepository,
    WorktreeRepository,
};
#[cfg(test)]
use crate::services::event_bus::ConvergenceTerminatedPayload;
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, HumanEscalationPayload,
    SequenceNumber, SwarmStatsPayload, TaskResultPayload, UnifiedEvent,
};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};
use crate::services::event_store::EventStore;
use crate::services::goal_context_service::GoalContextService;
use crate::services::memory_service::MemoryService;
use crate::services::swarm_orchestrator::SwarmStats;
use crate::services::task_service::TaskService;

use super::{try_update_task, update_with_retry};

// ============================================================================
// EventPruningHandler
// ============================================================================

/// Triggered by the "event-pruning" scheduled event.
/// Calls `event_store.prune_older_than()` to remove old events based on
/// the configured retention duration.
pub struct EventPruningHandler {
    event_store: Arc<dyn EventStore>,
    /// Retention duration in days.
    retention_days: u64,
}

impl EventPruningHandler {
    pub fn new(event_store: Arc<dyn EventStore>, retention_days: u64) -> Self {
        Self {
            event_store,
            retention_days,
        }
    }
}

#[async_trait]
impl EventHandler for EventPruningHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EventPruningHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "event-pruning"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let retention = std::time::Duration::from_secs(self.retention_days * 24 * 3600);

        let pruned = self
            .event_store
            .prune_older_than(retention)
            .await
            .map_err(|e| format!("EventPruning: failed to prune events: {}", e))?;

        if pruned > 0 {
            tracing::info!(
                "EventPruning: pruned {} events older than {} days",
                pruned,
                self.retention_days
            );

            let summary = UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Orchestrator,
                goal_id: None,
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ReconciliationCompleted {
                    corrections_made: pruned as u32,
                },
            };

            Ok(Reaction::EmitEvents(vec![summary]))
        } else {
            Ok(Reaction::None)
        }
    }
}

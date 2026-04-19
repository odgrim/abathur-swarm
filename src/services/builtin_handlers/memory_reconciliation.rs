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
use crate::services::memory_maintenance_service::MemoryMaintenanceService;
use crate::services::swarm_orchestrator::SwarmStats;
use crate::services::task_service::TaskService;

use super::{try_update_task, update_with_retry};

// ============================================================================
// MemoryReconciliationHandler
// ============================================================================

/// Periodic safety-net for memory subsystem: prunes expired/decayed memories,
/// promotes candidates, and detects orphaned memories.
///
/// Triggered by `ScheduledEventFired { name: "memory-reconciliation" }`.
pub struct MemoryReconciliationHandler<M: MemoryRepository> {
    maintenance_service: Arc<MemoryMaintenanceService<M>>,
}

impl<M: MemoryRepository> MemoryReconciliationHandler<M> {
    pub fn new(maintenance_service: Arc<MemoryMaintenanceService<M>>) -> Self {
        Self {
            maintenance_service,
        }
    }
}

#[async_trait]
impl<M: MemoryRepository + 'static> EventHandler for MemoryReconciliationHandler<M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryReconciliationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "memory-reconciliation"
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
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "memory-reconciliation" {
            return Ok(Reaction::None);
        }

        let (report, events) = self
            .maintenance_service
            .run_maintenance()
            .await
            .map_err(|e| format!("Memory reconciliation failed: {}", e))?;

        tracing::info!(
            expired = report.expired_pruned,
            decayed = report.decayed_pruned,
            promoted = report.promoted,
            conflicts = report.conflicts_resolved,
            "MemoryReconciliationHandler: maintenance complete"
        );

        if events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(events))
        }
    }
}

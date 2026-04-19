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
// MemoryMaintenanceHandler
// ============================================================================

/// Triggered by the "memory-maintenance" scheduled event. Runs full
/// maintenance (prune expired/decayed, check promotions, resolve conflicts).
pub struct MemoryMaintenanceHandler<M: MemoryRepository> {
    memory_service: Arc<MemoryService<M>>,
}

impl<M: MemoryRepository> MemoryMaintenanceHandler<M> {
    pub fn new(memory_service: Arc<MemoryService<M>>) -> Self {
        Self { memory_service }
    }
}

#[async_trait]
impl<M: MemoryRepository + 'static> EventHandler for MemoryMaintenanceHandler<M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryMaintenanceHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "memory-maintenance"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let (report, service_events) = self
            .memory_service
            .run_maintenance()
            .await
            .map_err(|e| format!("Memory maintenance failed: {}", e))?;

        let mut events = service_events;

        let total_pruned = report.expired_pruned + report.decayed_pruned;
        if total_pruned > 0 {
            events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::MemoryPruned {
                    count: total_pruned,
                    reason: format!(
                        "Scheduled maintenance: {} expired, {} decayed, {} promoted, {} conflicts resolved",
                        report.expired_pruned, report.decayed_pruned,
                        report.promoted, report.conflicts_resolved,
                    ),
                },
            });
        }

        // Always emit a summary event for observability
        events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Memory,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::MemoryMaintenanceCompleted {
                expired_pruned: report.expired_pruned,
                decayed_pruned: report.decayed_pruned,
                promoted: report.promoted,
                conflicts_resolved: report.conflicts_resolved,
            },
        });

        Ok(Reaction::EmitEvents(events))
    }
}

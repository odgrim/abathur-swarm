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
// SpecialistCheckHandler
// ============================================================================

/// Triggered by the "specialist-check" scheduled event (30s).
/// Scans tasks in `Failed` status with retries exhausted and signals the
/// orchestrator to trigger specialist processing via a shared channel.
pub struct SpecialistCheckHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    specialist_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
    max_retries: u32,
}

impl<T: TaskRepository> SpecialistCheckHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        specialist_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
        max_retries: u32,
    ) -> Self {
        Self {
            task_repo,
            specialist_tx,
            max_retries,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for SpecialistCheckHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "SpecialistCheckHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "specialist-check"
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
        _event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let failed = self
            .task_repo
            .list_by_status(TaskStatus::Failed)
            .await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        for task in failed {
            if task.retry_count >= self.max_retries {
                // Signal orchestrator to evaluate specialist intervention
                let _ = self.specialist_tx.try_send(task.id);
            }
        }

        Ok(Reaction::None)
    }
}

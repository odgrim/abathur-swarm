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
// TaskReadySpawnHandler
// ============================================================================

/// When a TaskReady event fires, push the task_id into a channel for the
/// orchestrator to spawn an agent. Validates the task is still Ready
/// (idempotent).
pub struct TaskReadySpawnHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    ready_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
}

impl<T: TaskRepository> TaskReadySpawnHandler<T> {
    pub fn new(task_repo: Arc<T>, ready_tx: tokio::sync::mpsc::Sender<uuid::Uuid>) -> Self {
        Self {
            task_repo,
            ready_tx,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskReadySpawnHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskReadySpawnHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskReady".to_string()]),
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
        let task_id = match &event.payload {
            EventPayload::TaskReady { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Validate task is still Ready (idempotent)
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;

        match task {
            Some(t) if t.status == TaskStatus::Ready => {
                let _ = self.ready_tx.try_send(task_id);
            }
            _ => {
                // Task no longer Ready, skip
            }
        }

        Ok(Reaction::None)
    }
}

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
// WorkflowSubtaskCompletionHandler
// ============================================================================

/// When a task completes or fails, check if its parent has workflow state.
/// If so, call `workflow_engine.handle_phase_complete()` to drive the
/// workflow state machine forward.
///
/// **Priority (S4):** Runs at SYSTEM priority (same as `TaskCompletedReadinessHandler`)
/// to prevent priority inversion: workflow state must be updated before or
/// alongside readiness cascades so that a dependent task outside the workflow
/// cannot become Ready before the workflow gate verdict.
///
/// **Deduplication (S3):** Both `TaskCompleted` and `TaskCompletedWithResult` may
/// fire for the same subtask. The workflow engine's `handle_phase_complete` is
/// idempotent — a second call for an already-advanced phase is a no-op.
pub struct WorkflowSubtaskCompletionHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    event_bus: Arc<EventBus>,
    verification_enabled: bool,
}

impl<T: TaskRepository> WorkflowSubtaskCompletionHandler<T> {
    pub fn new(task_repo: Arc<T>, event_bus: Arc<EventBus>, verification_enabled: bool) -> Self {
        Self {
            task_repo,
            event_bus,
            verification_enabled,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for WorkflowSubtaskCompletionHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorkflowSubtaskCompletionHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskCompleted".to_string(),
                    "TaskCompletedWithResult".to_string(),
                    "TaskFailed".to_string(),
                    "TaskCanceled".to_string(),
                ]),
            priority: HandlerPriority::SYSTEM, // S4: was HIGH — raised to SYSTEM to prevent priority inversion with TaskCompletedReadinessHandler
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let subtask_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            EventPayload::TaskCanceled { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Look up the subtask to find its parent
        let subtask = match self.task_repo.get(subtask_id).await {
            Ok(Some(t)) => t,
            _ => return Ok(Reaction::None),
        };

        // Guard: don't let verification tasks re-trigger the workflow handler
        if subtask.task_type.is_verification() {
            return Ok(Reaction::None);
        }

        // S3 dedup: if the subtask is already in a terminal state and we've
        // seen this before, the workflow engine will no-op, but we can skip
        // the parent lookup entirely when the subtask has no parent.
        let parent_id = match subtask.parent_id {
            Some(id) => id,
            None => return Ok(Reaction::None),
        };

        // Check if parent has workflow_state
        let parent = match self.task_repo.get(parent_id).await {
            Ok(Some(t)) => t,
            _ => return Ok(Reaction::None),
        };

        if !parent.has_workflow_state() {
            return Ok(Reaction::None);
        }

        // Fix 3: If parent is Validating but workflow state is NOT Verifying,
        // the parent is stuck in an inconsistent state. Transition it back to
        // Running so the workflow engine can drive it forward.
        if parent.status == TaskStatus::Validating {
            let ws = parent.workflow_state();
            let is_verifying = matches!(&ws, Some(WorkflowState::Verifying { .. }));
            if !is_verifying {
                tracing::warn!(
                    parent_id = %parent_id,
                    subtask_id = %subtask_id,
                    parent_status = ?parent.status,
                    workflow_state = ?ws,
                    "WorkflowSubtaskCompletionHandler: inconsistent state — \
                     parent is Validating but workflow state is not Verifying, \
                     transitioning parent to Running"
                );
                let ts = crate::services::task_service::TaskService::new(self.task_repo.clone());
                if let Err(e) = ts.transition_to_running(parent_id).await {
                    tracing::warn!(
                        parent_id = %parent_id,
                        "WorkflowSubtaskCompletionHandler: failed to transition parent to Running: {}",
                        e
                    );
                }
            }
        }

        // Delegate to workflow engine (via TaskService for all mutations).
        // Idempotent — safe to call twice for same subtask (e.g. from dual TaskCompleted events).
        let task_service = crate::services::task_service::TaskService::new(self.task_repo.clone());
        let engine = crate::services::workflow_engine::WorkflowEngine::new_with_config(
            self.task_repo.clone(),
            task_service,
            self.event_bus.clone(),
            self.verification_enabled,
        );
        if let Err(e) = engine.handle_phase_complete(parent_id, subtask_id).await {
            tracing::warn!(
                parent_id = %parent_id,
                subtask_id = %subtask_id,
                "WorkflowSubtaskCompletionHandler: handle_phase_complete failed: {}",
                e
            );
        }

        Ok(Reaction::None)
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::super::*;
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{Task, TaskStatus};
    use crate::services::EventBusConfig;
    use crate::services::task_service::TaskService;
    use std::sync::Arc;
    use uuid::Uuid;

    #[allow(dead_code)]
    async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        Arc::new(SqliteTaskRepository::new(pool))
    }

    #[allow(dead_code)]
    fn make_task_service(
        repo: &Arc<SqliteTaskRepository>,
    ) -> Arc<TaskService<SqliteTaskRepository>> {
        Arc::new(TaskService::new(repo.clone()))
    }

    // WorkflowSubtaskCompletionHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_task_canceled_handled_by_workflow_subtask_completion_handler() {
        let repo = setup_task_repo().await;
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

        let handler = WorkflowSubtaskCompletionHandler::new(
            repo.clone(),
            event_bus.clone(),
            false, // verification_enabled
        );

        // Create parent task with workflow_state in Running state
        let mut parent = Task::with_title("Parent workflow task", "Do workflow work");
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();

        // Create a canceled subtask linked to the parent
        let mut subtask = Task::with_title("Phase subtask", "Subtask work");
        subtask.parent_id = Some(parent.id);
        subtask.source = TaskSource::SubtaskOf(parent.id);
        subtask.max_retries = 0; // no retries => should fail the phase
        subtask.transition_to(TaskStatus::Ready).unwrap();
        subtask.transition_to(TaskStatus::Running).unwrap();
        subtask.transition_to(TaskStatus::Canceled).unwrap();

        // Write workflow state on the parent before persisting
        let ws = WorkflowState::PhaseRunning {
            workflow_name: "code".to_string(),
            phase_index: 0,
            phase_name: "implement".to_string(),
            subtask_ids: vec![subtask.id],
        };
        parent.set_workflow_state(&ws).unwrap();

        repo.create(&parent).await.unwrap();
        repo.create(&subtask).await.unwrap();

        // Fire a TaskCanceled event for the subtask
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(subtask.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCanceled {
                task_id: subtask.id,
                reason: "parent canceled".to_string(),
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // The handler should process this without error (drives workflow state machine)
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        // WorkflowSubtaskCompletionHandler always returns Reaction::None
        // (side effects happen through workflow engine + event bus)
        assert!(matches!(reaction, Reaction::None));

        // Verify that the handler drove handle_phase_complete, which should
        // have transitioned the parent to Failed (since retries are exhausted)
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(
            updated_parent.status,
            TaskStatus::Failed,
            "Parent task should be Failed after canceled subtask with no retries"
        );
    }

    // ========================================================================
}

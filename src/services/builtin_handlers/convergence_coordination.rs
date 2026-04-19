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
// ConvergenceCoordinationHandler
// ============================================================================

/// When a child task of a decomposed convergent parent completes or fails,
/// check if all siblings are done and cascade the result to the parent.
///
/// This supplements TaskCompletedReadinessHandler (which handles DAG dependencies)
/// with parent-child coordination for decomposed convergent tasks.
pub struct ConvergenceCoordinationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> ConvergenceCoordinationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ConvergenceCoordinationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceCoordinationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskCompleted".to_string(),
                    "TaskCompletedWithResult".to_string(),
                    "TaskFailed".to_string(),
                ]),
            priority: HandlerPriority::HIGH,
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
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Load the completed/failed task to check if it has a parent
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let parent_id = match task.parent_id {
            Some(id) => id,
            None => return Ok(Reaction::None), // Not a child task
        };

        // Load parent task
        let parent = self
            .task_repo
            .get(parent_id)
            .await
            .map_err(|e| format!("Failed to get parent task: {}", e))?;
        let parent = match parent {
            Some(p) => p,
            None => return Ok(Reaction::None),
        };

        // Skip parents that have workflow_state — the workflow engine owns their
        // lifecycle and transitions. Convergence coordination would race with
        // WorkflowSubtaskCompletionHandler and bypass the workflow state machine.
        if parent.has_workflow_state() {
            return Ok(Reaction::None);
        }

        // Only act on parents that are Running with a trajectory (convergent decomposition)
        if parent.status != TaskStatus::Running || parent.trajectory_id.is_none() {
            return Ok(Reaction::None);
        }

        // Load all sibling tasks (children of the parent)
        let siblings = self
            .task_repo
            .get_subtasks(parent_id)
            .await
            .map_err(|e| format!("Failed to get subtasks: {}", e))?;

        // Check if any sibling has failed
        let any_failed = siblings.iter().any(|s| s.status == TaskStatus::Failed);

        // Check if all siblings are in terminal states
        let all_terminal = siblings.iter().all(|s| s.status.is_terminal());

        if !all_terminal {
            return Ok(Reaction::None); // Still waiting for siblings
        }

        let mut new_events = Vec::new();

        if any_failed {
            // Fail the parent (with retry on conflict)
            let result = update_with_retry(
                self.task_repo.as_ref(),
                parent_id,
                |task| {
                    if task.status != TaskStatus::Running {
                        return Ok(false); // parent already transitioned
                    }
                    task.transition_to(TaskStatus::Failed)
                        .map(|_| true)
                        .map_err(|e| format!("transition failed: {}", e))
                },
                3,
                "ConvergenceCoordinationHandler(fail-parent)",
            )
            .await?;

            if let Some(updated_parent) = result {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Error,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(parent_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskFailed {
                        task_id: parent_id,
                        error: "Decomposed child task failed".to_string(),
                        retry_count: updated_parent.retry_count,
                    },
                });
            }
        } else {
            // All siblings completed successfully — complete the parent (with retry on conflict)
            let result = update_with_retry(
                self.task_repo.as_ref(),
                parent_id,
                |task| {
                    if task.status != TaskStatus::Running {
                        return Ok(false); // parent already transitioned
                    }
                    // Go through Validating then Complete in one mutation.
                    // Safety (validation deadlock fix): this is safe because both
                    // transitions happen atomically in a single update_with_retry
                    // closure — the task never persists in Validating state. This
                    // path is for decomposed (convergent) parent tasks whose
                    // children have all completed, NOT for workflow-managed tasks.
                    // Workflow parents use transition_to_validating() which has its
                    // own WorkflowState guard.
                    task.transition_to(TaskStatus::Validating)
                        .map_err(|e| format!("transition to Validating failed: {}", e))?;
                    task.transition_to(TaskStatus::Complete)
                        .map(|_| true)
                        .map_err(|e| format!("transition to Complete failed: {}", e))
                },
                3,
                "ConvergenceCoordinationHandler(complete-parent)",
            )
            .await?;

            if result.is_some() {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Info,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(parent_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskCompleted {
                        task_id: parent_id,
                        tokens_used: 0,
                    },
                });
            }
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
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

    // ========================================================================
    // ConvergenceCoordinationHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_coordination_all_children_complete() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCoordinationHandler::new(repo.clone());

        // Create parent: Running, convergent (has trajectory_id)
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: complete
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        child1.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: complete
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        child2.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskCompleted for child2 (last child to complete)
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(child2.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: child2.id,
                tokens_used: 50,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit a TaskCompleted event for the parent
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::TaskCompleted { task_id, .. } => {
                        assert_eq!(*task_id, parent.id);
                    }
                    other => panic!("Expected TaskCompleted, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify parent is now Complete
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(updated_parent.status, TaskStatus::Complete);
    }

    #[tokio::test]
    async fn test_convergence_coordination_child_fails() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCoordinationHandler::new(repo.clone());

        // Create parent: Running, convergent
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: failed
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        child1.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: complete
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        child2.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskFailed for child1
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(child1.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: child1.id,
                error: "child task error".to_string(),
                retry_count: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit TaskFailed for the parent
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::TaskFailed { task_id, error, .. } => {
                        assert_eq!(*task_id, parent.id);
                        assert!(error.contains("child task failed"));
                    }
                    other => panic!("Expected TaskFailed, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify parent is now Failed
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(updated_parent.status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn test_convergence_coordination_partial_complete() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCoordinationHandler::new(repo.clone());

        // Create parent: Running, convergent
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: complete
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        child1.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: still Running (not terminal)
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskCompleted for child1 only
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(child1.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: child1.id,
                tokens_used: 50,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should return None — still waiting for child2
        assert!(matches!(reaction, Reaction::None));

        // Parent should still be Running
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(updated_parent.status, TaskStatus::Running);
    }

    // ========================================================================
}

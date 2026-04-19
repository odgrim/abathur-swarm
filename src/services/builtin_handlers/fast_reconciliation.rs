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
// FastReconciliationHandler
// ============================================================================

/// Fast-path reconciliation handler (NORMAL priority, every ~15s).
///
/// Only checks cheap state transitions:
/// - Pending tasks with all deps Complete → Ready
/// - Pending tasks with failed/canceled deps → Blocked
/// - Blocked tasks with all deps Complete → Ready
/// - Blocked tasks with all deps terminal and some failed → cascade failure
///
/// Expensive checks (stale Running tasks, Validating timeouts, workflow parking)
/// remain in the slow-path `ReconciliationHandler` (LOW priority, every ~5min).
///
/// Triggered by `ScheduledEventFired { name: "fast-reconciliation" }`.
pub struct FastReconciliationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    task_service: Arc<TaskService<T>>,
}

impl<T: TaskRepository> FastReconciliationHandler<T> {
    pub fn new(task_repo: Arc<T>, task_service: Arc<TaskService<T>>) -> Self {
        Self {
            task_repo,
            task_service,
        }
    }
}

/// Maximum number of tasks to process per reconciliation cycle to avoid
/// monopolising the event loop.
const FAST_RECONCILIATION_BATCH_LIMIT: usize = 100;

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for FastReconciliationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "FastReconciliationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "fast-reconciliation"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let mut corrections: u32 = 0;
        let mut new_events = Vec::new();

        // Check Pending tasks
        let pending = self
            .task_repo
            .list_by_status(TaskStatus::Pending)
            .await
            .map_err(|e| format!("Failed to list pending tasks: {}", e))?;

        for task in pending.iter().take(FAST_RECONCILIATION_BATCH_LIMIT) {
            let deps = self
                .task_repo
                .get_dependencies(task.id)
                .await
                .map_err(|e| format!("Failed to get deps: {}", e))?;

            if deps
                .iter()
                .any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled)
            {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Blocked).is_ok()
                    && try_update_task(&*self.task_repo, &updated, "pending->blocked").await?
                {
                    corrections += 1;
                }
            } else if deps.iter().all(|d| d.status == TaskStatus::Complete) {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Ready).is_ok() {
                    if try_update_task(&*self.task_repo, &updated, "pending->ready").await? {
                        corrections += 1;
                    } else {
                        continue;
                    }
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Debug,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskReady {
                            task_id: task.id,
                            task_title: task.title.clone(),
                        },
                    });
                }
            }
        }

        // Check Blocked tasks that might now be unblocked or should cascade failure
        let blocked = self
            .task_repo
            .list_by_status(TaskStatus::Blocked)
            .await
            .map_err(|e| format!("Failed to list blocked tasks: {}", e))?;

        for task in blocked.iter().take(FAST_RECONCILIATION_BATCH_LIMIT) {
            let deps = self
                .task_repo
                .get_dependencies(task.id)
                .await
                .map_err(|e| format!("Failed to get deps: {}", e))?;

            let has_failed_dep = deps
                .iter()
                .any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled);

            if has_failed_dep {
                let all_failed_or_complete = deps.iter().all(|d| {
                    d.status == TaskStatus::Complete
                        || d.status == TaskStatus::Failed
                        || d.status == TaskStatus::Canceled
                });
                if all_failed_or_complete {
                    let mut updated = task.clone();
                    if updated.transition_to(TaskStatus::Failed).is_ok()
                        && try_update_task(&*self.task_repo, &updated, "blocked->failed cascade")
                            .await?
                    {
                        corrections += 1;
                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: chrono::Utc::now(),
                            severity: EventSeverity::Warning,
                            category: EventCategory::Task,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::TaskFailed {
                                task_id: task.id,
                                error: "cascade-failure: critical dependency failed or canceled"
                                    .to_string(),
                                retry_count: task.retry_count,
                            },
                        });
                    }
                }
                continue;
            }

            if deps.iter().all(|d| d.status == TaskStatus::Complete) {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Ready).is_ok() {
                    if try_update_task(&*self.task_repo, &updated, "blocked->ready").await? {
                        corrections += 1;
                    } else {
                        continue;
                    }
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Debug,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskReady {
                            task_id: task.id,
                            task_title: task.title.clone(),
                        },
                    });
                }
            }
        }

        // Check for zombie Pending tasks whose dependencies include Blocked,
        // terminally-Failed, or Canceled tasks — these will never become Ready.
        let zombie_pending = self
            .task_repo
            .list_by_status(TaskStatus::Pending)
            .await
            .map_err(|e| format!("Failed to list pending tasks (zombie check): {}", e))?;

        for task in zombie_pending.iter().take(FAST_RECONCILIATION_BATCH_LIMIT) {
            let deps = self
                .task_repo
                .get_dependencies(task.id)
                .await
                .map_err(|e| format!("Failed to get deps (zombie check): {}", e))?;

            let any_blocked_or_failed = deps.iter().any(|d| {
                d.status == TaskStatus::Blocked
                    || (d.status == TaskStatus::Failed && !d.can_retry())
                    || d.status == TaskStatus::Canceled
            });
            if any_blocked_or_failed {
                // This task will never become Ready — block it
                match self.task_service.transition_to_blocked(task.id).await {
                    Ok((_, events)) => {
                        new_events.extend(events);
                        corrections += 1;
                    }
                    Err(DomainError::ConcurrencyConflict { .. }) => {}
                    Err(e) => {
                        tracing::warn!("Failed to block zombie pending task {}: {}", task.id, e)
                    }
                }
            }
        }

        // Check for tasks where workflow_state is Completed but TaskStatus is
        // not Complete (workflow/status mismatch).
        let running_tasks = self
            .task_repo
            .list_by_status(TaskStatus::Running)
            .await
            .map_err(|e| format!("Failed to list running tasks (workflow check): {}", e))?;

        for task in running_tasks.iter().take(FAST_RECONCILIATION_BATCH_LIMIT) {
            if let Some(ws) = task.workflow_state()
                && matches!(ws, WorkflowState::Completed { .. })
            {
                // Workflow says done but task is still Running — force complete
                match self.task_service.complete_task(task.id).await {
                    Ok((_, events)) => {
                        new_events.extend(events);
                        corrections += 1;
                    }
                    Err(DomainError::ConcurrencyConflict { .. }) => {}
                    Err(e) => {
                        tracing::warn!("Failed to complete workflow-done task {}: {}", task.id, e)
                    }
                }
            }
        }

        if corrections > 0 {
            tracing::info!(
                "FastReconciliationHandler: made {} corrections",
                corrections
            );
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

    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{Task, TaskStatus};
    use crate::services::task_service::TaskService;
    use std::sync::Arc;

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

    fn make_fast_reconciliation_event() -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "fast-reconciliation".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_fast_reconciliation_pending_to_ready() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = FastReconciliationHandler::new(repo.clone(), task_service);

        // Create a parent task that is Complete
        let mut parent = Task::new("Parent task");
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        parent.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&parent).await.unwrap();

        // Create a child task that is Pending with parent as dependency
        let child = Task::new("Child task");
        repo.create(&child).await.unwrap();
        repo.add_dependency(child.id, parent.id).await.unwrap();

        let event = make_fast_reconciliation_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Verify child was transitioned to Ready
        let updated = repo.get(child.id).await.unwrap().unwrap();
        assert_eq!(
            updated.status,
            TaskStatus::Ready,
            "Pending task with complete deps should become Ready"
        );

        // Verify a TaskReady event was emitted
        match reaction {
            Reaction::EmitEvents(events) => {
                let has_ready = events.iter().any(|e| matches!(&e.payload, EventPayload::TaskReady { task_id, .. } if *task_id == child.id));
                assert!(has_ready, "Should emit TaskReady event");
            }
            Reaction::None => panic!("Expected EmitEvents reaction"),
        }
    }

    #[tokio::test]
    async fn test_fast_reconciliation_no_work() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = FastReconciliationHandler::new(repo.clone(), task_service);

        // No tasks at all
        let event = make_fast_reconciliation_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "No work should produce Reaction::None"
        );
    }

    // ========================================================================
    // update_with_retry tests
    // ========================================================================

    #[tokio::test]
    async fn test_update_with_retry_succeeds() {
        let repo = setup_task_repo().await;

        let task = Task::new("Retry test task");
        repo.create(&task).await.unwrap();

        // Mutation transitions Pending → Ready
        let result = update_with_retry(
            repo.as_ref(),
            task.id,
            |t| {
                t.transition_to(TaskStatus::Ready)
                    .map(|_| true)
                    .map_err(|e| e.to_string())
            },
            3,
            "test",
        )
        .await
        .unwrap();

        assert!(result.is_some(), "Should return updated task");
        let updated = result.unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);

        // Verify it persisted
        let from_db = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(from_db.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_update_with_retry_mutation_not_applicable() {
        let repo = setup_task_repo().await;

        let mut task = Task::new("Already ready");
        task.transition_to(TaskStatus::Ready).unwrap();
        repo.create(&task).await.unwrap();

        // Mutation returns Ok(false) because task is already Ready
        let result = update_with_retry(
            repo.as_ref(),
            task.id,
            |t| {
                if t.status == TaskStatus::Ready {
                    Ok(false) // not applicable
                } else {
                    t.transition_to(TaskStatus::Ready)
                        .map(|_| true)
                        .map_err(|e| e.to_string())
                }
            },
            3,
            "test",
        )
        .await
        .unwrap();

        assert!(
            result.is_none(),
            "Should return None when mutation is not applicable"
        );
    }

    #[tokio::test]
    async fn test_update_with_retry_task_not_found() {
        let repo = setup_task_repo().await;
        let fake_id = uuid::Uuid::new_v4();

        let result = update_with_retry(
            repo.as_ref(),
            fake_id,
            |t| {
                t.transition_to(TaskStatus::Ready)
                    .map(|_| true)
                    .map_err(|e| e.to_string())
            },
            3,
            "test",
        )
        .await;

        assert!(result.is_err(), "Should return error for missing task");
        assert!(result.unwrap_err().contains("not found"));
    }
}

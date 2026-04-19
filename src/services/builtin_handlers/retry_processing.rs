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
// RetryProcessingHandler
// ============================================================================

/// Triggered by the "retry-check" scheduled event. Supplements TaskFailedRetryHandler
/// for cases where the inline handler missed a retry.
pub struct RetryProcessingHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    max_retries: u32,
}

impl<T: TaskRepository> RetryProcessingHandler<T> {
    pub fn new(task_repo: Arc<T>, max_retries: u32) -> Self {
        Self {
            task_repo,
            max_retries,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for RetryProcessingHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "RetryProcessingHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "retry-check"
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
        let failed = self
            .task_repo
            .list_by_status(TaskStatus::Failed)
            .await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        let mut new_events = Vec::new();

        for task in failed {
            // Skip workflow phase subtasks — the workflow engine manages their
            // lifecycle. Generic retry would race with
            // WorkflowSubtaskCompletionHandler and cause double-advance.
            if task.is_workflow_phase_subtask() {
                continue;
            }

            // Skip review-loop-managed tasks — ReviewFailureLoopHandler owns
            // their full retry lifecycle.
            if task.has_review_loop_active_flag() || task.has_review_iteration() {
                continue;
            }

            // Circuit-break consecutive budget failures: tasks that repeatedly
            // exhaust their turn budget should not retry indefinitely.
            if task
                .last_failure_reason()
                .is_some_and(|e| e.starts_with("error_max_turns"))
            {
                let consecutive = task
                    .context
                    .custom
                    .get("consecutive_budget_failures")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if consecutive >= 3 {
                    continue;
                }
            }

            if task.retry_count < self.max_retries {
                let mut updated = task.clone();
                // Use retry() instead of transition_to(Ready) so that
                // retry_count is properly incremented.
                if updated.retry().is_ok() {
                    self.task_repo
                        .update(&updated)
                        .await
                        .map_err(|e| format!("Failed to update: {}", e))?;

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
                            task_title: updated.title.clone(),
                        },
                    });
                }
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
    // RetryProcessingHandler tests
    // ========================================================================

    /// Helper: create a ScheduledEventFired event for "retry-check".
    fn make_retry_check_event() -> UnifiedEvent {
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
                schedule_id: Uuid::new_v4(),
                name: "retry-check".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_retry_processing_skips_workflow_subtasks() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 3);

        // Create a failed workflow subtask
        let mut task = Task::new("Research phase subtask");
        task.max_retries = 3;
        task.set_workflow_phase_value(serde_json::Value::String("research".to_string()));
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should NOT retry — workflow engine owns this task's lifecycle
        assert!(matches!(reaction, Reaction::None));

        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
        assert_eq!(updated.retry_count, 0);
    }

    #[tokio::test]
    async fn test_retry_processing_skips_review_loop_tasks() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 3);

        // Create a failed review-loop task
        let mut task = Task::new("Review iteration task");
        task.max_retries = 3;
        task.set_review_loop_active(true);
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        assert!(matches!(reaction, Reaction::None));
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn test_retry_processing_uses_retry_increments_count() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 3);

        // Create a normal failed task (no workflow/review context)
        let mut task = Task::new("Normal task");
        task.max_retries = 3;
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should retry and increment retry_count
        assert!(matches!(reaction, Reaction::EmitEvents(_)));
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
        assert_eq!(
            updated.retry_count, 1,
            "retry() should increment retry_count"
        );
    }

    #[tokio::test]
    async fn test_retry_processing_circuit_breaks_budget_failures() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 5);

        // Create a task that has already hit budget failure 3 times
        let mut task = Task::new("Budget-exhausted task");
        task.max_retries = 5;
        task.context.custom.insert(
            "consecutive_budget_failures".to_string(),
            serde_json::Value::Number(3.into()),
        );
        task.set_last_failure_reason("error_max_turns: exceeded 40 turns");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should NOT retry — circuit breaker tripped
        assert!(matches!(reaction, Reaction::None));
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
    }
}

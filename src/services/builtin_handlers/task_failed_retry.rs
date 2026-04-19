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
// TaskFailedRetryHandler
// ============================================================================

/// When a task fails with retries remaining, transition it back to Ready.
/// Runs at NORMAL priority (after SYSTEM-priority TaskFailedBlockHandler).
pub struct TaskFailedRetryHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    max_retries: u32,
}

impl<T: TaskRepository> TaskFailedRetryHandler<T> {
    pub fn new(task_repo: Arc<T>, max_retries: u32) -> Self {
        Self {
            task_repo,
            max_retries,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskFailedRetryHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskFailedRetryHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string()]),
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
        let (task_id, error) = match &event.payload {
            EventPayload::TaskFailed { task_id, error, .. } => (*task_id, error.as_str()),
            _ => return Ok(Reaction::None),
        };

        // Re-fetch task to check it's still Failed (idempotency)
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if task.status != TaskStatus::Failed {
            return Ok(Reaction::None);
        }

        // Use task.can_retry() which checks retry_count < max_retries atomically
        if !task.can_retry() || task.retry_count >= self.max_retries {
            return Ok(Reaction::None);
        }

        // Skip tasks superseded by the review failure loop-back handler,
        // and tasks that are part of a review loop chain (ReviewFailureLoopHandler
        // manages their full lifecycle — independent retry would create duplicate work tracks).
        if task.has_review_loop_active_flag() || task.has_review_iteration() {
            return Ok(Reaction::None);
        }

        // Skip workflow phase subtasks — the workflow engine manages rework via
        // verification retries and gate escalation. Generic retry would race with
        // WorkflowSubtaskCompletionHandler and cause double-advance.
        if task.is_workflow_phase_subtask() {
            return Ok(Reaction::None);
        }

        // Skip gate-rejected tasks — retrying would race with the orchestrator's
        // replay_gate_rejection_event which re-emits WorkflowGateRejected for
        // adapter lifecycle sync. The rejection is final.
        if let Some(crate::domain::models::workflow_state::WorkflowState::Rejected { .. }) =
            crate::services::workflow_engine::WorkflowEngine::<T>::read_state_from_task(&task)
        {
            return Ok(Reaction::None);
        }

        let is_max_turns = error.starts_with("error_max_turns");

        // Circuit-break: tasks that repeatedly exhaust their turn budget should not retry
        // indefinitely. After MAX_CONSECUTIVE_BUDGET_FAILURES consecutive budget failures,
        // leave the task in Failed state so upstream handlers (review loop, specialist
        // triggers) can respond appropriately.
        const MAX_CONSECUTIVE_BUDGET_FAILURES: u64 = 3;
        if is_max_turns {
            let consecutive = task
                .context
                .custom
                .get("consecutive_budget_failures")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            if consecutive >= MAX_CONSECUTIVE_BUDGET_FAILURES {
                tracing::info!(
                    "Task {} circuit-breaker: {} consecutive budget failures, not retrying",
                    task_id,
                    consecutive
                );
                return Ok(Reaction::None);
            }
        }

        // Skip exponential backoff for structural failures (max_turns) — immediate retry
        if !is_max_turns {
            let backoff_secs = 2u64.pow(task.retry_count.min(10));
            if let Some(completed_at) = task.completed_at {
                let elapsed = (chrono::Utc::now() - completed_at).num_seconds();
                if elapsed < backoff_secs as i64 {
                    // Not ready to retry yet; the scheduled retry-check will try again
                    return Ok(Reaction::None);
                }
            }
        }

        let mut updated = task.clone();

        // For max_turns failures, inject hint so the spawner can increase the turn budget
        // and track consecutive failures for the circuit-breaker above.
        if is_max_turns {
            let consecutive = updated
                .context
                .custom
                .get("consecutive_budget_failures")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            updated.context.custom.insert(
                "consecutive_budget_failures".to_string(),
                serde_json::json!(consecutive),
            );
            updated
                .context
                .push_hint_bounded("retry:max_turns_exceeded".to_string());
            updated.set_last_failure_reason(error);
        } else {
            // Non-budget failure — reset the consecutive budget failure counter so a
            // later budget failure doesn't inherit a stale count from a different failure mode.
            updated.context.custom.remove("consecutive_budget_failures");
        }

        if updated.retry().is_ok() {
            self.task_repo
                .update(&updated)
                .await
                .map_err(|e| format!("Failed to update task: {}", e))?;

            let events = vec![
                UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(task_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskRetrying {
                        task_id,
                        attempt: updated.retry_count,
                        max_attempts: updated.max_retries,
                    },
                },
                UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Debug,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(task_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskReady {
                        task_id,
                        task_title: updated.title.clone(),
                    },
                },
            ];
            return Ok(Reaction::EmitEvents(events));
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

    // ========================================================================
    // TaskFailedRetryHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_retry_handler_injects_max_turns_hint() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 3);

        // Create a task that has failed due to max_turns
        let mut task = Task::new("Research codebase");
        task.max_retries = 3;
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: task.id,
                error: "error_max_turns: agent exceeded 25 turns".to_string(),
                retry_count: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should have emitted retry events
        assert!(matches!(reaction, Reaction::EmitEvents(_)));

        // Verify the retried task has the hint and custom field
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
        assert!(
            updated
                .context
                .hints
                .contains(&"retry:max_turns_exceeded".to_string())
        );
        assert!(updated.last_failure_reason().is_some());
    }

    #[tokio::test]
    async fn test_retry_handler_skips_backoff_for_max_turns() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 3);

        // Create a task that has already been retried once (retry_count=1)
        // and just failed again. With normal backoff, 2^1 = 2s wait would apply.
        let mut task = Task::new("Research codebase");
        task.max_retries = 3;
        task.retry_count = 1;
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        // Set completed_at to "just now" so backoff would normally block
        task.completed_at = Some(chrono::Utc::now());
        repo.create(&task).await.unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: task.id,
                error: "error_max_turns: agent exceeded 25 turns".to_string(),
                retry_count: 1,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Should retry immediately despite completed_at being "just now"
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::EmitEvents(_)));

        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
        assert_eq!(updated.retry_count, 2);
    }
}

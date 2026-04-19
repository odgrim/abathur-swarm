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
// TaskFailedBlockHandler
// ============================================================================

/// When a task fails with retries exhausted, block its dependent tasks.
///
/// State transitions are routed through `TaskService` to ensure validation,
/// optimistic locking, and proper event emission.
///
/// Blocking is **recursive**: the entire dependent subtree is blocked via
/// iterative BFS so that transitive dependents (e.g. A→B→C→D) are all
/// blocked when A fails.
pub struct TaskFailedBlockHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    task_service: Arc<TaskService<T>>,
}

/// Maximum number of tasks that will be blocked in a single cascade to
/// guard against cycles or unexpectedly large graphs.
const BLOCK_CASCADE_LIMIT: usize = 1000;

impl<T: TaskRepository> TaskFailedBlockHandler<T> {
    pub fn new(task_repo: Arc<T>, task_service: Arc<TaskService<T>>) -> Self {
        Self {
            task_repo,
            task_service,
        }
    }

    /// Recursively block all transitive dependents of `root_task_id` using
    /// iterative BFS. Returns `Err` only on non-recoverable failures;
    /// `ConcurrencyConflict` and `InvalidStateTransition` are logged and
    /// skipped.
    async fn block_dependent_subtree(&self, root_task_id: uuid::Uuid) -> Result<(), String> {
        let direct_dependents = self
            .task_repo
            .get_dependents(root_task_id)
            .await
            .map_err(|e| format!("Failed to get dependents: {}", e))?;

        let mut queue: Vec<uuid::Uuid> = Vec::new();
        let mut blocked_count: usize = 0;

        for dep in direct_dependents {
            if dep.status == TaskStatus::Blocked || dep.status.is_terminal() {
                continue;
            }
            match self.task_service.transition_to_blocked(dep.id).await {
                Ok(_) => {
                    blocked_count += 1;
                    queue.push(dep.id);
                }
                Err(DomainError::InvalidStateTransition { .. }) => {
                    // Idempotent skip
                }
                Err(DomainError::ConcurrencyConflict { entity, id }) => {
                    tracing::warn!(
                        "TaskFailedBlockHandler: ConcurrencyConflict on {} {} while blocking dependent; skipping",
                        entity,
                        id
                    );
                }
                Err(e) => {
                    return Err(format!("Failed to transition task to blocked: {}", e));
                }
            }
        }

        // BFS over transitive dependents
        while let Some(blocked_id) = queue.pop() {
            if blocked_count >= BLOCK_CASCADE_LIMIT {
                tracing::warn!(
                    "TaskFailedBlockHandler: reached cascade limit of {} tasks; stopping BFS",
                    BLOCK_CASCADE_LIMIT
                );
                break;
            }

            let grandchildren = self
                .task_repo
                .get_dependents(blocked_id)
                .await
                .map_err(|e| format!("Failed to get dependents: {}", e))?;

            for gc in grandchildren {
                if gc.status == TaskStatus::Blocked || gc.status.is_terminal() {
                    continue;
                }
                match self.task_service.transition_to_blocked(gc.id).await {
                    Ok(_) => {
                        blocked_count += 1;
                        queue.push(gc.id);
                    }
                    Err(DomainError::InvalidStateTransition { .. }) => {
                        // Idempotent skip
                    }
                    Err(DomainError::ConcurrencyConflict { entity, id }) => {
                        tracing::warn!(
                            "TaskFailedBlockHandler: ConcurrencyConflict on {} {} while blocking transitive dependent; skipping",
                            entity,
                            id
                        );
                    }
                    Err(e) => {
                        return Err(format!("Failed to transition task to blocked: {}", e));
                    }
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskFailedBlockHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskFailedBlockHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string(), "TaskCanceled".to_string()]),
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::CircuitBreak,
            critical: true,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let (task_id, retry_count) = match &event.payload {
            EventPayload::TaskFailed {
                task_id,
                retry_count,
                ..
            } => (*task_id, *retry_count),
            EventPayload::TaskCanceled { task_id, .. } => {
                // For canceled tasks, always block dependents (retries don't apply)
                self.block_dependent_subtree(*task_id).await?;
                return Ok(Reaction::None);
            }
            _ => return Ok(Reaction::None),
        };

        // Only block dependents if retries are exhausted.
        // Fetch the actual task to check max_retries.
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if retry_count < task.max_retries {
            return Ok(Reaction::None);
        }

        self.block_dependent_subtree(task_id).await?;

        Ok(Reaction::None)
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::super::*;
    use super::*;
    use crate::adapters::sqlite::test_support::{make_task_service, setup_task_repo};
    use crate::domain::models::{Task, TaskStatus};
    use crate::services::EventBusConfig;
    use crate::services::task_service::TaskService;
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_task_failed_block_handler() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = TaskFailedBlockHandler::new(repo.clone(), task_service);

        // Create upstream task that has exhausted retries
        let mut upstream = Task::new("Upstream");
        upstream.max_retries = 2;
        upstream.retry_count = 2;
        upstream.transition_to(TaskStatus::Ready).unwrap();
        upstream.transition_to(TaskStatus::Running).unwrap();
        upstream.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&upstream).await.unwrap();

        // Create downstream task
        let downstream = Task::new("Downstream");
        repo.create(&downstream).await.unwrap();
        repo.add_dependency(downstream.id, upstream.id)
            .await
            .unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(upstream.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: upstream.id,
                error: "test failure".to_string(),
                retry_count: 2,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        handler.handle(&event, &ctx).await.unwrap();

        // Verify downstream is now Blocked
        let updated = repo.get(downstream.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Blocked);
    }

    #[tokio::test]
    async fn test_task_failed_block_handler_retries_remaining() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = TaskFailedBlockHandler::new(repo.clone(), task_service);

        // Create upstream task that still has retries remaining
        let mut upstream = Task::new("Upstream");
        upstream.max_retries = 3;
        upstream.retry_count = 1;
        upstream.transition_to(TaskStatus::Ready).unwrap();
        upstream.transition_to(TaskStatus::Running).unwrap();
        upstream.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&upstream).await.unwrap();

        let downstream = Task::new("Downstream");
        repo.create(&downstream).await.unwrap();
        repo.add_dependency(downstream.id, upstream.id)
            .await
            .unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(upstream.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: upstream.id,
                error: "test failure".to_string(),
                retry_count: 1,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        handler.handle(&event, &ctx).await.unwrap();

        // Downstream should NOT be blocked since retries remain
        let updated = repo.get(downstream.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn test_task_failed_block_handler_recursive_cascade() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = TaskFailedBlockHandler::new(repo.clone(), task_service);

        // Create chain: A → B → C → D
        let mut task_a = Task::new("A");
        task_a.max_retries = 0;
        task_a.transition_to(TaskStatus::Ready).unwrap();
        task_a.transition_to(TaskStatus::Running).unwrap();
        task_a.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task_a).await.unwrap();

        let task_b = Task::new("B");
        repo.create(&task_b).await.unwrap();
        repo.add_dependency(task_b.id, task_a.id).await.unwrap();

        let task_c = Task::new("C");
        repo.create(&task_c).await.unwrap();
        repo.add_dependency(task_c.id, task_b.id).await.unwrap();

        let task_d = Task::new("D");
        repo.create(&task_d).await.unwrap();
        repo.add_dependency(task_d.id, task_c.id).await.unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_a.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: task_a.id,
                error: "test failure".to_string(),
                retry_count: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        handler.handle(&event, &ctx).await.unwrap();

        // All transitive dependents should be Blocked
        let b = repo.get(task_b.id).await.unwrap().unwrap();
        assert_eq!(b.status, TaskStatus::Blocked, "B should be blocked");

        let c = repo.get(task_c.id).await.unwrap().unwrap();
        assert_eq!(c.status, TaskStatus::Blocked, "C should be blocked");

        let d = repo.get(task_d.id).await.unwrap().unwrap();
        assert_eq!(d.status, TaskStatus::Blocked, "D should be blocked");
    }
}

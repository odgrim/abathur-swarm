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
// TaskCompletedReadinessHandler
// ============================================================================

/// When a task completes, check its dependents and transition Pending/Blocked → Ready
/// if all their dependencies are now complete.
///
/// State transitions are routed through `TaskService` to ensure validation,
/// optimistic locking, and proper event emission.
///
/// **Deduplication (S3):** Both `TaskCompleted` and `TaskCompletedWithResult` may
/// fire for the same task (from different code paths). The handler matches both
/// because some paths emit only one variant. The early status check on each
/// dependent (line `dep.status != Pending && dep.status != Blocked`) ensures
/// idempotent behavior — a second invocation for the same task_id is a no-op
/// when the dependents have already been transitioned.
pub struct TaskCompletedReadinessHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    task_service: Arc<TaskService<T>>,
}

impl<T: TaskRepository> TaskCompletedReadinessHandler<T> {
    pub fn new(task_repo: Arc<T>, task_service: Arc<TaskService<T>>) -> Self {
        Self {
            task_repo,
            task_service,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskCompletedReadinessHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskCompletedReadinessHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskCompleted".to_string(),
                    "TaskCompletedWithResult".to_string(),
                ]),
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
        let task_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        let dependents = self
            .task_repo
            .get_dependents(task_id)
            .await
            .map_err(|e| format!("Failed to get dependents: {}", e))?;

        let mut new_events = Vec::new();

        for dep in dependents {
            // Idempotency: only act if still in a state that needs updating
            if dep.status != TaskStatus::Pending && dep.status != TaskStatus::Blocked {
                continue;
            }

            let all_deps = self
                .task_repo
                .get_dependencies(dep.id)
                .await
                .map_err(|e| format!("Failed to get dependencies: {}", e))?;

            if all_deps.iter().all(|d| d.status == TaskStatus::Complete) {
                match self.task_service.transition_to_ready(dep.id).await {
                    Ok((_task, events)) => {
                        new_events.extend(events);
                    }
                    Err(DomainError::InvalidStateTransition { .. }) => {
                        // Idempotent skip: task already transitioned
                    }
                    Err(e) => {
                        return Err(format!("Failed to transition task to ready: {}", e));
                    }
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

    #[tokio::test]
    async fn test_task_completed_readiness_handler() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = TaskCompletedReadinessHandler::new(repo.clone(), task_service);

        // Create upstream task
        let mut upstream = Task::new("Upstream task");
        upstream.transition_to(TaskStatus::Ready).unwrap();
        upstream.transition_to(TaskStatus::Running).unwrap();
        upstream.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&upstream).await.unwrap();

        // Create downstream task that depends on upstream
        let downstream = Task::new("Downstream task");
        repo.create(&downstream).await.unwrap();
        repo.add_dependency(downstream.id, upstream.id)
            .await
            .unwrap();

        // Fire the handler with a TaskCompleted event
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(upstream.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: upstream.id,
                tokens_used: 100,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should have emitted a TaskReady event
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                assert!(matches!(events[0].payload, EventPayload::TaskReady { .. }));
            }
            Reaction::None => panic!("Expected EmitEvents reaction"),
        }

        // Verify downstream task is now Ready
        let updated = repo.get(downstream.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_task_completed_readiness_handler_idempotent() {
        let repo = setup_task_repo().await;
        let task_service = make_task_service(&repo);
        let handler = TaskCompletedReadinessHandler::new(repo.clone(), task_service);

        // Create upstream and downstream
        let mut upstream = Task::new("Upstream");
        upstream.transition_to(TaskStatus::Ready).unwrap();
        upstream.transition_to(TaskStatus::Running).unwrap();
        upstream.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&upstream).await.unwrap();

        let mut downstream = Task::new("Downstream");
        downstream.transition_to(TaskStatus::Ready).unwrap(); // Already ready
        repo.create(&downstream).await.unwrap();
        repo.add_dependency(downstream.id, upstream.id)
            .await
            .unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(upstream.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: upstream.id,
                tokens_used: 100,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Second call should be a no-op since downstream is already Ready
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));
    }
}

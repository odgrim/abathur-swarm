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
// ReadyTaskPollingHandler
// ============================================================================

/// Periodic safety-net companion to TaskReadySpawnHandler. Polls get_ready_tasks()
/// to discover tasks that are Ready in the DB but never got pushed to the spawn
/// channel (e.g. because the TaskReady event was dropped by the broadcast channel).
///
/// Triggered by `ScheduledEventFired { name: "ready-task-poll" }`.
pub struct ReadyTaskPollingHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    ready_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
}

impl<T: TaskRepository> ReadyTaskPollingHandler<T> {
    pub fn new(task_repo: Arc<T>, ready_tx: tokio::sync::mpsc::Sender<uuid::Uuid>) -> Self {
        Self {
            task_repo,
            ready_tx,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ReadyTaskPollingHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ReadyTaskPollingHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "ready-task-poll"
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
        _event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        let ready_tasks = self
            .task_repo
            .get_ready_tasks(100)
            .await
            .map_err(|e| format!("Failed to poll ready tasks: {}", e))?;

        let mut pushed = 0usize;
        for task in &ready_tasks {
            if self.ready_tx.try_send(task.id).is_ok() {
                pushed += 1;
            }
        }

        if pushed > 0 {
            tracing::info!(
                "ReadyTaskPollingHandler: pushed {} ready task(s) to spawn channel",
                pushed
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

    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{Task, TaskStatus};
    use std::sync::Arc;

    #[allow(dead_code)]
    async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        Arc::new(SqliteTaskRepository::new(pool))
    }

    fn make_ready_task_poll_event() -> UnifiedEvent {
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
                name: "ready-task-poll".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_ready_task_polling_pushes_ready_tasks() {
        let repo = setup_task_repo().await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let handler = ReadyTaskPollingHandler::new(repo.clone(), tx);

        // Create a task and transition it to Ready
        let mut task = Task::new("Ready task");
        task.transition_to(TaskStatus::Ready).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_ready_task_poll_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));

        // Verify the task was pushed to the channel
        let received = rx.try_recv().unwrap();
        assert_eq!(received, task.id);
    }

    #[tokio::test]
    async fn test_ready_task_polling_no_ready_tasks() {
        let repo = setup_task_repo().await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let handler = ReadyTaskPollingHandler::new(repo.clone(), tx);

        // Create a Pending task (not Ready)
        let task = Task::new("Pending task");
        repo.create(&task).await.unwrap();

        let event = make_ready_task_poll_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        handler.handle(&event, &ctx).await.unwrap();

        // Should not push anything
        assert!(rx.try_recv().is_err());
    }
}

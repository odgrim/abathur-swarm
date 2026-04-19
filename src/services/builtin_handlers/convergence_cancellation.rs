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
// ConvergenceCancellationHandler
// ============================================================================

/// When a convergent parent task is canceled or fails, cascade cancellation to all
/// Running/Ready children.
pub struct ConvergenceCancellationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> ConvergenceCancellationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ConvergenceCancellationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceCancellationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskCanceled".to_string(), "TaskFailed".to_string()]),
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
        let (task_id, reason_verb) = match &event.payload {
            EventPayload::TaskCanceled { task_id, .. } => (*task_id, "canceled"),
            EventPayload::TaskFailed { task_id, .. } => (*task_id, "failed"),
            _ => return Ok(Reaction::None),
        };

        // Load the canceled task
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        // Only cascade if this is a convergent task with a trajectory (decomposed parent)
        if task.trajectory_id.is_none() {
            return Ok(Reaction::None);
        }

        // Load children
        let children = self
            .task_repo
            .get_subtasks(task_id)
            .await
            .map_err(|e| format!("Failed to get subtasks: {}", e))?;

        let mut new_events = Vec::new();

        for child in children {
            let child_id = child.id;
            let result = update_with_retry(
                self.task_repo.as_ref(),
                child_id,
                |task| {
                    if task.status.is_terminal() {
                        return Ok(false);
                    }
                    task.transition_to(TaskStatus::Canceled)
                        .map(|_| true)
                        .map_err(|e| format!("transition failed: {}", e))
                },
                3,
                "ConvergenceCancellationHandler",
            )
            .await?;

            if result.is_some() {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(child_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskCanceled {
                        task_id: child_id,
                        reason: format!("Parent task {} was {}", task_id, reason_verb),
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
    use crate::adapters::sqlite::test_support::{make_task_service, setup_task_repo};
    use crate::domain::models::{Task, TaskStatus};
    use crate::services::EventBusConfig;
    use crate::services::task_service::TaskService;
    use std::sync::Arc;
    use uuid::Uuid;

    // ConvergenceCancellationHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_cancellation_cascades() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCancellationHandler::new(repo.clone());

        // Create parent: Canceled, convergent (has trajectory_id)
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        parent.transition_to(TaskStatus::Canceled).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: Running
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: Running
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskCanceled for the parent
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Warning,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(parent.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCanceled {
                task_id: parent.id,
                reason: "user requested cancellation".to_string(),
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit TaskCanceled events for both children
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 2);
                let canceled_ids: Vec<Uuid> = events
                    .iter()
                    .map(|e| match &e.payload {
                        EventPayload::TaskCanceled { task_id, .. } => *task_id,
                        other => panic!("Expected TaskCanceled, got {:?}", other),
                    })
                    .collect();
                assert!(canceled_ids.contains(&child1.id));
                assert!(canceled_ids.contains(&child2.id));
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify both children are now Canceled
        let updated_child1 = repo.get(child1.id).await.unwrap().unwrap();
        assert_eq!(updated_child1.status, TaskStatus::Canceled);
        let updated_child2 = repo.get(child2.id).await.unwrap().unwrap();
        assert_eq!(updated_child2.status, TaskStatus::Canceled);
    }

    #[tokio::test]
    async fn test_convergence_cancellation_cascades_on_parent_failure() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCancellationHandler::new(repo.clone());

        // Create parent: Failed, convergent (has trajectory_id)
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        parent.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: Running
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: Ready (non-terminal, should also be canceled)
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskFailed for the parent
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(parent.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: parent.id,
                error: "agent process crashed".to_string(),
                retry_count: 1,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit TaskCanceled events for both children
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 2);
                for evt in &events {
                    match &evt.payload {
                        EventPayload::TaskCanceled { reason, .. } => {
                            assert!(
                                reason.contains("failed"),
                                "Reason should mention 'failed': {}",
                                reason
                            );
                        }
                        other => panic!("Expected TaskCanceled, got {:?}", other),
                    }
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify both children are now Canceled
        let updated_child1 = repo.get(child1.id).await.unwrap().unwrap();
        assert_eq!(updated_child1.status, TaskStatus::Canceled);
        let updated_child2 = repo.get(child2.id).await.unwrap().unwrap();
        assert_eq!(updated_child2.status, TaskStatus::Canceled);
    }

    // ========================================================================
}

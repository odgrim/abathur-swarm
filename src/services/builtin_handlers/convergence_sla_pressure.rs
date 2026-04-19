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
// ConvergenceSLAPressureHandler
// ============================================================================

/// When a convergent task receives SLA pressure events (TaskSLAWarning or
/// TaskSLACritical), add hints to the task context so the convergent execution
/// loop can adjust its policy (lower acceptance threshold, skip expensive
/// overseers).
///
/// Idempotent: checks for existing hints before adding.
pub struct ConvergenceSLAPressureHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> ConvergenceSLAPressureHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ConvergenceSLAPressureHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceSLAPressureHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskSLAWarning".to_string(),
                    "TaskSLACritical".to_string(),
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
        let (task_id, hint) = match &event.payload {
            EventPayload::TaskSLAWarning { task_id, .. } => (*task_id, "sla:warning"),
            EventPayload::TaskSLACritical { task_id, .. } => (*task_id, "sla:critical"),
            _ => return Ok(Reaction::None),
        };

        // Load the task
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        // Only act on convergent tasks (those with a trajectory_id)
        if task.trajectory_id.is_none() {
            return Ok(Reaction::None);
        }

        // Idempotency: don't add the hint if it already exists
        if task.context.hints.iter().any(|h| h == hint) {
            return Ok(Reaction::None);
        }

        // When escalating to critical, also ensure warning hint is present
        let mut updated = task.clone();
        if hint == "sla:critical" && !updated.context.hints.iter().any(|h| h == "sla:warning") {
            updated.context.push_hint_bounded("sla:warning".to_string());
        }
        updated.context.push_hint_bounded(hint.to_string());
        updated.updated_at = chrono::Utc::now();

        self.task_repo
            .update(&updated)
            .await
            .map_err(|e| format!("Failed to update task with SLA hint: {}", e))?;

        tracing::info!(
            task_id = %task_id,
            hint = hint,
            "Added SLA pressure hint to convergent task context"
        );

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

    // ConvergenceSLAPressureHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_sla_pressure_warning() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceSLAPressureHandler::new(repo.clone());

        // Create convergent task (has trajectory_id), in Running state
        let mut task = Task::new("Convergent task with SLA");
        task.trajectory_id = Some(Uuid::new_v4());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        repo.create(&task).await.unwrap();

        // Fire TaskSLAWarning event
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Warning,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskSLAWarning {
                task_id: task.id,
                deadline: "2026-01-01T00:00:00Z".to_string(),
                remaining_secs: 60,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));

        // Verify the task now has "sla:warning" hint
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert!(updated.context.hints.contains(&"sla:warning".to_string()));
    }

    #[tokio::test]
    async fn test_convergence_sla_pressure_non_convergent_ignored() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceSLAPressureHandler::new(repo.clone());

        // Create direct task (no trajectory_id)
        let mut task = Task::new("Direct task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        repo.create(&task).await.unwrap();

        assert!(task.trajectory_id.is_none()); // sanity check

        // Fire TaskSLAWarning event
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Warning,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskSLAWarning {
                task_id: task.id,
                deadline: "2026-01-01T00:00:00Z".to_string(),
                remaining_secs: 60,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));

        // Verify the task does NOT have any SLA hints
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert!(!updated.context.hints.contains(&"sla:warning".to_string()));
    }

    // ========================================================================
}

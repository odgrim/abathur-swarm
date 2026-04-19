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
// ConvergenceEvolutionHandler
// ============================================================================

/// When a convergent task terminates, record convergence-specific metrics that
/// feed the evolution loop. Emits a TaskCompletedWithResult event so that the
/// EvolutionEvaluationHandler can pick it up and track per-agent-type
/// convergence performance.
///
/// Idempotent: only acts on ConvergenceTerminated events and checks task state
/// before emitting.
pub struct ConvergenceEvolutionHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> ConvergenceEvolutionHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ConvergenceEvolutionHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceEvolutionHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Convergence])
                .payload_types(vec!["ConvergenceTerminated".to_string()]),
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
        let (
            task_id,
            _trajectory_id,
            outcome,
            total_iterations,
            total_tokens,
            final_convergence_level,
        ) = match &event.payload {
            EventPayload::ConvergenceTerminated(p) => (
                p.task_id,
                p.trajectory_id,
                p.outcome.clone(),
                p.total_iterations,
                p.total_tokens,
                p.final_convergence_level,
            ),
            _ => return Ok(Reaction::None),
        };

        // Load the task to get agent_type and compute duration
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        // Compute duration from started_at to now (or completed_at if available)
        let duration_secs = task
            .started_at
            .map(|started| {
                let end = task.completed_at.unwrap_or_else(chrono::Utc::now);
                (end - started).num_seconds().max(0) as u64
            })
            .unwrap_or(0);

        // Map convergence outcome to task result status
        let (status_str, error) = match outcome.as_str() {
            "converged" => ("Complete".to_string(), None),
            "exhausted" => (
                "Failed".to_string(),
                Some("Convergence exhausted: max iterations reached".to_string()),
            ),
            "trapped" => (
                "Failed".to_string(),
                Some("Convergence trapped: attractor limit cycle detected".to_string()),
            ),
            "budget_denied" => (
                "Failed".to_string(),
                Some("Convergence budget extension denied".to_string()),
            ),
            "decomposed" => ("Complete".to_string(), None), // Decomposition is a valid outcome
            other => (
                "Failed".to_string(),
                Some(format!("Convergence terminated: {}", other)),
            ),
        };

        // Store convergence metadata on the task context for evolution queries
        let mut updated = task.clone();
        updated.context.custom.insert(
            "convergence_iterations".to_string(),
            serde_json::Value::Number(serde_json::Number::from(total_iterations)),
        );
        updated.context.custom.insert(
            "convergence_tokens".to_string(),
            serde_json::Value::Number(serde_json::Number::from(total_tokens)),
        );
        updated.context.custom.insert(
            "convergence_level".to_string(),
            serde_json::json!(final_convergence_level),
        );
        updated.context.custom.insert(
            "convergence_outcome".to_string(),
            serde_json::Value::String(outcome.clone()),
        );
        updated.updated_at = chrono::Utc::now();

        self.task_repo
            .update(&updated)
            .await
            .map_err(|e| format!("Failed to update task with convergence metadata: {}", e))?;

        // Emit TaskCompletedWithResult so EvolutionEvaluationHandler can track it
        let result_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: event.goal_id,
            task_id: Some(task_id),
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::TaskCompletedWithResult {
                task_id,
                result: TaskResultPayload {
                    task_id,
                    status: status_str,
                    error,
                    duration_secs,
                    retry_count: updated.retry_count,
                    tokens_used: total_tokens,
                },
            },
        };

        tracing::info!(
            task_id = %task_id,
            outcome = %outcome,
            iterations = total_iterations,
            tokens = total_tokens,
            convergence_level = final_convergence_level,
            "Recorded convergence evolution metrics"
        );

        Ok(Reaction::EmitEvents(vec![result_event]))
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

    // ConvergenceEvolutionHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_evolution_handler_records_metrics() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceEvolutionHandler::new(repo.clone());

        // Create convergent task in Running state
        let mut task = Task::new("Convergent task for evolution");
        let trajectory_id = Uuid::new_v4();
        task.trajectory_id = Some(trajectory_id);
        task.agent_type = Some("coder".to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        repo.create(&task).await.unwrap();

        // Fire ConvergenceTerminated event
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Convergence,
            goal_id: None,
            task_id: Some(task.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ConvergenceTerminated(ConvergenceTerminatedPayload {
                task_id: task.id,
                trajectory_id,
                outcome: "converged".to_string(),
                total_iterations: 5,
                total_tokens: 2500,
                final_convergence_level: 0.92,
            }),
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit a TaskCompletedWithResult event
        match &reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::TaskCompletedWithResult { task_id, result } => {
                        assert_eq!(*task_id, task.id);
                        assert_eq!(result.status, "Complete");
                        assert_eq!(result.tokens_used, 2500);
                        assert!(result.error.is_none());
                    }
                    other => panic!("Expected TaskCompletedWithResult, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify task context was updated with convergence metrics
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(
            updated.context.custom.get("convergence_iterations"),
            Some(&serde_json::Value::Number(serde_json::Number::from(5u32)))
        );
        assert_eq!(
            updated.context.custom.get("convergence_tokens"),
            Some(&serde_json::Value::Number(serde_json::Number::from(
                2500u64
            )))
        );
        assert_eq!(
            updated.context.custom.get("convergence_level"),
            Some(&serde_json::json!(0.92))
        );
        assert_eq!(
            updated.context.custom.get("convergence_outcome"),
            Some(&serde_json::Value::String("converged".to_string()))
        );
    }

    // ========================================================================
}

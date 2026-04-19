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
// ConvergenceMemoryHandler
// ============================================================================

/// When a convergent task terminates (ConvergenceTerminated event), record
/// convergence outcomes to memory for future strategy warm-starting.
///
/// On "converged" outcome: store success memory (episodic tier, Pattern type)
/// with task complexity, strategy sequence, iterations, and tokens.
///
/// On "exhausted"/"trapped"/"budget_denied" outcome: store failure memory
/// (episodic tier, Error type) with the same metrics so future bandits can
/// deprioritize strategies that failed on similar tasks.
///
/// Idempotent: uses an idempotency key based on trajectory_id to avoid
/// duplicate memory entries.
pub struct ConvergenceMemoryHandler<T: TaskRepository, M: MemoryRepository> {
    task_repo: Arc<T>,
    memory_repo: Arc<M>,
}

impl<T: TaskRepository, M: MemoryRepository> ConvergenceMemoryHandler<T, M> {
    pub fn new(task_repo: Arc<T>, memory_repo: Arc<M>) -> Self {
        Self {
            task_repo,
            memory_repo,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, M: MemoryRepository + 'static> EventHandler
    for ConvergenceMemoryHandler<T, M>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceMemoryHandler".to_string(),
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
            trajectory_id,
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

        // Idempotency: check if we already stored a memory for this trajectory
        let idempotency_key = format!("convergence-outcome:{}", trajectory_id);
        let existing = self
            .memory_repo
            .get_by_key(&idempotency_key, "convergence")
            .await
            .map_err(|e| format!("Failed to check existing memory: {}", e))?;
        if existing.is_some() {
            return Ok(Reaction::None);
        }

        // Load the task for additional context (complexity, agent_type)
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let complexity = format!("{:?}", task.routing_hints.complexity);
        let agent_type = task
            .agent_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let is_success = outcome == "converged";

        // Build memory content as a structured summary
        let content = format!(
            "Convergence {outcome} for task {task_id} (trajectory {trajectory_id}):\n\
             - complexity: {complexity}\n\
             - agent_type: {agent_type}\n\
             - iterations: {total_iterations}\n\
             - tokens: {total_tokens}\n\
             - final_convergence_level: {final_convergence_level:.3}",
        );

        // Build the memory entry
        let memory_type = if is_success {
            crate::domain::models::MemoryType::Pattern
        } else {
            crate::domain::models::MemoryType::Error
        };

        let mut memory = crate::domain::models::Memory::episodic(idempotency_key, content)
            .with_namespace("convergence")
            .with_type(memory_type)
            .with_source("convergence_engine")
            .with_task(task_id);

        // Add goal context if available
        if let Some(goal_id) = event.goal_id {
            memory = memory.with_goal(goal_id);
        }

        // Tag with outcome and complexity for future queries
        memory = memory
            .with_tag(format!("outcome:{}", outcome))
            .with_tag(format!("complexity:{}", complexity))
            .with_tag(format!("agent:{}", agent_type));

        // Store custom metadata for machine consumption
        memory.metadata.custom.insert(
            "total_iterations".to_string(),
            serde_json::Value::Number(serde_json::Number::from(total_iterations)),
        );
        memory.metadata.custom.insert(
            "total_tokens".to_string(),
            serde_json::Value::Number(serde_json::Number::from(total_tokens)),
        );
        memory.metadata.custom.insert(
            "final_convergence_level".to_string(),
            serde_json::json!(final_convergence_level),
        );
        memory.metadata.custom.insert(
            "trajectory_id".to_string(),
            serde_json::Value::String(trajectory_id.to_string()),
        );
        memory.metadata.relevance = if is_success { 0.8 } else { 0.6 };

        self.memory_repo
            .store(&memory)
            .await
            .map_err(|e| format!("Failed to store convergence memory: {}", e))?;

        tracing::info!(
            task_id = %task_id,
            trajectory_id = %trajectory_id,
            outcome = %outcome,
            "Stored convergence {} memory",
            if is_success { "success" } else { "failure" }
        );

        // Emit a MemoryStored event for downstream processing
        let memory_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Memory,
            goal_id: event.goal_id,
            task_id: Some(task_id),
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::MemoryStored {
                memory_id: memory.id,
                key: memory.key.clone(),
                namespace: memory.namespace.clone(),
                tier: memory.tier.as_str().to_string(),
                memory_type: memory.memory_type.as_str().to_string(),
            },
        };

        Ok(Reaction::EmitEvents(vec![memory_event]))
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

    // ConvergenceMemoryHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_memory_handler_stores_success() {
        use crate::adapters::sqlite::SqliteMemoryRepository;

        let pool = crate::adapters::sqlite::create_migrated_test_pool()
            .await
            .unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool));
        let handler = ConvergenceMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        // Create the task that the event refers to
        let mut task = Task::new("Convergent task for memory");
        let trajectory_id = Uuid::new_v4();
        task.trajectory_id = Some(trajectory_id);
        task.agent_type = Some("coder".to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task_repo.create(&task).await.unwrap();

        // Fire ConvergenceTerminated with "converged" outcome
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
                total_iterations: 3,
                total_tokens: 1500,
                final_convergence_level: 0.95,
            }),
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit a MemoryStored event
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::MemoryStored {
                        namespace,
                        tier,
                        memory_type,
                        ..
                    } => {
                        assert_eq!(namespace, "convergence");
                        assert_eq!(tier, "episodic");
                        assert_eq!(memory_type, "pattern"); // success -> Pattern
                    }
                    other => panic!("Expected MemoryStored, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify memory was actually stored by looking it up via the idempotency key
        let idempotency_key = format!("convergence-outcome:{}", trajectory_id);
        let stored = memory_repo
            .get_by_key(&idempotency_key, "convergence")
            .await
            .unwrap();
        assert!(stored.is_some(), "Memory should have been stored");
        let stored = stored.unwrap();
        assert!(stored.content.contains("converged"));
        assert!(stored.content.contains("iterations: 3"));
        assert!(stored.content.contains("tokens: 1500"));
    }

    // ========================================================================
}

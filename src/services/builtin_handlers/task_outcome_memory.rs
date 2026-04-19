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
// TaskOutcomeMemoryHandler
// ============================================================================

/// When any task completes or fails (via TaskCompleted, TaskCompletedWithResult,
/// or TaskFailed), store an episodic memory entry so that future tasks, agents,
/// and learning loops can reason about historical outcomes.
///
/// This fills the event-chain-integrity gap where orchestrator direct-mode
/// task completions emit `TaskCompleted` but never store episodic memories.
/// Convergent tasks emit `TaskCompletedWithResult`, which is also handled.
/// Failed tasks emit `TaskFailed`, which is captured to satisfy the
/// failure-capture constraint: every task failure must produce a memory event.
///
/// Idempotent: uses `task-outcome:{task_id}` as an idempotency key. If a
/// task somehow emits both `TaskCompleted` and `TaskCompletedWithResult`, the
/// second invocation will find the existing memory and return `Reaction::None`.
pub struct TaskOutcomeMemoryHandler<T: TaskRepository, M: MemoryRepository> {
    task_repo: Arc<T>,
    memory_repo: Arc<M>,
}

impl<T: TaskRepository, M: MemoryRepository> TaskOutcomeMemoryHandler<T, M> {
    pub fn new(task_repo: Arc<T>, memory_repo: Arc<M>) -> Self {
        Self {
            task_repo,
            memory_repo,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, M: MemoryRepository + 'static> EventHandler
    for TaskOutcomeMemoryHandler<T, M>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskOutcomeMemoryHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskCompleted".to_string(),
                    "TaskCompletedWithResult".to_string(),
                    "TaskFailed".to_string(),
                ]),
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
        let task_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Idempotency check: skip if we already stored a memory for this task outcome
        let idempotency_key = format!("task-outcome:{}", task_id);
        let existing = self
            .memory_repo
            .get_by_key(&idempotency_key, "task-outcomes")
            .await
            .map_err(|e| format!("Failed to check existing task outcome memory: {}", e))?;
        if existing.is_some() {
            return Ok(Reaction::None);
        }

        // Load the task to get title, agent_type, execution_mode, complexity, and timing
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task for outcome memory: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let succeeded = task.status == TaskStatus::Complete;
        let outcome_str = if succeeded { "succeeded" } else { "failed" };
        let mode_str = if task.execution_mode.is_direct() {
            "direct"
        } else {
            "convergent"
        };
        let complexity_str = format!("{:?}", task.routing_hints.complexity);
        let agent_type = task
            .agent_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // Compute duration if both timestamps are available
        let duration_secs: Option<i64> = task
            .started_at
            .zip(task.completed_at)
            .map(|(started, completed)| (completed - started).num_seconds());

        // Build a structured summary for the memory content
        let content = if let Some(secs) = duration_secs {
            format!(
                "Task {outcome_str} for {task_id} (\"{title}\"):\n\
                 - execution_mode: {mode_str}\n\
                 - complexity: {complexity_str}\n\
                 - agent_type: {agent_type}\n\
                 - duration_secs: {secs}",
                title = task.title,
            )
        } else {
            format!(
                "Task {outcome_str} for {task_id} (\"{title}\"):\n\
                 - execution_mode: {mode_str}\n\
                 - complexity: {complexity_str}\n\
                 - agent_type: {agent_type}",
                title = task.title,
            )
        };

        // Enrich content with error details when the event carries them
        let content = if let EventPayload::TaskFailed {
            error, retry_count, ..
        } = &event.payload
        {
            format!("{content}\n - error: {error}\n - retry_count: {retry_count}")
        } else {
            content
        };

        // Choose memory type based on outcome
        let memory_type = if succeeded {
            crate::domain::models::MemoryType::Pattern
        } else {
            crate::domain::models::MemoryType::Error
        };

        let mut memory = crate::domain::models::Memory::episodic(idempotency_key.clone(), content)
            .with_namespace("task-outcomes")
            .with_type(memory_type)
            .with_source("task_completion")
            .with_task(task_id);

        // Add goal context if available
        if let Some(goal_id) = event.goal_id {
            memory = memory.with_goal(goal_id);
        }

        // Tag with outcome, mode, complexity, and agent for future queries
        memory = memory
            .with_tag(format!("outcome:{}", outcome_str))
            .with_tag(format!("mode:{}", mode_str))
            .with_tag(format!("complexity:{}", complexity_str))
            .with_tag(format!("agent:{}", agent_type));

        // Store custom metadata for machine consumption
        memory
            .metadata
            .custom
            .insert("succeeded".to_string(), serde_json::Value::Bool(succeeded));
        memory.metadata.custom.insert(
            "execution_mode".to_string(),
            serde_json::Value::String(mode_str.to_string()),
        );
        memory.metadata.custom.insert(
            "complexity".to_string(),
            serde_json::Value::String(complexity_str.clone()),
        );
        memory.metadata.custom.insert(
            "agent_type".to_string(),
            serde_json::Value::String(agent_type.clone()),
        );
        if let Some(secs) = duration_secs {
            memory.metadata.custom.insert(
                "duration_secs".to_string(),
                serde_json::Value::Number(serde_json::Number::from(secs)),
            );
        }

        // Successful outcomes are more relevant than failures for future planning
        memory.metadata.relevance = if succeeded { 0.7 } else { 0.5 };

        self.memory_repo
            .store(&memory)
            .await
            .map_err(|e| format!("Failed to store task outcome memory: {}", e))?;

        tracing::info!(
            task_id = %task_id,
            outcome = outcome_str,
            execution_mode = mode_str,
            complexity = %complexity_str,
            "Stored episodic task outcome memory"
        );

        // Emit a MemoryStored event for downstream processing (evolution loop, etc.)
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

    // TaskOutcomeMemoryHandler tests
    // ========================================================================

    async fn setup_task_and_memory_repos() -> (
        Arc<SqliteTaskRepository>,
        Arc<crate::adapters::sqlite::SqliteMemoryRepository>,
    ) {
        use crate::adapters::sqlite::SqliteMemoryRepository;
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool));
        (task_repo, memory_repo)
    }

    #[tokio::test]
    async fn test_task_outcome_memory_handler_stores_on_task_completed() {
        let (task_repo, memory_repo) = setup_task_and_memory_repos().await;
        let handler = TaskOutcomeMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        // Create a completed task
        let mut task = Task::new("Complete my implementation");
        task.agent_type = Some("coder".to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Complete).unwrap();
        task_repo.create(&task).await.unwrap();

        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: task.id,
                tokens_used: 500,
            },
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
                        assert_eq!(namespace, "task-outcomes");
                        assert_eq!(tier, "episodic");
                        assert_eq!(memory_type, "pattern"); // success -> Pattern
                    }
                    other => panic!("Expected MemoryStored, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify memory was stored with correct idempotency key
        let key = format!("task-outcome:{}", task.id);
        let stored = memory_repo.get_by_key(&key, "task-outcomes").await.unwrap();
        assert!(stored.is_some(), "Memory should have been stored");
        let stored = stored.unwrap();
        assert!(stored.content.contains("succeeded"));
        assert!(stored.content.contains("coder"));
    }

    #[tokio::test]
    async fn test_task_outcome_memory_handler_idempotent() {
        let (task_repo, memory_repo) = setup_task_and_memory_repos().await;
        let handler = TaskOutcomeMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        // Create a completed task
        let mut task = Task::new("Idempotency test task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Complete).unwrap();
        task_repo.create(&task).await.unwrap();

        let make_event = |task_id: uuid::Uuid| UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id,
                tokens_used: 100,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // First call stores the memory
        let reaction1 = handler.handle(&make_event(task.id), &ctx).await.unwrap();
        assert!(
            matches!(reaction1, Reaction::EmitEvents(_)),
            "First call should store memory"
        );

        // Second call (idempotency) should return None, no second store
        let reaction2 = handler.handle(&make_event(task.id), &ctx).await.unwrap();
        assert!(
            matches!(reaction2, Reaction::None),
            "Second call should be idempotent (Reaction::None)"
        );
    }

    #[tokio::test]
    async fn test_task_outcome_memory_handler_returns_none_if_task_not_found() {
        let (task_repo, memory_repo) = setup_task_and_memory_repos().await;
        let handler = TaskOutcomeMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        let nonexistent_id = uuid::Uuid::new_v4();
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(nonexistent_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: nonexistent_id,
                tokens_used: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Should return None if task not found"
        );
    }

    #[tokio::test]
    async fn test_task_outcome_memory_handler_stores_error_type_for_failed_task() {
        let (task_repo, memory_repo) = setup_task_and_memory_repos().await;
        let handler = TaskOutcomeMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        // Create a failed task
        let mut task = Task::new("Task that will fail");
        task.agent_type = Some("researcher".to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        task_repo.create(&task).await.unwrap();

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
                error: "agent exhausted turns".to_string(),
                retry_count: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit a MemoryStored event with Error type (failed task)
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::MemoryStored { memory_type, .. } => {
                        // failed task -> Error memory type
                        assert_eq!(memory_type, "error");
                    }
                    other => panic!("Expected MemoryStored, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        let key = format!("task-outcome:{}", task.id);
        let stored = memory_repo.get_by_key(&key, "task-outcomes").await.unwrap();
        assert!(
            stored.is_some(),
            "Memory should have been stored for failed task"
        );
        let stored = stored.unwrap();
        assert!(stored.content.contains("failed"));
    }

    #[tokio::test]
    async fn test_task_outcome_memory_handler_captures_task_failed_error_details() {
        let (task_repo, memory_repo) = setup_task_and_memory_repos().await;
        let handler = TaskOutcomeMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        let mut task = Task::new("Task with detailed failure");
        task.agent_type = Some("implementer".to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        task_repo.create(&task).await.unwrap();

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
                error: "compilation failed: unresolved import".to_string(),
                retry_count: 2,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::MemoryStored { memory_type, .. } => {
                        assert_eq!(memory_type, "error");
                    }
                    other => panic!("Expected MemoryStored, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        let key = format!("task-outcome:{}", task.id);
        let stored = memory_repo.get_by_key(&key, "task-outcomes").await.unwrap();
        assert!(
            stored.is_some(),
            "Memory should have been stored for failed task"
        );
        let stored = stored.unwrap();
        assert!(
            stored.content.contains("failed"),
            "Content should indicate failure"
        );
        assert!(
            stored.content.contains("compilation failed"),
            "Content should include error message"
        );
        assert!(
            stored.content.contains("retry_count: 2"),
            "Content should include retry count"
        );
    }

    #[tokio::test]
    async fn test_task_outcome_memory_handler_ignores_non_task_events() {
        let (task_repo, memory_repo) = setup_task_and_memory_repos().await;
        let handler = TaskOutcomeMemoryHandler::new(task_repo.clone(), memory_repo.clone());

        // Fire a non-task event (TaskFailed should still work, but let's use a different payload)
        let task_id = uuid::Uuid::new_v4();
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskReady {
                task_id,
                task_title: "some task".to_string(),
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Handler should ignore non-TaskCompleted/TaskCompletedWithResult events"
        );
    }

    // ========================================================================
}

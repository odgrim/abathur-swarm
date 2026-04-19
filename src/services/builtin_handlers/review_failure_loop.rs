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
// ReviewFailureLoopHandler
// ============================================================================

/// When a review task fails, loop back by creating a new plan → implement → review
/// cycle that incorporates the review feedback. Bounded by `max_review_iterations`.
///
/// Runs at HIGH priority so it can set the `review_loop_active` flag before the
/// NORMAL-priority `TaskFailedRetryHandler` sees the event.
pub struct ReviewFailureLoopHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    max_review_iterations: u32,
}

impl<T: TaskRepository> ReviewFailureLoopHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        max_review_iterations: u32,
    ) -> Self {
        Self {
            task_repo,
            command_bus,
            max_review_iterations,
        }
    }

    /// Check whether a task is a review task based on agent_type or title.
    fn is_review_task(task: &Task) -> bool {
        if let Some(ref agent_type) = task.agent_type
            && agent_type == "code-reviewer"
        {
            return true;
        }
        task.title.starts_with("Review")
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ReviewFailureLoopHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ReviewFailureLoopHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string()]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        use crate::domain::models::{TaskContext, TaskPriority, TaskSource};
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        let task_id = match &event.payload {
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Fetch the failed task
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("ReviewFailureLoopHandler: failed to get task: {}", e))?
            .ok_or_else(|| format!("ReviewFailureLoopHandler: task {} not found", task_id))?;

        // Only handle review tasks
        if !Self::is_review_task(&task) {
            return Ok(Reaction::None);
        }

        // Skip workflow phase subtasks — the workflow engine handles rework via
        // verification retries and gate escalation, not the review loop handler.
        if task.is_workflow_phase_subtask() {
            return Ok(Reaction::None);
        }

        // Idempotency: skip if already handled
        if task.has_review_loop_active_flag() {
            return Ok(Reaction::None);
        }

        // Check iteration count
        let current_iteration = task.review_iteration().unwrap_or(1) as u32;

        if current_iteration >= self.max_review_iterations {
            tracing::info!(
                "ReviewFailureLoopHandler: task {} at iteration {}/{}, deferring to normal failure handling",
                task_id,
                current_iteration,
                self.max_review_iterations,
            );
            return Ok(Reaction::None);
        }

        // Set the review_loop_active flag to prevent the retry handler from acting
        let mut flagged = task.clone();
        flagged.set_review_loop_active(true);
        self.task_repo
            .update(&flagged)
            .await
            .map_err(|e| format!("ReviewFailureLoopHandler: failed to flag task: {}", e))?;

        let next_iteration = current_iteration + 1;
        let parent_id = task.parent_id;

        // Build review feedback from the task description + error
        let review_feedback = format!(
            "Previous review (iteration {}) failed. Task description:\n{}\n\nReview the feedback above and produce a revised implementation.",
            current_iteration, task.description,
        );

        // Collect the original implementation task IDs from depends_on
        let original_impl_deps = task.depends_on.clone();

        // --- Create re-plan task ---
        let replan_id = uuid::Uuid::new_v4();
        let mut replan_context = TaskContext {
            input: review_feedback.clone(),
            ..TaskContext::default()
        };
        replan_context.custom.insert(
            crate::domain::models::task::KEY_REVIEW_ITERATION.to_string(),
            serde_json::json!(next_iteration),
        );
        // `review_feedback` is rare (1-site, new-task context only) — stays untyped.
        replan_context.custom.insert(
            "review_feedback".to_string(),
            serde_json::json!(task.description),
        );

        let replan_idem = format!("review-loop:plan:{}:{}", task_id, next_iteration);
        let replan_envelope = CommandEnvelope::new(
            CommandSource::EventHandler("ReviewFailureLoopHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(format!("Re-plan (review iteration {})", next_iteration)),
                description: format!(
                    "Re-plan the implementation based on review feedback from iteration {}.\n\n{}",
                    current_iteration, review_feedback,
                ),
                parent_id,
                priority: TaskPriority::High,
                agent_type: None,
                depends_on: original_impl_deps,
                context: Box::new(Some(replan_context)),
                idempotency_key: Some(replan_idem),
                source: TaskSource::System,
                deadline: None,
                task_type: None,
                execution_mode: None,
            }),
        );

        let replan_result = self.command_bus.dispatch(replan_envelope).await;
        let new_plan_task_id = match replan_result {
            Ok(crate::services::command_bus::CommandResult::Task(t)) => t.id,
            Ok(_) => replan_id,
            Err(e) => {
                tracing::warn!(
                    "ReviewFailureLoopHandler: failed to create re-plan task: {}",
                    e
                );
                return Ok(Reaction::None);
            }
        };

        // --- Create re-implement task ---
        let mut reimpl_context = TaskContext::default();
        reimpl_context.custom.insert(
            crate::domain::models::task::KEY_REVIEW_ITERATION.to_string(),
            serde_json::json!(next_iteration),
        );

        let reimpl_idem = format!("review-loop:impl:{}:{}", task_id, next_iteration);
        let reimpl_envelope = CommandEnvelope::new(
            CommandSource::EventHandler("ReviewFailureLoopHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(format!(
                    "Re-implement (review iteration {})",
                    next_iteration
                )),
                description: format!(
                    "Implement the revised plan from review iteration {}.",
                    next_iteration,
                ),
                parent_id,
                priority: TaskPriority::High,
                agent_type: None,
                depends_on: vec![new_plan_task_id],
                context: Box::new(Some(reimpl_context)),
                idempotency_key: Some(reimpl_idem),
                source: TaskSource::System,
                deadline: None,
                task_type: None,
                execution_mode: None,
            }),
        );

        let reimpl_result = self.command_bus.dispatch(reimpl_envelope).await;
        let new_impl_task_id = match reimpl_result {
            Ok(crate::services::command_bus::CommandResult::Task(t)) => t.id,
            Ok(_) => uuid::Uuid::new_v4(),
            Err(e) => {
                tracing::warn!(
                    "ReviewFailureLoopHandler: failed to create re-implement task: {}",
                    e
                );
                return Ok(Reaction::None);
            }
        };

        // --- Create re-review task ---
        let mut rereview_context = TaskContext::default();
        rereview_context.custom.insert(
            crate::domain::models::task::KEY_REVIEW_ITERATION.to_string(),
            serde_json::json!(next_iteration),
        );

        let rereview_idem = format!("review-loop:review:{}:{}", task_id, next_iteration);
        let rereview_envelope = CommandEnvelope::new(
            CommandSource::EventHandler("ReviewFailureLoopHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(format!("Review (review iteration {})", next_iteration)),
                description: format!(
                    "Review the re-implementation from iteration {}. Check for correctness, edge cases, and adherence to the revised plan.",
                    next_iteration,
                ),
                parent_id,
                priority: TaskPriority::High,
                agent_type: Some("code-reviewer".to_string()),
                depends_on: vec![new_impl_task_id],
                context: Box::new(Some(rereview_context)),
                idempotency_key: Some(rereview_idem),
                source: TaskSource::System,
                deadline: None,
                task_type: None,
                execution_mode: None,
            }),
        );

        let rereview_task_id = match self.command_bus.dispatch(rereview_envelope).await {
            Ok(crate::services::command_bus::CommandResult::Task(t)) => {
                // Store the successor review task ID on the failing task so the parent
                // orchestrating agent can follow the chain without spawning its own fix.
                let mut with_successor = flagged.clone();
                with_successor.context.custom.insert(
                    "review_loop_successor".to_string(),
                    serde_json::json!(t.id.to_string()),
                );
                if let Err(e) = self.task_repo.update(&with_successor).await {
                    tracing::warn!(
                        "ReviewFailureLoopHandler: failed to store successor task ID on task {}: {}",
                        task_id,
                        e
                    );
                }
                t.id
            }
            Ok(_) => {
                tracing::warn!(
                    "ReviewFailureLoopHandler: unexpected result type for re-review task"
                );
                uuid::Uuid::new_v4()
            }
            Err(e) => {
                tracing::warn!(
                    "ReviewFailureLoopHandler: failed to create re-review task: {}",
                    e
                );
                uuid::Uuid::new_v4()
            }
        };

        tracing::info!(
            "ReviewFailureLoopHandler: created review loop-back iteration {} for task {}",
            next_iteration,
            task_id,
        );

        // Emit ReviewLoopTriggered event
        let loop_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: event.goal_id,
            task_id: Some(task_id),
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ReviewLoopTriggered {
                failed_review_task_id: task_id,
                iteration: next_iteration,
                max_iterations: self.max_review_iterations,
                new_plan_task_id,
                new_review_task_id: rereview_task_id,
            },
        };

        Ok(Reaction::EmitEvents(vec![loop_event]))
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

    // ReviewFailureLoopHandler tests
    // ========================================================================

    async fn setup_command_bus(
        repo: Arc<SqliteTaskRepository>,
    ) -> Arc<crate::services::command_bus::CommandBus> {
        use crate::domain::ports::NullMemoryRepository;
        use crate::services::goal_service::GoalService;
        use crate::services::memory_service::MemoryService;
        use crate::services::task_service::TaskService;

        let pool = create_migrated_test_pool().await.unwrap();
        let goal_repo =
            Arc::new(crate::adapters::sqlite::goal_repository::SqliteGoalRepository::new(pool));
        let task_service = Arc::new(TaskService::new(repo));
        let goal_service = Arc::new(GoalService::new(goal_repo));
        let memory_service = Arc::new(MemoryService::new(Arc::new(NullMemoryRepository::new())));
        let event_bus = Arc::new(crate::services::EventBus::new(
            crate::services::EventBusConfig {
                persist_events: false,
                ..Default::default()
            },
        ));
        Arc::new(crate::services::command_bus::CommandBus::new(
            task_service,
            goal_service,
            memory_service,
            event_bus,
        ))
    }

    fn make_task_failed_event(task_id: Uuid, retry_count: u32) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id,
                error: "review found issues".to_string(),
                retry_count,
            },
        }
    }

    #[tokio::test]
    async fn test_review_failure_loop_creates_three_tasks() {
        let repo = setup_task_repo().await;
        let command_bus = setup_command_bus(repo.clone()).await;
        let handler = ReviewFailureLoopHandler::new(repo.clone(), command_bus, 3);

        // Create a review task that has failed
        let mut review_task = Task::new("Review implementation");
        review_task.agent_type = Some("code-reviewer".to_string());
        review_task.transition_to(TaskStatus::Ready).unwrap();
        review_task.transition_to(TaskStatus::Running).unwrap();
        review_task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&review_task).await.unwrap();

        let event = make_task_failed_event(review_task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit ReviewLoopTriggered event
        let new_review_task_id = match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::ReviewLoopTriggered {
                        failed_review_task_id,
                        iteration,
                        max_iterations,
                        new_review_task_id,
                        ..
                    } => {
                        assert_eq!(*failed_review_task_id, review_task.id);
                        assert_eq!(*iteration, 2); // next iteration
                        assert_eq!(*max_iterations, 3);
                        *new_review_task_id
                    }
                    other => panic!("Expected ReviewLoopTriggered, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        };

        // Verify the review_loop_active flag was set and review_loop_successor points to
        // the newly created re-review task
        let updated = repo.get(review_task.id).await.unwrap().unwrap();
        assert!(
            updated.review_loop_active(),
            "review_loop_active flag should be set to true"
        );
        let successor_val = updated
            .context
            .custom
            .get("review_loop_successor")
            .expect("review_loop_successor should be set on the original task");
        let successor_str = successor_val
            .as_str()
            .expect("review_loop_successor should be a string");
        let successor_id: uuid::Uuid = successor_str
            .parse()
            .expect("review_loop_successor should be a valid UUID");
        assert_eq!(
            successor_id, new_review_task_id,
            "review_loop_successor must match new_review_task_id in the event"
        );
    }

    #[tokio::test]
    async fn test_review_failure_loop_max_iterations_skips() {
        let repo = setup_task_repo().await;
        let command_bus = setup_command_bus(repo.clone()).await;
        let handler = ReviewFailureLoopHandler::new(repo.clone(), command_bus, 3);

        // Create a review task at iteration 3 (== max)
        let mut review_task = Task::new("Review implementation");
        review_task.agent_type = Some("code-reviewer".to_string());
        review_task.set_review_iteration(3);
        review_task.transition_to(TaskStatus::Ready).unwrap();
        review_task.transition_to(TaskStatus::Running).unwrap();
        review_task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&review_task).await.unwrap();

        let event = make_task_failed_event(review_task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should return None — max iterations reached
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_review_failure_loop_non_review_task_skips() {
        let repo = setup_task_repo().await;
        let command_bus = setup_command_bus(repo.clone()).await;
        let handler = ReviewFailureLoopHandler::new(repo.clone(), command_bus, 3);

        // Create a non-review task
        let mut task = Task::new("Implement feature X");
        task.agent_type = Some("implementer".to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_task_failed_event(task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_review_failure_loop_idempotency() {
        let repo = setup_task_repo().await;
        let command_bus = setup_command_bus(repo.clone()).await;
        let handler = ReviewFailureLoopHandler::new(repo.clone(), command_bus, 3);

        // Create a review task with review_loop_active already set
        let mut review_task = Task::new("Review implementation");
        review_task.agent_type = Some("code-reviewer".to_string());
        review_task.set_review_loop_active(true);
        review_task.transition_to(TaskStatus::Ready).unwrap();
        review_task.transition_to(TaskStatus::Running).unwrap();
        review_task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&review_task).await.unwrap();

        let event = make_task_failed_event(review_task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should skip — already handled
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_retry_handler_skips_review_loop_active() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 3);

        // Create a review task with review_loop_active flag
        let mut task = Task::new("Review implementation");
        task.agent_type = Some("code-reviewer".to_string());
        task.set_review_loop_active(true);
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_task_failed_event(task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should skip — review_loop_active flag prevents retry
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_retry_handler_skips_review_iteration_tasks() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 3);

        // Create a task that is part of a review loop chain (has review_iteration)
        let mut task = Task::new("Re-implement (review iteration 2)");
        task.set_review_iteration(2);
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_task_failed_event(task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should skip — review_iteration flag prevents independent retry
        assert!(
            matches!(reaction, Reaction::None),
            "TaskFailedRetryHandler must not retry tasks with review_iteration set"
        );
    }

    #[tokio::test]
    async fn test_retry_handler_circuit_breaks_consecutive_budget_failures() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 10); // high max_retries to not hit that limit

        // Create a task that has already seen 2 consecutive budget failures
        let mut task = Task::new("Some long-running task");
        task.context.custom.insert(
            "consecutive_budget_failures".to_string(),
            serde_json::json!(2u64),
        );
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        // Fire a third error_max_turns failure — consecutive becomes 3, triggering circuit-break
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
                error: "error_max_turns: limit reached".to_string(),
                retry_count: 2,
            },
        };
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should circuit-break and not retry
        assert!(
            matches!(reaction, Reaction::None),
            "TaskFailedRetryHandler must circuit-break after 3 consecutive budget failures"
        );
    }

    #[tokio::test]
    async fn test_review_failure_loop_title_prefix_detection() {
        let repo = setup_task_repo().await;
        let command_bus = setup_command_bus(repo.clone()).await;
        let handler = ReviewFailureLoopHandler::new(repo.clone(), command_bus, 3);

        // Create a review task identified by title prefix (no agent_type)
        let mut review_task = Task::new("placeholder");
        review_task.title = "Review the implementation for correctness".to_string();
        review_task.transition_to(TaskStatus::Ready).unwrap();
        review_task.transition_to(TaskStatus::Running).unwrap();
        review_task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&review_task).await.unwrap();

        let event = make_task_failed_event(review_task.id, 0);
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should trigger — title starts with "Review"
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                assert!(matches!(
                    events[0].payload,
                    EventPayload::ReviewLoopTriggered { .. }
                ));
            }
            Reaction::None => panic!("Expected EmitEvents for title-based review detection"),
        }
    }

    // ========================================================================
    // SystemStallDetectorHandler tests
}

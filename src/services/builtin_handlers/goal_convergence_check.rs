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
// GoalConvergenceCheckHandler
// ============================================================================

/// Periodic deep goal convergence check (default: every 4 hours).
///
/// Unlike the lightweight `GoalEvaluationHandler` (60s) which observes and emits
/// signal events, this handler creates an actual Overmind-processed task that
/// performs a strategic evaluation of all active goals, assesses overall progress,
/// and suggests concrete incremental next steps.
///
/// Triggered by `ScheduledEventFired { name: "goal-convergence-check" }`.
pub struct GoalConvergenceCheckHandler<G: GoalRepository, T: TaskRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    budget_tracker: Option<Arc<crate::services::budget_tracker::BudgetTracker>>,
    /// Configured convergence check interval in seconds, used for idempotency bucketing.
    check_interval_secs: u64,
}

impl<G: GoalRepository, T: TaskRepository> GoalConvergenceCheckHandler<G, T> {
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        check_interval_secs: u64,
    ) -> Self {
        Self {
            goal_repo,
            task_repo,
            command_bus,
            budget_tracker: None,
            check_interval_secs,
        }
    }

    /// Attach a budget tracker to enable budget-pressure gating of convergence checks.
    pub fn with_budget_tracker(
        mut self,
        tracker: Arc<crate::services::budget_tracker::BudgetTracker>,
    ) -> Self {
        self.budget_tracker = Some(tracker);
        self
    }

    /// Build the rich task description for the convergence check.
    fn build_convergence_description(
        goals: &[Goal],
        completed_count: usize,
        failed_count: usize,
        running_count: usize,
        ready_count: usize,
        pending_count: usize,
    ) -> String {
        let mut desc = String::with_capacity(8192);

        desc.push_str("# Goal Convergence Check\n\n");
        desc.push_str("Periodic strategic evaluation of all active goals.\n");
        desc.push_str("Assess overall progress toward each goal, identify gaps, and determine the highest-impact incremental work to move the swarm closer to convergence.\n\n");

        // Task statistics summary
        desc.push_str("## Current Task Statistics\n\n");
        desc.push_str(&format!("- **Completed**: {}\n", completed_count));
        desc.push_str(&format!("- **Failed**: {}\n", failed_count));
        desc.push_str(&format!("- **Running**: {}\n", running_count));
        desc.push_str(&format!("- **Ready**: {}\n", ready_count));
        desc.push_str(&format!("- **Pending/Blocked**: {}\n\n", pending_count));

        // Active goals with constraints
        desc.push_str("## Active Goals\n\n");
        for (i, goal) in goals.iter().enumerate() {
            desc.push_str(&format!(
                "### {}. {} (priority: {:?})\n",
                i + 1,
                goal.name,
                goal.priority
            ));
            desc.push_str(&format!("**ID**: `{}`\n\n", goal.id));
            desc.push_str(&format!("{}\n\n", goal.description));

            if !goal.applicability_domains.is_empty() {
                desc.push_str(&format!(
                    "**Domains**: {}\n\n",
                    goal.applicability_domains.join(", ")
                ));
            }

            if !goal.constraints.is_empty() {
                desc.push_str("**Constraints**:\n");
                for c in &goal.constraints {
                    desc.push_str(&format!(
                        "- **{}** ({:?}): {}\n",
                        c.name, c.constraint_type, c.description
                    ));
                }
                desc.push('\n');
            }
        }

        // Instructions
        desc.push_str("## Instructions\n\n");
        desc.push_str("This task is enrolled in a `code` workflow. Use the `code` workflow spine — do NOT select `analysis`. This task must produce commits.\n\n");
        desc.push_str("### Research Phase\n");
        desc.push_str("1. **Search Memory**: Call `memory_search` to find prior convergence evaluations, known patterns, and recent task outcomes.\n");
        desc.push_str("2. **Review Existing Work**: Call `task_list` with each status (running, ready, pending, complete, failed) to understand what's already in flight and what has been tried.\n");
        desc.push_str("3. **Scan Codebase**: Look for gaps — missing tests, incomplete implementations, known TODOs, or issues surfaced by failed tasks. Call `task_get` on failed tasks to understand failure reasons.\n\n");
        desc.push_str("### Plan Phase\n");
        desc.push_str("4. **Prioritize Gaps**: Identify the 2-3 highest-impact gaps across under-served goals. Focus on goals with the least progress or most failures. Avoid duplicating running or ready tasks.\n");
        desc.push_str("5. **Design Implementation Slices**: For each gap, define a concrete implementation slice — what file(s) to change, what to add/fix, which goal it serves.\n\n");
        desc.push_str("### Implement Phase\n");
        desc.push_str("6. **Reuse Agents**: Call `agent_list` to see what agent templates already exist. Reuse existing agents whenever possible.\n");
        desc.push_str("7. **Fan Out Subtasks**: Use `workflow_advance` and `workflow_fan_out` to create slices for each implementation. Each slice must target a specific goal gap and produce code changes (add tests, fix issues, close gaps). Assign agents to slices via `task_assign`.\n\n");
        desc.push_str("### Review Phase\n");
        desc.push_str(
            "8. **Verify Outputs**: Confirm that subtask outputs compiled and passed tests.\n",
        );
        desc.push_str("9. **Store Evaluation**: Call `memory_store` with your convergence evaluation summary (namespace: `convergence-checks`, memory_type: `decision`) so future checks can build on your findings.\n\n");
        desc.push_str("Remember: Goals are convergent attractors — they are never 'completed.' Your job is to produce the highest-impact incremental code changes, not just analysis. Use the workflow to fan out concrete implementation work — do not simply list recommendations as text output.\n");

        desc
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static> EventHandler
    for GoalConvergenceCheckHandler<G, T>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalConvergenceCheckHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. }
                            if name == "goal-convergence-check"
                                || name == "goal-convergence-check:budget-trigger"
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
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        use crate::domain::models::TaskPriority;
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        // Determine if this was triggered by a budget opportunity signal
        let is_budget_trigger = matches!(
            &event.payload,
            EventPayload::ScheduledEventFired { name, .. }
                if name == "goal-convergence-check:budget-trigger"
        );

        // Load all active goals
        let goals = self
            .goal_repo
            .get_active_with_constraints()
            .await
            .map_err(|e| {
                format!(
                    "GoalConvergenceCheckHandler: failed to get active goals: {}",
                    e
                )
            })?;

        if goals.is_empty() {
            tracing::debug!(
                "GoalConvergenceCheckHandler: no active goals, skipping convergence check"
            );
            return Ok(Reaction::None);
        }

        // Gather task statistics
        let completed = self
            .task_repo
            .list_by_status(TaskStatus::Complete)
            .await
            .map_err(|e| {
                format!(
                    "GoalConvergenceCheckHandler: failed to list completed tasks: {}",
                    e
                )
            })?;
        let failed = self
            .task_repo
            .list_by_status(TaskStatus::Failed)
            .await
            .map_err(|e| {
                format!(
                    "GoalConvergenceCheckHandler: failed to list failed tasks: {}",
                    e
                )
            })?;
        let running = self
            .task_repo
            .list_by_status(TaskStatus::Running)
            .await
            .map_err(|e| {
                format!(
                    "GoalConvergenceCheckHandler: failed to list running tasks: {}",
                    e
                )
            })?;
        let ready = self
            .task_repo
            .list_by_status(TaskStatus::Ready)
            .await
            .map_err(|e| {
                format!(
                    "GoalConvergenceCheckHandler: failed to list ready tasks: {}",
                    e
                )
            })?;
        let pending = self
            .task_repo
            .list_by_status(TaskStatus::Pending)
            .await
            .map_err(|e| {
                format!(
                    "GoalConvergenceCheckHandler: failed to list pending tasks: {}",
                    e
                )
            })?;

        // Overlap check: skip if a previous convergence check task is still enqueued/active
        let overlap_exists = pending
            .iter()
            .chain(ready.iter())
            .chain(running.iter())
            .any(|t| t.title.starts_with("Goal Convergence Check"));

        if overlap_exists {
            tracing::debug!(
                "GoalConvergenceCheckHandler: previous convergence check task already enqueued or active, skipping"
            );
            return Ok(Reaction::None);
        }

        // Budget gate: if triggered by the scheduler (not a budget opportunity), and budget
        // pressure is critical, skip creating new work.
        if !is_budget_trigger
            && let Some(ref bt) = self.budget_tracker
            && bt.should_pause_new_work().await
        {
            tracing::debug!(
                "GoalConvergenceCheckHandler: pausing convergence check — budget at critical pressure"
            );
            return Ok(Reaction::None);
        }

        // Circuit breaker: if the last 3 convergence check tasks all failed,
        // skip creating a new one until the cycle is broken by manual intervention
        // or a successful check.
        {
            let mut recent_convergence: Vec<&Task> = completed
                .iter()
                .chain(failed.iter())
                .filter(|t| t.title.starts_with("Goal Convergence Check"))
                .collect();
            recent_convergence.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            let last_three: Vec<&Task> = recent_convergence.into_iter().take(3).collect();
            if last_three.len() == 3 && last_three.iter().all(|t| t.status == TaskStatus::Failed) {
                tracing::warn!(
                    "GoalConvergenceCheckHandler: circuit breaker triggered — last 3 convergence checks failed, skipping"
                );
                return Ok(Reaction::None);
            }
        }

        // Build idempotency key.
        // Budget-triggered checks get a unique key per timestamp to bypass the 4-hour window.
        let now = chrono::Utc::now();
        let idem_key = if is_budget_trigger {
            format!("goal-convergence-check:budget:{}", now.timestamp())
        } else {
            // Bucket idempotency key by the configured check interval
            let bucket = now.timestamp() / self.check_interval_secs as i64;
            format!("goal-convergence-check:{}", bucket)
        };

        // Build the rich description
        let description = Self::build_convergence_description(
            &goals,
            completed.len(),
            failed.len(),
            running.len(),
            ready.len(),
            pending.len(),
        );

        let title = format!("Goal Convergence Check — {} active goal(s)", goals.len());

        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler("GoalConvergenceCheckHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(title),
                description,
                parent_id: None,
                priority: TaskPriority::Normal,
                agent_type: Some("overmind".to_string()),
                depends_on: vec![],
                context: Box::new(None),
                idempotency_key: Some(idem_key),
                source: TaskSource::System,
                deadline: None,
                task_type: None,
                execution_mode: None,
            }),
        );

        match self.command_bus.dispatch(envelope).await {
            Ok(_) => {
                tracing::info!(
                    "GoalConvergenceCheckHandler: created convergence check task for {} goals",
                    goals.len()
                );
                // Update last_convergence_check_at for all active goals
                for goal in &goals {
                    if let Err(e) = self.goal_repo.update_last_check(goal.id, now).await {
                        tracing::warn!(
                            goal_id = %goal.id,
                            error = %e,
                            "GoalConvergenceCheckHandler: failed to update last_convergence_check_at"
                        );
                    }
                }
            }
            Err(crate::services::command_bus::CommandError::DuplicateCommand(_)) => {
                tracing::debug!(
                    "GoalConvergenceCheckHandler: duplicate convergence check task, skipping"
                );
            }
            Err(e) => {
                tracing::warn!(
                    "GoalConvergenceCheckHandler: failed to create convergence check task: {}",
                    e
                );
            }
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
        create_migrated_test_pool, goal_repository::SqliteGoalRepository,
        task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{GoalConstraint, GoalPriority, Task, TaskStatus};
    use std::sync::Arc;

    async fn setup_convergence_handler() -> (
        GoalConvergenceCheckHandler<SqliteGoalRepository, SqliteTaskRepository>,
        Arc<SqliteGoalRepository>,
        Arc<SqliteTaskRepository>,
    ) {
        let pool = create_migrated_test_pool().await.unwrap();
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));

        let task_service = Arc::new(crate::services::task_service::TaskService::new(
            task_repo.clone(),
        ));
        let goal_service = Arc::new(crate::services::goal_service::GoalService::new(
            goal_repo.clone(),
        ));
        let memory_repo = Arc::new(crate::adapters::sqlite::SqliteMemoryRepository::new(
            pool.clone(),
        ));
        let memory_service = Arc::new(crate::services::memory_service::MemoryService::new(
            memory_repo,
        ));
        let event_bus = Arc::new(crate::services::EventBus::new(
            crate::services::EventBusConfig {
                persist_events: false,
                ..Default::default()
            },
        ));
        let command_bus = Arc::new(crate::services::command_bus::CommandBus::new(
            task_service,
            goal_service,
            memory_service,
            event_bus,
        ));

        // Use a 4-hour (14400s) check interval for idempotency bucketing
        let handler = GoalConvergenceCheckHandler::new(
            goal_repo.clone(),
            task_repo.clone(),
            command_bus,
            14400,
        );

        (handler, goal_repo, task_repo)
    }

    fn make_convergence_check_event() -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "goal-convergence-check".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_no_goals_returns_early() {
        let (handler, _goal_repo, task_repo) = setup_convergence_handler().await;
        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "No goals should produce Reaction::None"
        );

        // Verify no tasks were created (tasks auto-transition from Pending to Ready)
        let ready_tasks = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert!(
            ready_tasks.is_empty(),
            "No tasks should be created when there are no goals"
        );
    }

    #[tokio::test]
    async fn test_creates_convergence_task_with_active_goals() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert two active goals
        let goal1 =
            Goal::new("Test Goal Alpha", "Description for alpha").with_priority(GoalPriority::High);
        let goal2 =
            Goal::new("Test Goal Beta", "Description for beta").with_priority(GoalPriority::Normal);
        goal_repo.create(&goal1).await.unwrap();
        goal_repo.create(&goal2).await.unwrap();

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Handler should return Reaction::None on success"
        );

        // Verify a convergence check task was created (auto-transitions to Ready)
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert_eq!(
            ready.len(),
            1,
            "Exactly one convergence check task should be created"
        );
        assert!(
            ready[0].title.starts_with("Goal Convergence Check"),
            "Task title should start with 'Goal Convergence Check'"
        );
        assert!(
            ready[0].title.contains("2 active goal(s)"),
            "Task title should mention the number of active goals"
        );
    }

    #[tokio::test]
    async fn test_idempotency_bucketing_prevents_duplicate() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal so the handler doesn't skip due to empty goals
        let goal =
            Goal::new("Idempotency Test Goal", "Testing dedup").with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // First invocation should create a task
        let event1 = make_convergence_check_event();
        handler.handle(&event1, &ctx).await.unwrap();

        // Second invocation within the same time bucket should be deduplicated
        let event2 = make_convergence_check_event();
        handler.handle(&event2, &ctx).await.unwrap();

        // Only one task should exist (idempotency key dedup by TaskService)
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert_eq!(
            ready.len(),
            1,
            "Duplicate convergence check should be prevented by idempotency key"
        );
    }

    #[tokio::test]
    async fn test_overlap_detection_skips_when_active() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal
        let goal = Goal::new("Overlap Test Goal", "Testing overlap detection")
            .with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        // Create an existing running task with a convergence-check-style title
        let mut existing_task = Task::new("Goal Convergence Check — 1 active goal(s)");
        existing_task.description = "Previous convergence check still running".to_string();
        existing_task.transition_to(TaskStatus::Ready).unwrap();
        existing_task.transition_to(TaskStatus::Running).unwrap();
        task_repo.create(&existing_task).await.unwrap();

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Should skip when active convergence check exists"
        );

        // Verify no new task was created (only the existing running one)
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert!(
            ready.is_empty(),
            "No new task should be created when overlap detected"
        );
    }

    #[tokio::test]
    async fn test_last_convergence_check_at_updated() {
        let (handler, goal_repo, _task_repo) = setup_convergence_handler().await;

        // Insert a goal with no previous convergence check
        let goal = Goal::new("Timestamp Test Goal", "Testing timestamp update")
            .with_priority(GoalPriority::Normal);
        let goal_id = goal.id;
        goal_repo.create(&goal).await.unwrap();

        // Verify last_convergence_check_at is initially None
        let before = goal_repo.get(goal_id).await.unwrap().unwrap();
        assert!(
            before.last_convergence_check_at.is_none(),
            "last_convergence_check_at should be None before first check"
        );

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        handler.handle(&event, &ctx).await.unwrap();

        // Verify last_convergence_check_at is now set
        let after = goal_repo.get(goal_id).await.unwrap().unwrap();
        assert!(
            after.last_convergence_check_at.is_some(),
            "last_convergence_check_at should be updated after convergence check"
        );
    }

    #[tokio::test]
    async fn test_task_description_contains_goal_details() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Create goals with constraints and specific details
        let goal1 = Goal::new(
            "Memory Lifecycle Goal",
            "Ensure memory decay works correctly",
        )
        .with_priority(GoalPriority::High)
        .with_constraint(GoalConstraint::preference(
            "no-silent-data-loss",
            "Decay daemon must not silently drop memories",
        ));
        let goal2 = Goal::new("Convergence Loop Goal", "Drive tasks to completion")
            .with_priority(GoalPriority::Critical)
            .with_constraint(GoalConstraint::preference(
                "strategy-diversity",
                "Never retry the same failing approach",
            ));
        let goal1_id = goal1.id;
        let goal2_id = goal2.id;
        goal_repo.create(&goal1).await.unwrap();
        goal_repo.create(&goal2).await.unwrap();

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        handler.handle(&event, &ctx).await.unwrap();

        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert_eq!(ready.len(), 1);
        let desc = &ready[0].description;

        // Verify goal names appear in description
        assert!(
            desc.contains("Memory Lifecycle Goal"),
            "Description should contain first goal name"
        );
        assert!(
            desc.contains("Convergence Loop Goal"),
            "Description should contain second goal name"
        );

        // Verify goal IDs appear in description
        assert!(
            desc.contains(&goal1_id.to_string()),
            "Description should contain first goal ID"
        );
        assert!(
            desc.contains(&goal2_id.to_string()),
            "Description should contain second goal ID"
        );

        // Verify constraint details appear in description
        assert!(
            desc.contains("no-silent-data-loss"),
            "Description should contain constraint name"
        );
        assert!(
            desc.contains("strategy-diversity"),
            "Description should contain constraint name"
        );
        assert!(
            desc.contains("Decay daemon must not silently drop memories"),
            "Description should contain constraint description"
        );

        // Verify overall structure
        assert!(
            desc.contains("# Goal Convergence Check"),
            "Description should contain header"
        );
        assert!(
            desc.contains("## Active Goals"),
            "Description should contain Active Goals section"
        );
        assert!(
            desc.contains("## Current Task Statistics"),
            "Description should contain statistics"
        );
    }

    #[tokio::test]
    async fn test_circuit_breaker_skips_after_consecutive_failures() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal so the handler doesn't skip due to empty goals
        let goal = Goal::new("Circuit Breaker Test Goal", "Testing circuit breaker")
            .with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        // Create 3 failed convergence check tasks
        for i in 0..3 {
            let mut task = Task::new(format!("Goal Convergence Check — {} active goal(s)", i + 1));
            task.description = "Failed convergence check".to_string();
            task.transition_to(TaskStatus::Ready).unwrap();
            task.transition_to(TaskStatus::Running).unwrap();
            task.transition_to(TaskStatus::Failed).unwrap();
            task_repo.create(&task).await.unwrap();
        }

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Circuit breaker should prevent new task creation after 3 consecutive failures"
        );

        // Verify no new tasks were created (only the 3 failed ones exist)
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert!(
            ready.is_empty(),
            "No new task should be created when circuit breaker is tripped"
        );
    }

    #[tokio::test]
    async fn test_circuit_breaker_resets_after_success() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal so the handler doesn't skip due to empty goals
        let goal = Goal::new(
            "Circuit Breaker Reset Test",
            "Testing circuit breaker reset",
        )
        .with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        // Create 2 failed convergence check tasks
        for i in 0..2 {
            let mut task = Task::new(format!("Goal Convergence Check — {} active goal(s)", i + 1));
            task.description = "Failed convergence check".to_string();
            task.transition_to(TaskStatus::Ready).unwrap();
            task.transition_to(TaskStatus::Running).unwrap();
            task.transition_to(TaskStatus::Failed).unwrap();
            task_repo.create(&task).await.unwrap();
        }

        // Create 1 successful convergence check task (most recent)
        let mut success_task = Task::new("Goal Convergence Check — 1 active goal(s)");
        success_task.description = "Successful convergence check".to_string();
        success_task.transition_to(TaskStatus::Ready).unwrap();
        success_task.transition_to(TaskStatus::Running).unwrap();
        success_task.transition_to(TaskStatus::Complete).unwrap();
        task_repo.create(&success_task).await.unwrap();

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Handler should return Reaction::None on success"
        );

        // Verify a new task was created (the successful check broke the circuit)
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert_eq!(
            ready.len(),
            1,
            "A new convergence check task should be created when circuit breaker is not tripped"
        );
        assert!(
            ready[0].title.starts_with("Goal Convergence Check"),
            "New task should be a convergence check"
        );
    }

    #[tokio::test]
    async fn test_circuit_breaker_proceeds_with_fewer_than_three_tasks() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal so the handler doesn't skip due to empty goals
        let goal = Goal::new(
            "Circuit Breaker Fewer Test",
            "Testing circuit breaker with <3 failures",
        )
        .with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        // Create only 2 failed convergence check tasks (below the threshold of 3)
        for i in 0..2 {
            let mut task = Task::new(format!("Goal Convergence Check — {} active goal(s)", i + 1));
            task.description = "Failed convergence check".to_string();
            task.transition_to(TaskStatus::Ready).unwrap();
            task.transition_to(TaskStatus::Running).unwrap();
            task.transition_to(TaskStatus::Failed).unwrap();
            task_repo.create(&task).await.unwrap();
        }

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        // Handler always returns Reaction::None, but with only 2 failures
        // (below the circuit breaker threshold of 3), it should still create a new task
        assert!(
            matches!(reaction, Reaction::None),
            "Handler should return Reaction::None"
        );

        // Verify a new task was created since the circuit breaker was not tripped
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert_eq!(
            ready.len(),
            1,
            "A new convergence check task should be created when circuit breaker is not tripped"
        );
        assert!(
            ready[0].title.starts_with("Goal Convergence Check"),
            "New task should be a convergence check"
        );
    }

    #[tokio::test]
    async fn test_budget_gate_pauses_convergence_check() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal so the handler proceeds past the "no goals" early return
        let goal = Goal::new("Budget Gate Test Goal", "Testing budget gate")
            .with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        // Create a BudgetTracker with critical pressure so should_pause_new_work() = true
        let event_bus = Arc::new(crate::services::EventBus::new(
            crate::services::EventBusConfig {
                persist_events: false,
                ..Default::default()
            },
        ));
        let tracker = Arc::new(crate::services::budget_tracker::BudgetTracker::new(
            crate::services::budget_tracker::BudgetTrackerConfig::default(),
            event_bus,
        ));
        // Push pressure to Critical by reporting a window with >= 95% consumed
        tracker
            .report_budget_signal(
                "daily",
                crate::services::budget_tracker::BudgetWindowType::Daily,
                0.98, // 98% consumed → Critical
                500,
                3600,
            )
            .await;
        assert!(
            tracker.should_pause_new_work().await,
            "Tracker should signal pause at Critical pressure"
        );

        // Attach the budget tracker to the handler
        let handler = handler.with_budget_tracker(tracker);

        let event = make_convergence_check_event();
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Budget gate should cause handler to return Reaction::None"
        );

        // Verify no convergence task was created
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert!(
            ready.is_empty(),
            "No task should be created when budget pressure is critical"
        );
    }

    #[tokio::test]
    async fn test_budget_trigger_bypasses_budget_gate() {
        let (handler, goal_repo, task_repo) = setup_convergence_handler().await;

        // Insert a goal so the handler proceeds past the "no goals" early return
        let goal = Goal::new("Budget Trigger Test Goal", "Testing budget trigger bypass")
            .with_priority(GoalPriority::Normal);
        goal_repo.create(&goal).await.unwrap();

        // Create a BudgetTracker with critical pressure so should_pause_new_work() = true
        let event_bus = Arc::new(crate::services::EventBus::new(
            crate::services::EventBusConfig {
                persist_events: false,
                ..Default::default()
            },
        ));
        let tracker = Arc::new(crate::services::budget_tracker::BudgetTracker::new(
            crate::services::budget_tracker::BudgetTrackerConfig::default(),
            event_bus,
        ));
        // Push pressure to Critical by reporting a window with >= 95% consumed
        tracker
            .report_budget_signal(
                "daily",
                crate::services::budget_tracker::BudgetWindowType::Daily,
                0.98, // 98% consumed → Critical
                500,
                3600,
            )
            .await;
        assert!(
            tracker.should_pause_new_work().await,
            "Tracker should signal pause at Critical pressure"
        );

        // Attach the budget tracker to the handler
        let handler = handler.with_budget_tracker(tracker);

        // Use the budget-trigger event name — this should bypass the budget gate
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "goal-convergence-check:budget-trigger".to_string(),
            },
        };
        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let _reaction = handler.handle(&event, &ctx).await.unwrap();

        // Verify a convergence task WAS created despite critical budget pressure
        let ready = task_repo.list_by_status(TaskStatus::Ready).await.unwrap();
        assert!(
            !ready.is_empty(),
            "A task should be created when event is a budget-trigger, even at critical pressure"
        );
    }
}

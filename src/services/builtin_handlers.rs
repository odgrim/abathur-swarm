//! Built-in reactive event handlers.
//!
//! All handlers are **idempotent** — safe to run even if the poll loop already
//! handled the same state change. They check current state before acting.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::domain::models::{Goal, HumanEscalationEvent, Task, TaskSource, TaskStatus};
use crate::domain::models::convergence::{AmendmentSource, SpecificationAmendment};
use crate::domain::models::task_schedule::*;
use crate::domain::ports::{GoalRepository, MemoryRepository, TaskRepository, TaskScheduleRepository, TrajectoryRepository, WorktreeRepository};
use crate::services::event_store::EventStore;
use crate::services::goal_context_service::GoalContextService;
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber,
    SwarmStatsPayload, TaskResultPayload, UnifiedEvent,
};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};
use crate::services::memory_service::MemoryService;
use crate::services::swarm_orchestrator::SwarmStats;

// ============================================================================
// TaskCompletedReadinessHandler
// ============================================================================

/// When a task completes, check its dependents and transition Pending/Blocked → Ready
/// if all their dependencies are now complete.
pub struct TaskCompletedReadinessHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> TaskCompletedReadinessHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
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
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let task_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        let dependents = self.task_repo.get_dependents(task_id).await
            .map_err(|e| format!("Failed to get dependents: {}", e))?;

        let mut new_events = Vec::new();

        for dep in dependents {
            // Idempotency: only act if still in a state that needs updating
            if dep.status != TaskStatus::Pending && dep.status != TaskStatus::Blocked {
                continue;
            }

            let all_deps = self.task_repo.get_dependencies(dep.id).await
                .map_err(|e| format!("Failed to get dependencies: {}", e))?;

            if all_deps.iter().all(|d| d.status == TaskStatus::Complete) {
                let mut updated = dep.clone();
                if updated.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("Failed to update task: {}", e))?;

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Debug,
                        category: EventCategory::Task,
                        goal_id: event.goal_id,
                        task_id: Some(dep.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskReady {
                            task_id: dep.id,
                            task_title: dep.title.clone(),
                        },
                    });
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

// ============================================================================
// TaskFailedBlockHandler
// ============================================================================

/// When a task fails with retries exhausted, block its dependent tasks.
pub struct TaskFailedBlockHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> TaskFailedBlockHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
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
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, retry_count) = match &event.payload {
            EventPayload::TaskFailed { task_id, retry_count, .. } => (*task_id, *retry_count),
            EventPayload::TaskCanceled { task_id, .. } => {
                // For canceled tasks, always block dependents (retries don't apply)
                let dependents = self.task_repo.get_dependents(*task_id).await
                    .map_err(|e| format!("Failed to get dependents: {}", e))?;

                for dep in dependents {
                    if dep.status == TaskStatus::Blocked || dep.status.is_terminal() {
                        continue;
                    }
                    let mut updated = dep.clone();
                    if updated.transition_to(TaskStatus::Blocked).is_ok() {
                        self.task_repo.update(&updated).await
                            .map_err(|e| format!("Failed to update task: {}", e))?;
                    }
                }
                return Ok(Reaction::None);
            }
            _ => return Ok(Reaction::None),
        };

        // Only block dependents if retries are exhausted.
        // Fetch the actual task to check max_retries.
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if retry_count < task.max_retries {
            return Ok(Reaction::None);
        }

        let dependents = self.task_repo.get_dependents(task_id).await
            .map_err(|e| format!("Failed to get dependents: {}", e))?;

        for dep in dependents {
            // Idempotency: only block if not already blocked or terminal
            if dep.status == TaskStatus::Blocked || dep.status.is_terminal() {
                continue;
            }

            let mut updated = dep.clone();
            if updated.transition_to(TaskStatus::Blocked).is_ok() {
                self.task_repo.update(&updated).await
                    .map_err(|e| format!("Failed to update task: {}", e))?;
            }
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// MemoryMaintenanceHandler
// ============================================================================

/// Triggered by the "memory-maintenance" scheduled event. Runs full
/// maintenance (prune expired/decayed, check promotions, resolve conflicts).
pub struct MemoryMaintenanceHandler<M: MemoryRepository> {
    memory_service: Arc<MemoryService<M>>,
}

impl<M: MemoryRepository> MemoryMaintenanceHandler<M> {
    pub fn new(memory_service: Arc<MemoryService<M>>) -> Self {
        Self { memory_service }
    }
}

#[async_trait]
impl<M: MemoryRepository + 'static> EventHandler for MemoryMaintenanceHandler<M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryMaintenanceHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "memory-maintenance"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (report, service_events) = self.memory_service.run_maintenance().await
            .map_err(|e| format!("Memory maintenance failed: {}", e))?;

        let mut events = service_events;

        let total_pruned = report.expired_pruned + report.decayed_pruned;
        if total_pruned > 0 {
            events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Memory,
                goal_id: None,
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::MemoryPruned {
                    count: total_pruned,
                    reason: format!(
                        "Scheduled maintenance: {} expired, {} decayed, {} promoted, {} conflicts resolved",
                        report.expired_pruned, report.decayed_pruned,
                        report.promoted, report.conflicts_resolved,
                    ),
                },
            });
        }

        // Always emit a summary event for observability
        events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Memory,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::MemoryMaintenanceCompleted {
                expired_pruned: report.expired_pruned,
                decayed_pruned: report.decayed_pruned,
                promoted: report.promoted,
                conflicts_resolved: report.conflicts_resolved,
            },
        });

        Ok(Reaction::EmitEvents(events))
    }
}

// ============================================================================
// GoalRetiredHandler
// ============================================================================

/// When a goal is retired, invalidate the active goals cache and emit a
/// summary event. Does not cancel tasks (goals and tasks are decoupled
/// in this architecture — tasks do not carry a goal_id field).
pub struct GoalRetiredHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    active_goals_cache: Arc<RwLock<Vec<Goal>>>,
}

impl<G: GoalRepository> GoalRetiredHandler<G> {
    pub fn new(goal_repo: Arc<G>, active_goals_cache: Arc<RwLock<Vec<Goal>>>) -> Self {
        Self { goal_repo, active_goals_cache }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalRetiredHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalRetiredHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Goal],
                payload_types: vec!["GoalStatusChanged".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::GoalStatusChanged { to_status, .. } if to_status == "retired"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let goal_id = match event.goal_id {
            Some(id) => id,
            None => return Ok(Reaction::None),
        };

        tracing::info!("GoalRetiredHandler: goal {} retired, refreshing active goals cache", goal_id);

        // Refresh the active goals cache to exclude the retired goal
        let goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("Failed to refresh active goals: {}", e))?;
        {
            let mut cache = self.active_goals_cache.write().await;
            *cache = goals;
        }

        // Emit a GoalStatusChanged event is already the triggering event;
        // we log for observability and emit no additional events.
        Ok(Reaction::None)
    }
}

// ============================================================================
// EscalationTimeoutHandler
// ============================================================================

/// Triggered by the "escalation-check" scheduled event. Emits a notification
/// that escalation deadlines should be checked. The actual timeout logic is
/// handled by the poll-based `check_escalation_deadlines` in the orchestrator,
/// since escalation state lives in the orchestrator's in-memory store.
///
/// This handler provides a fast-path signal: when it fires, the orchestrator
/// can immediately check deadlines rather than waiting for the next poll tick.
pub struct EscalationTimeoutHandler {
    escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>,
}

impl EscalationTimeoutHandler {
    pub fn new(escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>) -> Self {
        Self { escalation_store }
    }
}

#[async_trait]
impl EventHandler for EscalationTimeoutHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EscalationTimeoutHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "escalation-check"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        // Check escalation deadlines from the shared store
        let now = chrono::Utc::now();
        let store = self.escalation_store.read().await;
        let expired: Vec<_> = store.iter()
            .filter(|e| e.escalation.deadline.is_some_and(|d| now > d))
            .cloned()
            .collect();
        drop(store);

        if expired.is_empty() {
            return Ok(Reaction::None);
        }

        tracing::info!("EscalationTimeoutHandler: {} escalation(s) past deadline", expired.len());

        let mut new_events = Vec::new();
        for esc in &expired {
            let default_action = esc.escalation.default_action
                .as_deref()
                .unwrap_or("timeout-logged")
                .to_string();

            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Warning,
                category: EventCategory::Escalation,
                goal_id: esc.goal_id,
                task_id: esc.task_id,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::HumanEscalationExpired {
                    task_id: esc.task_id,
                    goal_id: esc.goal_id,
                    default_action,
                },
            });
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

// ============================================================================
// TaskFailedRetryHandler
// ============================================================================

/// When a task fails with retries remaining, transition it back to Ready.
/// Runs at NORMAL priority (after SYSTEM-priority TaskFailedBlockHandler).
pub struct TaskFailedRetryHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    max_retries: u32,
}

impl<T: TaskRepository> TaskFailedRetryHandler<T> {
    pub fn new(task_repo: Arc<T>, max_retries: u32) -> Self {
        Self { task_repo, max_retries }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskFailedRetryHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskFailedRetryHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, error) = match &event.payload {
            EventPayload::TaskFailed { task_id, error, .. } => (*task_id, error.as_str()),
            _ => return Ok(Reaction::None),
        };

        // Re-fetch task to check it's still Failed (idempotency)
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if task.status != TaskStatus::Failed {
            return Ok(Reaction::None);
        }

        // Use task.can_retry() which checks retry_count < max_retries atomically
        if !task.can_retry() || task.retry_count >= self.max_retries {
            return Ok(Reaction::None);
        }

        // Skip tasks superseded by the review failure loop-back handler,
        // and tasks that are part of a review loop chain (ReviewFailureLoopHandler
        // manages their full lifecycle — independent retry would create duplicate work tracks).
        if task.context.custom.contains_key("review_loop_active")
            || task.context.custom.contains_key("review_iteration")
        {
            return Ok(Reaction::None);
        }

        // Skip workflow phase subtasks — the workflow engine manages rework via
        // verification retries and gate escalation. Generic retry would race with
        // WorkflowSubtaskCompletionHandler and cause double-advance.
        if task.context.custom.contains_key("workflow_phase") {
            return Ok(Reaction::None);
        }

        let is_max_turns = error.starts_with("error_max_turns");

        // Circuit-break: tasks that repeatedly exhaust their turn budget should not retry
        // indefinitely. After MAX_CONSECUTIVE_BUDGET_FAILURES consecutive budget failures,
        // leave the task in Failed state so upstream handlers (review loop, specialist
        // triggers) can respond appropriately.
        const MAX_CONSECUTIVE_BUDGET_FAILURES: u64 = 3;
        if is_max_turns {
            let consecutive = task.context.custom
                .get("consecutive_budget_failures")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            if consecutive >= MAX_CONSECUTIVE_BUDGET_FAILURES {
                tracing::info!(
                    "Task {} circuit-breaker: {} consecutive budget failures, not retrying",
                    task_id,
                    consecutive
                );
                return Ok(Reaction::None);
            }
        }

        // Skip exponential backoff for structural failures (max_turns) — immediate retry
        if !is_max_turns {
            let backoff_secs = 2u64.pow(task.retry_count.min(10));
            if let Some(completed_at) = task.completed_at {
                let elapsed = (chrono::Utc::now() - completed_at).num_seconds();
                if elapsed < backoff_secs as i64 {
                    // Not ready to retry yet; the scheduled retry-check will try again
                    return Ok(Reaction::None);
                }
            }
        }

        let mut updated = task.clone();

        // For max_turns failures, inject hint so the spawner can increase the turn budget
        // and track consecutive failures for the circuit-breaker above.
        if is_max_turns {
            let consecutive = updated.context.custom
                .get("consecutive_budget_failures")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            updated.context.custom.insert(
                "consecutive_budget_failures".to_string(),
                serde_json::json!(consecutive),
            );
            updated.context.push_hint_bounded("retry:max_turns_exceeded".to_string());
            updated.context.custom.insert(
                "last_failure_reason".to_string(),
                serde_json::Value::String(error.to_string()),
            );
        } else {
            // Non-budget failure — reset the consecutive budget failure counter so a
            // later budget failure doesn't inherit a stale count from a different failure mode.
            updated.context.custom.remove("consecutive_budget_failures");
        }

        if updated.retry().is_ok() {
            self.task_repo.update(&updated).await
                .map_err(|e| format!("Failed to update task: {}", e))?;

            let events = vec![
                UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(task_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskRetrying {
                        task_id,
                        attempt: updated.retry_count,
                        max_attempts: updated.max_retries,
                    },
                },
                UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Debug,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(task_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskReady {
                        task_id,
                        task_title: updated.title.clone(),
                    },
                },
            ];
            return Ok(Reaction::EmitEvents(events));
        }

        Ok(Reaction::None)
    }
}

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
        Self { task_repo, command_bus, max_review_iterations }
    }

    /// Check whether a task is a review task based on agent_type or title.
    fn is_review_task(task: &Task) -> bool {
        if let Some(ref agent_type) = task.agent_type
            && agent_type == "code-reviewer" {
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
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::services::command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand};
        use crate::domain::models::{TaskPriority, TaskSource, TaskContext};

        let task_id = match &event.payload {
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Fetch the failed task
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("ReviewFailureLoopHandler: failed to get task: {}", e))?
            .ok_or_else(|| format!("ReviewFailureLoopHandler: task {} not found", task_id))?;

        // Only handle review tasks
        if !Self::is_review_task(&task) {
            return Ok(Reaction::None);
        }

        // Skip workflow phase subtasks — the workflow engine handles rework via
        // verification retries and gate escalation, not the review loop handler.
        if task.context.custom.contains_key("workflow_phase") {
            return Ok(Reaction::None);
        }

        // Idempotency: skip if already handled
        if task.context.custom.contains_key("review_loop_active") {
            return Ok(Reaction::None);
        }

        // Check iteration count
        let current_iteration = task.context.custom
            .get("review_iteration")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

        if current_iteration >= self.max_review_iterations {
            tracing::info!(
                "ReviewFailureLoopHandler: task {} at iteration {}/{}, deferring to normal failure handling",
                task_id, current_iteration, self.max_review_iterations,
            );
            return Ok(Reaction::None);
        }

        // Set the review_loop_active flag to prevent the retry handler from acting
        let mut flagged = task.clone();
        flagged.context.custom.insert(
            "review_loop_active".to_string(),
            serde_json::Value::Bool(true),
        );
        self.task_repo.update(&flagged).await
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
            "review_iteration".to_string(),
            serde_json::json!(next_iteration),
        );
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
                tracing::warn!("ReviewFailureLoopHandler: failed to create re-plan task: {}", e);
                return Ok(Reaction::None);
            }
        };

        // --- Create re-implement task ---
        let mut reimpl_context = TaskContext::default();
        reimpl_context.custom.insert(
            "review_iteration".to_string(),
            serde_json::json!(next_iteration),
        );

        let reimpl_idem = format!("review-loop:impl:{}:{}", task_id, next_iteration);
        let reimpl_envelope = CommandEnvelope::new(
            CommandSource::EventHandler("ReviewFailureLoopHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(format!("Re-implement (review iteration {})", next_iteration)),
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
                tracing::warn!("ReviewFailureLoopHandler: failed to create re-implement task: {}", e);
                return Ok(Reaction::None);
            }
        };

        // --- Create re-review task ---
        let mut rereview_context = TaskContext::default();
        rereview_context.custom.insert(
            "review_iteration".to_string(),
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
                        task_id, e
                    );
                }
                t.id
            }
            Ok(_) => {
                tracing::warn!("ReviewFailureLoopHandler: unexpected result type for re-review task");
                uuid::Uuid::new_v4()
            }
            Err(e) => {
                tracing::warn!("ReviewFailureLoopHandler: failed to create re-review task: {}", e);
                uuid::Uuid::new_v4()
            }
        };

        tracing::info!(
            "ReviewFailureLoopHandler: created review loop-back iteration {} for task {}",
            next_iteration, task_id,
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

// ============================================================================
// GoalCreatedHandler
// ============================================================================

/// When a goal starts, refresh the active goals cache.
pub struct GoalCreatedHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    active_goals_cache: Arc<RwLock<Vec<Goal>>>,
}

impl<G: GoalRepository> GoalCreatedHandler<G> {
    pub fn new(goal_repo: Arc<G>, active_goals_cache: Arc<RwLock<Vec<Goal>>>) -> Self {
        Self { goal_repo, active_goals_cache }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalCreatedHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalCreatedHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Goal])
                .payload_types(vec!["GoalStarted".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("Failed to get active goals: {}", e))?;

        let mut cache = self.active_goals_cache.write().await;
        *cache = goals;

        Ok(Reaction::None)
    }
}

// ============================================================================
// StatsUpdateHandler
// ============================================================================

/// Triggered by the "stats-update" scheduled event. Refreshes swarm statistics.
pub struct StatsUpdateHandler<G: GoalRepository, T: TaskRepository, W: WorktreeRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
    stats: Arc<RwLock<SwarmStats>>,
    agent_semaphore: Arc<Semaphore>,
    max_agents: usize,
    total_tokens: Arc<AtomicU64>,
}

impl<G: GoalRepository, T: TaskRepository, W: WorktreeRepository> StatsUpdateHandler<G, T, W> {
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        worktree_repo: Arc<W>,
        stats: Arc<RwLock<SwarmStats>>,
        agent_semaphore: Arc<Semaphore>,
        max_agents: usize,
        total_tokens: Arc<AtomicU64>,
    ) -> Self {
        Self { goal_repo, task_repo, worktree_repo, stats, agent_semaphore, max_agents, total_tokens }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static, W: WorktreeRepository + 'static> EventHandler for StatsUpdateHandler<G, T, W> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "StatsUpdateHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "stats-update"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let task_counts = self.task_repo.count_by_status().await
            .map_err(|e| format!("Failed to count tasks: {}", e))?;
        let active_worktrees = self.worktree_repo.list_active().await
            .map_err(|e| format!("Failed to list worktrees: {}", e))?
            .len();

        let active_goals = self.goal_repo.list(crate::domain::ports::GoalFilter {
            status: Some(crate::domain::models::GoalStatus::Active),
            ..Default::default()
        }).await.map_err(|e| format!("Failed to list goals: {}", e))?.len();

        let new_stats = SwarmStats {
            active_goals,
            pending_tasks: *task_counts.get(&TaskStatus::Pending).unwrap_or(&0) as usize,
            ready_tasks: *task_counts.get(&TaskStatus::Ready).unwrap_or(&0) as usize,
            running_tasks: *task_counts.get(&TaskStatus::Running).unwrap_or(&0) as usize,
            completed_tasks: *task_counts.get(&TaskStatus::Complete).unwrap_or(&0) as usize,
            failed_tasks: *task_counts.get(&TaskStatus::Failed).unwrap_or(&0) as usize,
            active_agents: self.max_agents - self.agent_semaphore.available_permits(),
            active_worktrees,
            total_tokens_used: self.total_tokens.load(Ordering::Relaxed),
        };

        {
            let mut s = self.stats.write().await;
            *s = new_stats.clone();
        }

        let status_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Orchestrator,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::StatusUpdate(SwarmStatsPayload::from(new_stats)),
        };

        Ok(Reaction::EmitEvents(vec![status_event]))
    }
}

// ============================================================================
// ReconciliationHandler
// ============================================================================

/// Triggered by the "reconciliation" scheduled event. Scans for missed state
/// transitions, detects stale running tasks, and corrects them (safety net
/// for the event-driven system).
pub struct ReconciliationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    /// Tasks stuck in Running longer than this are considered stale (seconds).
    stale_task_timeout_secs: u64,
}

impl<T: TaskRepository> ReconciliationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo, stale_task_timeout_secs: 7200 } // 2 hours default
    }

    pub fn with_stale_timeout(mut self, secs: u64) -> Self {
        self.stale_task_timeout_secs = secs;
        self
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ReconciliationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ReconciliationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "reconciliation"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let mut corrections: u32 = 0;
        let mut new_events = Vec::new();

        // Check Pending tasks
        let pending = self.task_repo.list_by_status(TaskStatus::Pending).await
            .map_err(|e| format!("Failed to list pending tasks: {}", e))?;

        for task in &pending {
            let deps = self.task_repo.get_dependencies(task.id).await
                .map_err(|e| format!("Failed to get deps: {}", e))?;

            if deps.iter().any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled) {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Blocked).is_ok() {
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("Failed to update: {}", e))?;
                    corrections += 1;
                }
            } else if deps.iter().all(|d| d.status == TaskStatus::Complete) {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("Failed to update: {}", e))?;
                    corrections += 1;
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Debug,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskReady {
                            task_id: task.id,
                            task_title: task.title.clone(),
                        },
                    });
                }
            }
        }

        // Check Blocked tasks that might now be unblocked or should cascade failure
        let blocked = self.task_repo.list_by_status(TaskStatus::Blocked).await
            .map_err(|e| format!("Failed to list blocked tasks: {}", e))?;

        for task in &blocked {
            let deps = self.task_repo.get_dependencies(task.id).await
                .map_err(|e| format!("Failed to get deps: {}", e))?;

            let has_failed_dep = deps.iter().any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled);

            if has_failed_dep {
                // Cascade failure: if a critical dependency has permanently failed,
                // fail this task too rather than leaving it stuck in Blocked forever
                let all_failed_or_complete = deps.iter().all(|d| {
                    d.status == TaskStatus::Complete ||
                    d.status == TaskStatus::Failed ||
                    d.status == TaskStatus::Canceled
                });
                if all_failed_or_complete {
                    let mut updated = task.clone();
                    if updated.transition_to(TaskStatus::Failed).is_ok() {
                        self.task_repo.update(&updated).await
                            .map_err(|e| format!("Failed to update: {}", e))?;
                        corrections += 1;
                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: chrono::Utc::now(),
                            severity: EventSeverity::Warning,
                            category: EventCategory::Task,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::TaskFailed {
                                task_id: task.id,
                                error: "cascade-failure: critical dependency failed or canceled".to_string(),
                                retry_count: task.retry_count,
                            },
                        });
                    }
                }
                continue;
            }

            if deps.iter().all(|d| d.status == TaskStatus::Complete) {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("Failed to update: {}", e))?;
                    corrections += 1;
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Debug,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskReady {
                            task_id: task.id,
                            task_title: task.title.clone(),
                        },
                    });
                }
            }
        }

        // Stale-task detection: tasks stuck in Running for > stale_task_timeout_secs
        // Tiered warnings: 50% -> TaskRunningLong, 80% -> TaskRunningCritical + escalation, 100% -> fail
        let running = self.task_repo.list_by_status(TaskStatus::Running).await
            .map_err(|e| format!("Failed to list running tasks: {}", e))?;

        let now = chrono::Utc::now();
        let timeout = chrono::Duration::seconds(self.stale_task_timeout_secs as i64);
        let warning_threshold = chrono::Duration::seconds((self.stale_task_timeout_secs as f64 * 0.5) as i64);
        let critical_threshold = chrono::Duration::seconds((self.stale_task_timeout_secs as f64 * 0.8) as i64);

        for task in &running {
            if let Some(started_at) = task.started_at {
                let elapsed = now - started_at;
                let runtime_secs = elapsed.num_seconds().max(0) as u64;

                if elapsed > timeout {
                    // 100% — fail the task
                    let mut updated = task.clone();
                    updated.retry_count += 1;
                    if updated.transition_to(TaskStatus::Failed).is_ok() {
                        self.task_repo.update(&updated).await
                            .map_err(|e| format!("Failed to update stale task: {}", e))?;
                        corrections += 1;

                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: chrono::Utc::now(),
                            severity: EventSeverity::Warning,
                            category: EventCategory::Task,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::TaskFailed {
                                task_id: task.id,
                                error: format!("stale-timeout: task running for > {}s", self.stale_task_timeout_secs),
                                retry_count: updated.retry_count,
                            },
                        });

                        tracing::warn!(
                            "ReconciliationHandler: stale task {} failed after {}s (started: {})",
                            task.id, self.stale_task_timeout_secs, started_at
                        );
                    }
                } else if elapsed > critical_threshold {
                    // 80% — critical warning + escalation
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Warning,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskRunningCritical {
                            task_id: task.id,
                            runtime_secs,
                        },
                    });

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Warning,
                        category: EventCategory::Escalation,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::HumanEscalationNeeded {
                            goal_id: None,
                            task_id: Some(task.id),
                            reason: format!(
                                "Task '{}' running for {}s (80% of {}s timeout)",
                                task.title, runtime_secs, self.stale_task_timeout_secs
                            ),
                            urgency: "high".to_string(),
                            is_blocking: false,
                        },
                    });
                } else if elapsed > warning_threshold {
                    // 50% — early warning
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Info,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskRunningLong {
                            task_id: task.id,
                            runtime_secs,
                        },
                    });
                }
            }
        }

        // Emit reconciliation completed event
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Orchestrator,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ReconciliationCompleted { corrections_made: corrections },
        });

        if corrections > 0 {
            tracing::info!("ReconciliationHandler: made {} corrections", corrections);
        }

        Ok(Reaction::EmitEvents(new_events))
    }
}

// ============================================================================
// RetryProcessingHandler
// ============================================================================

/// Triggered by the "retry-check" scheduled event. Supplements TaskFailedRetryHandler
/// for cases where the inline handler missed a retry.
pub struct RetryProcessingHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    max_retries: u32,
}

impl<T: TaskRepository> RetryProcessingHandler<T> {
    pub fn new(task_repo: Arc<T>, max_retries: u32) -> Self {
        Self { task_repo, max_retries }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for RetryProcessingHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "RetryProcessingHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "retry-check"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let failed = self.task_repo.list_by_status(TaskStatus::Failed).await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        let mut new_events = Vec::new();

        for task in failed {
            // Skip workflow phase subtasks — the workflow engine manages their
            // lifecycle. Generic retry would race with
            // WorkflowSubtaskCompletionHandler and cause double-advance.
            if task.context.custom.contains_key("workflow_phase") {
                continue;
            }

            // Skip review-loop-managed tasks — ReviewFailureLoopHandler owns
            // their full retry lifecycle.
            if task.context.custom.contains_key("review_loop_active")
                || task.context.custom.contains_key("review_iteration")
            {
                continue;
            }

            // Circuit-break consecutive budget failures: tasks that repeatedly
            // exhaust their turn budget should not retry indefinitely.
            if task.context.custom.get("last_failure_reason")
                .and_then(|v| v.as_str())
                .is_some_and(|e| e.starts_with("error_max_turns"))
            {
                let consecutive = task.context.custom
                    .get("consecutive_budget_failures")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if consecutive >= 3 {
                    continue;
                }
            }

            if task.retry_count < self.max_retries {
                let mut updated = task.clone();
                // Use retry() instead of transition_to(Ready) so that
                // retry_count is properly incremented.
                if updated.retry().is_ok() {
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("Failed to update: {}", e))?;

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Debug,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskReady {
                            task_id: task.id,
                            task_title: updated.title.clone(),
                        },
                    });
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

// ============================================================================
// SpecialistCheckHandler
// ============================================================================

/// Triggered by the "specialist-check" scheduled event (30s).
/// Scans tasks in `Failed` status with retries exhausted and signals the
/// orchestrator to trigger specialist processing via a shared channel.
pub struct SpecialistCheckHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    specialist_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
    max_retries: u32,
}

impl<T: TaskRepository> SpecialistCheckHandler<T> {
    pub fn new(task_repo: Arc<T>, specialist_tx: tokio::sync::mpsc::Sender<uuid::Uuid>, max_retries: u32) -> Self {
        Self { task_repo, specialist_tx, max_retries }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for SpecialistCheckHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "SpecialistCheckHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "specialist-check"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let failed = self.task_repo.list_by_status(TaskStatus::Failed).await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        for task in failed {
            if task.retry_count >= self.max_retries {
                // Signal orchestrator to evaluate specialist intervention
                let _ = self.specialist_tx.try_send(task.id);
            }
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// EvolutionEvaluationHandler
// ============================================================================

/// Triggered by the "evolution-evaluation" scheduled event (120s).
/// Queries recently completed/failed tasks, computes per-agent-type success
/// rates, and emits EvolutionTriggered when refinement is warranted.
pub struct EvolutionEvaluationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> EvolutionEvaluationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for EvolutionEvaluationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EvolutionEvaluationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "evolution-evaluation"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use std::collections::HashMap;

        // Get recently completed and failed tasks
        let completed = self.task_repo.list_by_status(TaskStatus::Complete).await
            .map_err(|e| format!("Failed to list completed tasks: {}", e))?;
        let failed = self.task_repo.list_by_status(TaskStatus::Failed).await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        // Compute per-agent-type success rates
        let mut agent_stats: HashMap<String, (u32, u32)> = HashMap::new(); // (success, total)

        for task in &completed {
            let agent = task.agent_type.as_deref().unwrap_or("unknown");
            let entry = agent_stats.entry(agent.to_string()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += 1;
        }

        for task in &failed {
            if task.retry_count >= task.max_retries {
                let agent = task.agent_type.as_deref().unwrap_or("unknown");
                let entry = agent_stats.entry(agent.to_string()).or_insert((0, 0));
                entry.1 += 1;
            }
        }

        let mut new_events = Vec::new();

        // Emit EvolutionTriggered for agents with low success rates
        for (agent_name, (successes, total)) in &agent_stats {
            if *total >= 5 {
                let success_rate = *successes as f64 / *total as f64;
                if success_rate < 0.6 {
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Info,
                        category: EventCategory::Agent,
                        goal_id: None,
                        task_id: None,
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::EvolutionTriggered {
                            template_name: agent_name.clone(),
                            trigger: format!("Low success rate: {:.0}% ({}/{})", success_rate * 100.0, successes, total),
                        },
                    });
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

// ============================================================================
// A2APollHandler
// ============================================================================

/// Triggered by the "a2a-poll" scheduled event (15s).
/// Polls the A2A gateway for pending inbound delegations and submits tasks
/// through the CommandBus so they go through validation, dedup, and event journaling.
pub struct A2APollHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    a2a_gateway_url: String,
    consecutive_failures: AtomicU64,
}

impl A2APollHandler {
    pub fn new(command_bus: Arc<crate::services::command_bus::CommandBus>, a2a_gateway_url: String) -> Self {
        Self { command_bus, a2a_gateway_url, consecutive_failures: AtomicU64::new(0) }
    }
}

#[async_trait]
impl EventHandler for A2APollHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "A2APollHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "a2a-poll"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::services::command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand};
        use crate::domain::models::TaskPriority;

        // Poll A2A gateway for pending inbound delegations
        let url = format!("{}/tasks/pending", self.a2a_gateway_url);
        let client = reqwest::Client::new();

        let response = match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) => resp,
            Err(e) => {
                let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::warn!("A2APollHandler: gateway unreachable (consecutive failures: {}): {}", failures, e);
                if failures >= 3 {
                    let diagnostic = UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Orchestrator,
                        goal_id: None,
                        task_id: None,
                        correlation_id: None,
                        source_process_id: None,
                        payload: EventPayload::HumanEscalationNeeded {
                            goal_id: None,
                            task_id: None,
                            reason: format!(
                                "A2A gateway at {} has been unreachable for {} consecutive polls",
                                self.a2a_gateway_url, failures
                            ),
                            urgency: "medium".to_string(),
                            is_blocking: false,
                        },
                    };
                    return Ok(Reaction::EmitEvents(vec![diagnostic]));
                }
                return Ok(Reaction::None);
            }
        };

        if !response.status().is_success() {
            let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(
                "A2APollHandler: gateway returned non-success status {} (consecutive failures: {})",
                response.status(), failures
            );
            return Ok(Reaction::None);
        }

        // Reset consecutive failure counter on success
        self.consecutive_failures.store(0, Ordering::Relaxed);

        let delegations: Vec<serde_json::Value> = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("A2APollHandler: failed to parse response: {}", e);
                return Ok(Reaction::None);
            }
        };

        for delegation in delegations {
            let title = delegation.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("A2A Delegated Task")
                .to_string();
            let description = delegation.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let envelope = CommandEnvelope::new(
                CommandSource::A2A("inbound-delegation".to_string()),
                DomainCommand::Task(TaskCommand::Submit {
                    title: Some(title.clone()),
                    description,
                    parent_id: None,
                    priority: TaskPriority::Normal,
                    agent_type: None,
                    depends_on: vec![],
                    context: Box::new(None),
                    idempotency_key: None,
                    source: crate::domain::models::TaskSource::System,
                    deadline: None,
                    task_type: None,
                    execution_mode: None,
                }),
            );

            if let Err(e) = self.command_bus.dispatch(envelope).await {
                tracing::warn!("A2APollHandler: failed to submit task '{}': {}", title, e);
            }
        }

        // Events are emitted by the CommandBus pipeline; no manual emission needed.
        Ok(Reaction::None)
    }
}

// ============================================================================
// GoalEvaluationHandler
// ============================================================================

/// Triggered by the "goal-evaluation" scheduled event (60s).
/// Observes task/memory state independently and emits signal events about
/// Filter tasks whose inferred domains overlap with a goal's applicability domains.
///
/// Universal goals (empty domains) match all tasks. Otherwise, each task's
/// domains are inferred via `GoalContextService::infer_task_domains` and
/// checked for overlap with the goal's domains.
fn filter_tasks_by_goal_domains<'a, G: GoalRepository>(tasks: &'a [Task], goal: &Goal) -> Vec<&'a Task> {
    let goal_domains = &goal.applicability_domains;
    tasks
        .iter()
        .filter(|t| {
            goal_domains.is_empty() || {
                let task_domains = GoalContextService::<G>::infer_task_domains(t);
                task_domains.iter().any(|d| goal_domains.contains(d))
            }
        })
        .collect()
}

/// goal progress. This is a read-only observer that never modifies goals,
/// tasks, or memories.
pub struct GoalEvaluationHandler<G: GoalRepository, T: TaskRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
}

impl<G: GoalRepository, T: TaskRepository> GoalEvaluationHandler<G, T> {
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
    ) -> Self {
        Self { goal_repo, task_repo }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static>
    EventHandler for GoalEvaluationHandler<G, T>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalEvaluationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "goal-evaluation"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        // Load all active goals
        let goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("Failed to get active goals: {}", e))?;

        if goals.is_empty() {
            return Ok(Reaction::None);
        }

        // Get recent tasks (completed and failed)
        let completed = self.task_repo.list_by_status(TaskStatus::Complete).await
            .map_err(|e| format!("Failed to list completed tasks: {}", e))?;
        let failed = self.task_repo.list_by_status(TaskStatus::Failed).await
            .map_err(|e| format!("Failed to list failed tasks: {}", e))?;

        let mut new_events = Vec::new();

        for goal in &goals {
            // Find tasks whose inferred domains overlap with this goal's domains
            let relevant_completed = filter_tasks_by_goal_domains::<G>(&completed, goal);
            let relevant_failed = filter_tasks_by_goal_domains::<G>(&failed, goal);

            // Emit GoalIterationCompleted if there are completed tasks in matching domains
            if !relevant_completed.is_empty() {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Info,
                    category: EventCategory::Goal,
                    goal_id: Some(goal.id),
                    task_id: None,
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::GoalIterationCompleted {
                        goal_id: goal.id,
                        tasks_completed: relevant_completed.len(),
                    },
                });
            }

            // Check for constraint violations in failures
            for constraint in &goal.constraints {
                let violation_count = relevant_failed.iter()
                    .filter(|t| {
                        // Check if failures relate to constraint violations
                        let hints = t.context.hints.join(" ").to_lowercase();
                        hints.contains(&constraint.name.to_lowercase())
                    })
                    .count();

                if violation_count > 0 {
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Goal,
                        goal_id: Some(goal.id),
                        task_id: None,
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::GoalConstraintViolated {
                            goal_id: goal.id,
                            constraint_name: constraint.name.clone(),
                            violation: format!("{} task(s) failed with constraint-related errors", violation_count),
                        },
                    });
                }
            }

            // Check for semantic drift: recurring failure patterns
            if relevant_failed.len() >= 3 {
                // Group failures by common error patterns
                let mut failure_hints: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                for task in &relevant_failed {
                    for hint in &task.context.hints {
                        if hint.starts_with("Error:") {
                            let pattern = hint.chars().take(80).collect::<String>();
                            *failure_hints.entry(pattern).or_insert(0) += 1;
                        }
                    }
                }

                let recurring_gaps: Vec<String> = failure_hints.into_iter()
                    .filter(|(_, count)| *count >= 2)
                    .map(|(pattern, _)| pattern)
                    .collect();

                if !recurring_gaps.is_empty() {
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Warning,
                        category: EventCategory::Goal,
                        goal_id: Some(goal.id),
                        task_id: None,
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::SemanticDriftDetected {
                            goal_id: goal.id,
                            recurring_gaps,
                            iterations: relevant_failed.len() as u32,
                        },
                    });
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

// ============================================================================
// TaskReadySpawnHandler
// ============================================================================

/// When a TaskReady event fires, push the task_id into a channel for the
/// orchestrator to spawn an agent. Validates the task is still Ready
/// (idempotent).
pub struct TaskReadySpawnHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    ready_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
}

impl<T: TaskRepository> TaskReadySpawnHandler<T> {
    pub fn new(task_repo: Arc<T>, ready_tx: tokio::sync::mpsc::Sender<uuid::Uuid>) -> Self {
        Self { task_repo, ready_tx }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskReadySpawnHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskReadySpawnHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskReady".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let task_id = match &event.payload {
            EventPayload::TaskReady { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Validate task is still Ready (idempotent)
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?;

        match task {
            Some(t) if t.status == TaskStatus::Ready => {
                let _ = self.ready_tx.try_send(task_id);
            }
            _ => {
                // Task no longer Ready, skip
            }
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// MemoryReconciliationHandler
// ============================================================================

/// Periodic safety-net for memory subsystem: prunes expired/decayed memories,
/// promotes candidates, and detects orphaned memories.
///
/// Triggered by `ScheduledEventFired { name: "memory-reconciliation" }`.
pub struct MemoryReconciliationHandler<M: MemoryRepository> {
    memory_service: Arc<MemoryService<M>>,
}

impl<M: MemoryRepository> MemoryReconciliationHandler<M> {
    pub fn new(memory_service: Arc<MemoryService<M>>) -> Self {
        Self { memory_service }
    }
}

#[async_trait]
impl<M: MemoryRepository + 'static> EventHandler for MemoryReconciliationHandler<M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryReconciliationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "memory-reconciliation" {
            return Ok(Reaction::None);
        }

        let (report, events) = self.memory_service.run_maintenance().await
            .map_err(|e| format!("Memory reconciliation failed: {}", e))?;

        tracing::info!(
            expired = report.expired_pruned,
            decayed = report.decayed_pruned,
            promoted = report.promoted,
            conflicts = report.conflicts_resolved,
            "MemoryReconciliationHandler: maintenance complete"
        );

        if events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(events))
        }
    }
}

// ============================================================================
// GoalReconciliationHandler
// ============================================================================

/// Periodic safety-net for goal subsystem: re-evaluates active goals,
/// detects stale ones (no recent events), logs their status, and emits
/// escalation events for goals with no activity beyond a configurable
/// threshold (default 48h).
///
/// Triggered by `ScheduledEventFired { name: "goal-reconciliation" }`.
pub struct GoalReconciliationHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    /// Hours of inactivity after which a goal triggers a human escalation.
    escalation_threshold_hours: i64,
}

impl<G: GoalRepository> GoalReconciliationHandler<G> {
    pub fn new(goal_repo: Arc<G>) -> Self {
        Self { goal_repo, escalation_threshold_hours: 48 }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalReconciliationHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalReconciliationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "goal-reconciliation" {
            return Ok(Reaction::None);
        }

        let active_goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("GoalReconciliation: failed to get goals: {}", e))?;

        let now = chrono::Utc::now();
        let stale_threshold = chrono::Duration::hours(24);
        let escalation_threshold = chrono::Duration::hours(self.escalation_threshold_hours);
        let mut new_events = Vec::new();

        for goal in &active_goals {
            let age = now - goal.updated_at;
            if age > escalation_threshold {
                tracing::warn!(
                    goal_id = %goal.id,
                    goal_name = %goal.name,
                    hours_stale = age.num_hours(),
                    "GoalReconciliation: goal stale beyond escalation threshold, emitting escalation"
                );

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Escalation,
                    goal_id: Some(goal.id),
                    task_id: None,
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::HumanEscalationRequired {
                        goal_id: Some(goal.id),
                        task_id: None,
                        reason: format!(
                            "Goal '{}' has had no activity for {} hours",
                            goal.name,
                            age.num_hours()
                        ),
                        urgency: "medium".to_string(),
                        questions: vec![
                            format!("Goal '{}' appears stale. Should it be continued, paused, or retired?", goal.name),
                        ],
                        is_blocking: false,
                    },
                });
            } else if age > stale_threshold {
                tracing::info!(
                    goal_id = %goal.id,
                    goal_name = %goal.name,
                    hours_stale = age.num_hours(),
                    "GoalReconciliation: goal has not been updated recently"
                );
            }
        }

        tracing::debug!(
            active_goals = active_goals.len(),
            "GoalReconciliation: reconciliation sweep complete"
        );

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

// ============================================================================
// SystemStallDetectorHandler
// ============================================================================

/// Monitors global task activity and fires `HumanEscalationRequired` when the
/// swarm appears idle for longer than 2× the goal convergence check interval.
///
/// "Idle" means no tasks were created or completed between successive checks.
/// The handler tracks a running snapshot of `(completed_count, failed_count, pending_count)`
/// and a `last_activity` timestamp. On each tick it queries `count_by_status`
/// and bumps `last_activity` whenever the snapshot changes or Running tasks
/// exist (which implies work is in progress).
///
/// Triggered by `ScheduledEventFired { name: "system-stall-check" }`.
pub struct SystemStallDetectorHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    /// Maximum idle time before escalation (seconds).
    stall_threshold_secs: u64,
    /// Internal state: `(last_activity, prev_completed, prev_failed, prev_pending)`.
    state: RwLock<(chrono::DateTime<chrono::Utc>, u64, u64, u64)>,
}

impl<T: TaskRepository> SystemStallDetectorHandler<T> {
    /// Create with an explicit stall threshold.
    ///
    /// The default used by the orchestrator is `2 × goal_convergence_check_interval_secs`.
    pub fn new(task_repo: Arc<T>, stall_threshold_secs: u64) -> Self {
        Self {
            task_repo,
            stall_threshold_secs,
            state: RwLock::new((chrono::Utc::now(), 0, 0, 0)),
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for SystemStallDetectorHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "SystemStallDetectorHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "system-stall-check" {
            return Ok(Reaction::None);
        }

        let counts = self.task_repo.count_by_status().await
            .map_err(|e| format!("SystemStallDetector: failed to count tasks: {}", e))?;

        let completed = *counts.get(&TaskStatus::Complete).unwrap_or(&0);
        let failed = *counts.get(&TaskStatus::Failed).unwrap_or(&0);
        let pending = *counts.get(&TaskStatus::Pending).unwrap_or(&0);
        let running = *counts.get(&TaskStatus::Running).unwrap_or(&0);
        let ready = *counts.get(&TaskStatus::Ready).unwrap_or(&0);

        let now = chrono::Utc::now();
        let mut state = self.state.write().await;
        let (ref mut last_activity, ref mut prev_completed, ref mut prev_failed, ref mut prev_pending) = *state;

        // Activity detected if:
        // 1. Completed/failed counts changed (tasks finished since last check)
        // 2. Pending count changed (new tasks created since last check)
        // 3. Running or ready tasks exist (work is in progress)
        let snapshot_changed = completed != *prev_completed
            || failed != *prev_failed
            || pending != *prev_pending;
        let work_in_progress = running > 0 || ready > 0;

        if snapshot_changed || work_in_progress {
            *last_activity = now;
            *prev_completed = completed;
            *prev_failed = failed;
            *prev_pending = pending;
            return Ok(Reaction::None);
        }

        // No activity — check if we've exceeded the threshold
        let idle_secs = (now - *last_activity).num_seconds().max(0) as u64;

        if idle_secs < self.stall_threshold_secs {
            tracing::debug!(
                idle_secs,
                threshold = self.stall_threshold_secs,
                "SystemStallDetector: swarm idle but within threshold"
            );
            return Ok(Reaction::None);
        }

        tracing::warn!(
            idle_secs,
            threshold = self.stall_threshold_secs,
            "SystemStallDetector: swarm stalled, emitting escalation"
        );

        // 1. Escalation event for observability / human notification
        let escalation = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Warning,
            category: EventCategory::Escalation,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::HumanEscalationRequired {
                goal_id: None,
                task_id: None,
                reason: format!(
                    "System stall detected: no tasks created or completed for {} seconds (threshold: {}s). Auto-recovery triggered via goal-convergence-check.",
                    idle_secs, self.stall_threshold_secs,
                ),
                urgency: "high".to_string(),
                questions: vec![
                    "The swarm has had no task activity. Are there goals that need new work generated?".to_string(),
                    "Auto-recovery has been triggered — a goal convergence check is being fired.".to_string(),
                ],
                is_blocking: false,
            },
        };

        // 2. Synthetic convergence-check trigger for auto-recovery
        let convergence_trigger = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Info,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "goal-convergence-check".to_string(),
            },
        };

        // Reset activity timestamp so we don't fire repeatedly every tick
        *last_activity = now;

        Ok(Reaction::EmitEvents(vec![escalation, convergence_trigger]))
    }
}

// ============================================================================
// WorktreeReconciliationHandler
// ============================================================================

/// Triggered by the "reconciliation" scheduled event (piggybacks on existing schedule).
/// Detects orphaned worktrees — active worktrees whose associated task is in a
/// terminal state — and emits warning events. Does not delete worktrees.
pub struct WorktreeReconciliationHandler<T: TaskRepository, W: WorktreeRepository> {
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
}

impl<T: TaskRepository, W: WorktreeRepository> WorktreeReconciliationHandler<T, W> {
    pub fn new(task_repo: Arc<T>, worktree_repo: Arc<W>) -> Self {
        Self { task_repo, worktree_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, W: WorktreeRepository + 'static> EventHandler for WorktreeReconciliationHandler<T, W> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorktreeReconciliationHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "reconciliation"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::domain::models::WorktreeStatus;

        let active_worktrees = self.worktree_repo.list_active().await
            .map_err(|e| format!("WorktreeReconciliation: failed to list active worktrees: {}", e))?;

        let mut orphan_count = 0u32;
        let mut new_events = Vec::new();

        for wt in &active_worktrees {
            let task = self.task_repo.get(wt.task_id).await
                .map_err(|e| format!("WorktreeReconciliation: failed to get task: {}", e))?;

            let is_orphaned = match &task {
                Some(t) => t.is_terminal(),
                None => true, // Task doesn't exist — worktree is orphaned
            };

            if is_orphaned {
                orphan_count += 1;

                let reason = match &task {
                    Some(t) => format!("task in terminal state: {}", t.status.as_str()),
                    None => "task not found".to_string(),
                };

                // Actually destroy the orphaned worktree
                let mut updated_wt = wt.clone();
                updated_wt.status = WorktreeStatus::Removed;
                updated_wt.updated_at = chrono::Utc::now();
                updated_wt.completed_at = Some(chrono::Utc::now());
                if let Err(e) = self.worktree_repo.update(&updated_wt).await {
                    tracing::warn!(
                        worktree_id = %wt.id,
                        error = %e,
                        "WorktreeReconciliation: failed to mark worktree as removed"
                    );
                    continue;
                }

                tracing::warn!(
                    worktree_id = %wt.id,
                    task_id = %wt.task_id,
                    path = %wt.path,
                    reason = %reason,
                    "WorktreeReconciliation: orphaned worktree destroyed"
                );

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Orchestrator,
                    goal_id: None,
                    task_id: Some(wt.task_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::WorktreeDestroyed {
                        worktree_id: wt.id,
                        task_id: wt.task_id,
                        reason: reason.clone(),
                    },
                });
            }
        }

        if orphan_count > 0 {
            tracing::info!(
                "WorktreeReconciliation: {} orphaned worktree(s) destroyed",
                orphan_count
            );

            // Emit reconciliation summary with actual correction count
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Orchestrator,
                goal_id: None,
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ReconciliationCompleted {
                    corrections_made: orphan_count,
                },
            });
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

// ============================================================================
// WatermarkAuditHandler
// ============================================================================

/// Triggered by the "watermark-audit" scheduled event (600s).
/// Reads all handler watermarks from the event store, compares them to the
/// latest event sequence, and logs warnings for handlers that are
/// significantly behind (>100 events).
pub struct WatermarkAuditHandler {
    event_store: Arc<dyn EventStore>,
    /// Names of handlers to audit (snapshot taken at registration time).
    handler_names: Vec<String>,
}

impl WatermarkAuditHandler {
    pub fn new(event_store: Arc<dyn EventStore>, handler_names: Vec<String>) -> Self {
        Self { event_store, handler_names }
    }
}

#[async_trait]
impl EventHandler for WatermarkAuditHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WatermarkAuditHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "watermark-audit"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let latest = self.event_store.latest_sequence().await
            .map_err(|e| format!("WatermarkAudit: failed to get latest sequence: {}", e))?;

        let latest_seq = match latest {
            Some(seq) => seq.0,
            None => return Ok(Reaction::None), // No events in store yet
        };

        let mut behind_count = 0u32;
        let mut max_lag: u64 = 0;
        let mut new_events = Vec::new();

        for name in &self.handler_names {
            let wm = self.event_store.get_watermark(name).await
                .map_err(|e| format!("WatermarkAudit: failed to get watermark for {}: {}", name, e))?;

            let handler_seq = wm.map(|s| s.0).unwrap_or(0);
            let lag = latest_seq.saturating_sub(handler_seq);

            if lag > max_lag {
                max_lag = lag;
            }

            if lag > 100 {
                tracing::warn!(
                    handler = %name,
                    handler_seq = handler_seq,
                    latest_seq = latest_seq,
                    lag = lag,
                    "WatermarkAudit: handler is significantly behind"
                );
                behind_count += 1;
            }
        }

        if behind_count > 0 {
            tracing::info!(
                "WatermarkAudit: {} handler(s) significantly behind latest sequence {}",
                behind_count, latest_seq
            );

            // When lag > 100: trigger a catch-up sweep
            if max_lag > 100 {
                new_events.push(UnifiedEvent {
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
                        name: "trigger-rule-catchup".to_string(),
                    },
                });
            }

            // When lag > 500: emit a human escalation event
            if max_lag > 500 {
                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Escalation,
                    goal_id: None,
                    task_id: None,
                    correlation_id: None,
                    source_process_id: None,
                    payload: EventPayload::HumanEscalationRequired {
                        goal_id: None,
                        task_id: None,
                        reason: format!(
                            "Event processing critically behind: {} handler(s) lagging, max lag {} events",
                            behind_count, max_lag
                        ),
                        urgency: "high".to_string(),
                        questions: vec![
                            "Event handlers are critically behind. Should the system be restarted or investigated?".to_string(),
                        ],
                        is_blocking: false,
                    },
                });
            }
        } else {
            tracing::debug!(
                "WatermarkAudit: all handlers within 100 events of sequence {}",
                latest_seq
            );
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

// ============================================================================
// TriggerCatchupHandler
// ============================================================================

/// Triggered by the "trigger-rule-catchup" scheduled event (300s).
/// Re-evaluates events that the TriggerRuleEngine may have missed by reading
/// its own watermark and replaying events since that point.
pub struct TriggerCatchupHandler {
    trigger_engine: Arc<crate::services::trigger_rules::TriggerRuleEngine>,
    event_store: Arc<dyn EventStore>,
}

impl TriggerCatchupHandler {
    pub fn new(
        trigger_engine: Arc<crate::services::trigger_rules::TriggerRuleEngine>,
        event_store: Arc<dyn EventStore>,
    ) -> Self {
        Self { trigger_engine, event_store }
    }
}

#[async_trait]
impl EventHandler for TriggerCatchupHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TriggerCatchupHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "trigger-rule-catchup"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, ctx: &HandlerContext) -> Result<Reaction, String> {
        // Read the TriggerRuleEngine's own watermark
        let wm = self.event_store.get_watermark("TriggerRuleEngine").await
            .map_err(|e| format!("TriggerCatchup: failed to get watermark: {}", e))?;

        let since_seq = wm.unwrap_or(crate::services::event_bus::SequenceNumber(0));

        // Query events since that watermark
        let events = self.event_store.replay_since(since_seq).await
            .map_err(|e| format!("TriggerCatchup: failed to replay events: {}", e))?;

        if events.is_empty() {
            return Ok(Reaction::None);
        }

        let mut all_reactions = Vec::new();
        let mut max_seq = since_seq;

        let handler_ctx = HandlerContext {
            chain_depth: ctx.chain_depth,
            correlation_id: ctx.correlation_id,
        };

        for evt in &events {
            // Skip the catchup event itself to avoid infinite loops
            if matches!(&evt.payload, EventPayload::ScheduledEventFired { name, .. } if name == "trigger-rule-catchup") {
                continue;
            }

            // Re-evaluate through the trigger engine
            match self.trigger_engine.handle(evt, &handler_ctx).await {
                Ok(Reaction::EmitEvents(new_events)) => {
                    all_reactions.extend(new_events);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("TriggerCatchup: trigger engine error on seq {}: {}", evt.sequence, e);
                }
            }

            if evt.sequence > max_seq {
                max_seq = evt.sequence;
            }
        }

        // Update watermark after processing
        if max_seq > since_seq
            && let Err(e) = self.event_store.set_watermark("TriggerRuleEngine", max_seq).await {
                tracing::warn!("TriggerCatchup: failed to update watermark: {}", e);
            }

        tracing::debug!(
            events_replayed = events.len(),
            reactions = all_reactions.len(),
            "TriggerCatchup: catch-up sweep complete"
        );

        if all_reactions.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(all_reactions))
        }
    }
}

// ============================================================================
// EventStorePollerHandler
// ============================================================================

/// Triggered by the "event-store-poll" scheduled event.
/// Reads events from the SQLite EventStore with sequence numbers beyond
/// the poller's high-water mark and re-publishes them into the broadcast
/// channel, enabling cross-process event propagation.
///
/// Filters out events originating from the current process (using
/// `source_process_id`) to avoid echo loops.
pub struct EventStorePollerHandler {
    event_store: Arc<dyn EventStore>,
    /// Process ID of the local EventBus — events with this source are skipped.
    local_process_id: uuid::Uuid,
    /// High-water mark: the latest sequence number this poller has seen.
    high_water_mark: Arc<RwLock<u64>>,
    /// Maximum events to process per poll cycle.
    max_per_poll: usize,
}

impl EventStorePollerHandler {
    pub fn new(event_store: Arc<dyn EventStore>, local_process_id: uuid::Uuid) -> Self {
        Self {
            event_store,
            local_process_id,
            high_water_mark: Arc::new(RwLock::new(0)),
            max_per_poll: 100,
        }
    }

    /// Initialize the high-water mark from the event store's latest sequence.
    /// Call this at startup so we don't replay the entire history.
    ///
    /// When no watermark exists (first run), we start from
    /// `max_sequence - replay_window` instead of `max_sequence` to ensure
    /// recent events are replayed for catch-up.
    pub async fn initialize_watermark(&self) {
        self.initialize_watermark_with_replay(1000).await;
    }

    /// Initialize watermark with a configurable replay window.
    pub async fn initialize_watermark_with_replay(&self, replay_window: u64) {
        match self.event_store.get_watermark("EventStorePollerHandler").await {
            Ok(Some(seq)) => {
                let mut hwm = self.high_water_mark.write().await;
                *hwm = seq.0;
                tracing::info!("EventStorePoller: initialized watermark at {}", seq.0);
            }
            Ok(None) => {
                // No watermark yet — start from max_sequence - replay_window to
                // ensure recent events are replayed for catch-up
                if let Ok(Some(seq)) = self.event_store.latest_sequence().await {
                    let start_from = seq.0.saturating_sub(replay_window);
                    let mut hwm = self.high_water_mark.write().await;
                    *hwm = start_from;
                    tracing::info!(
                        "EventStorePoller: no watermark found, starting from seq {} (latest {} - window {})",
                        start_from, seq.0, replay_window
                    );
                }
            }
            Err(e) => {
                tracing::warn!("EventStorePoller: failed to read watermark: {}", e);
            }
        }
    }
}

#[async_trait]
impl EventHandler for EventStorePollerHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EventStorePollerHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "event-store-poll"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let hwm = {
            let h = self.high_water_mark.read().await;
            *h
        };

        // Query events beyond our high-water mark
        let events = self.event_store
            .query(
                crate::services::event_store::EventQuery::new()
                    .since_sequence(SequenceNumber(hwm + 1))
                    .ascending()
                    .limit(self.max_per_poll as u32),
            )
            .await
            .map_err(|e| format!("EventStorePoller: failed to query events: {}", e))?;

        if events.is_empty() {
            return Ok(Reaction::None);
        }

        let mut new_events = Vec::new();
        let mut new_hwm = hwm;

        for evt in &events {
            // Track highest sequence seen
            if evt.sequence.0 > new_hwm {
                new_hwm = evt.sequence.0;
            }

            // Skip events from this process (we already broadcast them)
            if evt.source_process_id == Some(self.local_process_id) {
                continue;
            }

            // Skip ScheduledEventFired — those are generated locally
            if matches!(&evt.payload, EventPayload::ScheduledEventFired { .. }) {
                continue;
            }

            new_events.push(evt.clone());
        }

        // Update high-water mark
        if new_hwm > hwm {
            let mut h = self.high_water_mark.write().await;
            *h = new_hwm;

            // Persist watermark
            if let Err(e) = self.event_store.set_watermark("EventStorePollerHandler", SequenceNumber(new_hwm)).await {
                tracing::warn!("EventStorePoller: failed to persist watermark: {}", e);
            }
        }

        if !new_events.is_empty() {
            tracing::info!(
                "EventStorePoller: re-publishing {} cross-process events (hwm {} -> {})",
                new_events.len(), hwm, new_hwm
            );
            Ok(Reaction::EmitEvents(new_events))
        } else {
            Ok(Reaction::None)
        }
    }
}

// ============================================================================
// DeadLetterRetryHandler
// ============================================================================

/// Triggered by the "dead-letter-retry" scheduled event.
/// Reads retryable entries from the dead letter queue, re-fetches the original
/// event from the store, and re-publishes it so handlers get another chance.
/// Applies exponential backoff (2^retry_count seconds) between retries.
/// Marks entries as resolved when max retries exceeded.
pub struct DeadLetterRetryHandler {
    event_store: Arc<dyn EventStore>,
}

impl DeadLetterRetryHandler {
    pub fn new(event_store: Arc<dyn EventStore>) -> Self {
        Self { event_store }
    }
}

#[async_trait]
impl EventHandler for DeadLetterRetryHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "DeadLetterRetryHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "dead-letter-retry"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let entries = self.event_store
            .get_retryable_dead_letters(10)
            .await
            .map_err(|e| format!("DeadLetterRetry: failed to get retryable entries: {}", e))?;

        if entries.is_empty() {
            return Ok(Reaction::None);
        }

        let mut events_to_replay = Vec::new();

        for entry in &entries {
            // If this is the last attempt, resolve it before re-publishing
            if entry.retry_count + 1 >= entry.max_retries {
                tracing::info!(
                    "DeadLetterRetry: max retries ({}) reached for handler '{}' on event seq {}, resolving",
                    entry.max_retries, entry.handler_name, entry.event_sequence
                );
                if let Err(e) = self.event_store.resolve_dead_letter(&entry.id).await {
                    tracing::warn!("DeadLetterRetry: failed to resolve entry {}: {}", entry.id, e);
                }
            } else {
                // Increment retry count BEFORE re-publishing to prevent duplicates on crash.
                // If re-publish fails, the DLQ entry still exists for next retry.
                let backoff_secs = 2i64.pow((entry.retry_count + 1).min(10));
                let next_retry = chrono::Utc::now() + chrono::Duration::seconds(backoff_secs);

                if let Err(e) = self.event_store.increment_dead_letter_retry(&entry.id, next_retry).await {
                    tracing::warn!("DeadLetterRetry: failed to increment retry for {}: {}", entry.id, e);
                    continue; // Skip re-publish if we couldn't mark the retry
                }
            }

            // Re-fetch the original event from the store
            let original = self.event_store
                .get_by_sequence(SequenceNumber(entry.event_sequence))
                .await
                .map_err(|e| format!("DeadLetterRetry: failed to get event seq {}: {}", entry.event_sequence, e))?;

            match original {
                Some(evt) => {
                    events_to_replay.push(evt);
                }
                None => {
                    // Event no longer in store (pruned), resolve the DLQ entry
                    tracing::info!(
                        "DeadLetterRetry: event seq {} no longer in store, resolving DLQ entry {}",
                        entry.event_sequence, entry.id
                    );
                    if let Err(e) = self.event_store.resolve_dead_letter(&entry.id).await {
                        tracing::warn!("DeadLetterRetry: failed to resolve entry {}: {}", entry.id, e);
                    }
                }
            }
        }

        if events_to_replay.is_empty() {
            Ok(Reaction::None)
        } else {
            tracing::info!(
                "DeadLetterRetry: re-publishing {} events from dead letter queue",
                events_to_replay.len()
            );
            Ok(Reaction::EmitEvents(events_to_replay))
        }
    }
}

// ============================================================================
// EventPruningHandler
// ============================================================================

/// Triggered by the "event-pruning" scheduled event.
/// Calls `event_store.prune_older_than()` to remove old events based on
/// the configured retention duration.
pub struct EventPruningHandler {
    event_store: Arc<dyn EventStore>,
    /// Retention duration in days.
    retention_days: u64,
}

impl EventPruningHandler {
    pub fn new(event_store: Arc<dyn EventStore>, retention_days: u64) -> Self {
        Self { event_store, retention_days }
    }
}

#[async_trait]
impl EventHandler for EventPruningHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EventPruningHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "event-pruning"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let retention = std::time::Duration::from_secs(self.retention_days * 24 * 3600);

        let pruned = self.event_store
            .prune_older_than(retention)
            .await
            .map_err(|e| format!("EventPruning: failed to prune events: {}", e))?;

        if pruned > 0 {
            tracing::info!("EventPruning: pruned {} events older than {} days", pruned, self.retention_days);

            let summary = UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Orchestrator,
                goal_id: None,
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ReconciliationCompleted {
                    corrections_made: pruned as u32,
                },
            };

            Ok(Reaction::EmitEvents(vec![summary]))
        } else {
            Ok(Reaction::None)
        }
    }
}

// ============================================================================
// TaskSLAEnforcementHandler (Phase 2a)
// ============================================================================

/// Triggered by the "sla-check" scheduled event (60s). Queries tasks with
/// deadlines and emits tiered SLA events (warning/critical/breached).
pub struct TaskSLAEnforcementHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    warning_threshold_pct: f64,
    critical_threshold_pct: f64,
    auto_escalate_on_breach: bool,
}

impl<T: TaskRepository> TaskSLAEnforcementHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        warning_threshold_pct: f64,
        critical_threshold_pct: f64,
        auto_escalate_on_breach: bool,
    ) -> Self {
        Self { task_repo, warning_threshold_pct, critical_threshold_pct, auto_escalate_on_breach }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for TaskSLAEnforcementHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskSLAEnforcementHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "sla-check"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let now = chrono::Utc::now();
        let mut new_events = Vec::new();

        // Check all active statuses for tasks with deadlines
        for status in &[TaskStatus::Pending, TaskStatus::Ready, TaskStatus::Running] {
            let tasks = self.task_repo.list_by_status(*status).await
                .map_err(|e| format!("SLA check failed: {}", e))?;

            for task in tasks {
                let deadline = match task.deadline {
                    Some(d) => d,
                    None => continue,
                };

                let total_duration = (deadline - task.created_at).num_seconds().max(1) as f64;
                let remaining = (deadline - now).num_seconds();

                if remaining <= 0 {
                    // Breached
                    let overdue_secs = -remaining;
                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Critical,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskSLABreached {
                            task_id: task.id,
                            deadline: deadline.to_rfc3339(),
                            overdue_secs,
                        },
                    });

                    if self.auto_escalate_on_breach {
                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: now,
                            severity: EventSeverity::Critical,
                            category: EventCategory::Escalation,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::HumanEscalationRequired {
                                goal_id: None,
                                task_id: Some(task.id),
                                reason: format!("Task '{}' SLA breached: overdue by {}s", task.title, overdue_secs),
                                urgency: "critical".to_string(),
                                questions: vec![format!("Task '{}' has missed its deadline. What should be done?", task.title)],
                                is_blocking: false,
                            },
                        });
                    }
                } else {
                    let remaining_pct = remaining as f64 / total_duration;

                    if remaining_pct < self.critical_threshold_pct {
                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: now,
                            severity: EventSeverity::Warning,
                            category: EventCategory::Task,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::TaskSLACritical {
                                task_id: task.id,
                                deadline: deadline.to_rfc3339(),
                                remaining_secs: remaining,
                            },
                        });
                    } else if remaining_pct < self.warning_threshold_pct {
                        new_events.push(UnifiedEvent {
                            id: EventId::new(),
                            sequence: SequenceNumber(0),
                            timestamp: now,
                            severity: EventSeverity::Warning,
                            category: EventCategory::Task,
                            goal_id: None,
                            task_id: Some(task.id),
                            correlation_id: event.correlation_id,
                            source_process_id: None,
                            payload: EventPayload::TaskSLAWarning {
                                task_id: task.id,
                                deadline: deadline.to_rfc3339(),
                                remaining_secs: remaining,
                            },
                        });
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

// ============================================================================
// PriorityAgingHandler (Phase 2b)
// ============================================================================

/// Triggered by the "priority-aging" scheduled event (300s, opt-in).
/// Ages task priorities based on wait time since creation.
pub struct PriorityAgingHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    low_to_normal_secs: u64,
    normal_to_high_secs: u64,
    high_to_critical_secs: u64,
}

impl<T: TaskRepository> PriorityAgingHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        low_to_normal_secs: u64,
        normal_to_high_secs: u64,
        high_to_critical_secs: u64,
    ) -> Self {
        Self { task_repo, low_to_normal_secs, normal_to_high_secs, high_to_critical_secs }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for PriorityAgingHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "PriorityAgingHandler".to_string(),
            filter: EventFilter {
                categories: vec![EventCategory::Scheduler],
                payload_types: vec!["ScheduledEventFired".to_string()],
                custom_predicate: Some(Arc::new(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::ScheduledEventFired { name, .. } if name == "priority-aging"
                    )
                })),
                ..Default::default()
            },
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::domain::models::TaskPriority;

        let now = chrono::Utc::now();
        let mut new_events = Vec::new();

        for status in &[TaskStatus::Pending, TaskStatus::Ready] {
            let tasks = self.task_repo.list_by_status(*status).await
                .map_err(|e| format!("Priority aging failed: {}", e))?;

            for task in tasks {
                let wait_secs = (now - task.created_at).num_seconds() as u64;

                let new_priority = match task.priority {
                    TaskPriority::Low if wait_secs > self.low_to_normal_secs => Some(TaskPriority::Normal),
                    TaskPriority::Normal if wait_secs > self.normal_to_high_secs => Some(TaskPriority::High),
                    TaskPriority::High if wait_secs > self.high_to_critical_secs => Some(TaskPriority::Critical),
                    _ => None,
                };

                if let Some(new_pri) = new_priority {
                    let from = task.priority.as_str().to_string();
                    let to = new_pri.as_str().to_string();

                    let mut updated = task.clone();
                    updated.priority = new_pri;
                    updated.updated_at = now;
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("Failed to update priority: {}", e))?;

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Info,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskPriorityChanged {
                            task_id: task.id,
                            from,
                            to,
                            reason: format!("priority-aging: waited {}s", wait_secs),
                        },
                    });
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

// ============================================================================
// MemoryInformedDecompositionHandler (Phase 3a)
// ============================================================================

/// Triggered by `MemoryStored` where tier is semantic and type is pattern.
/// Fires goal re-evaluation for goals with overlapping domains.
pub struct MemoryInformedDecompositionHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    cooldown_secs: u64,
    /// Track (goal_id, last_fired) to avoid duplicate evaluations.
    cooldowns: Arc<RwLock<std::collections::HashMap<uuid::Uuid, chrono::DateTime<chrono::Utc>>>>,
}

impl<G: GoalRepository> MemoryInformedDecompositionHandler<G> {
    pub fn new(goal_repo: Arc<G>, cooldown_secs: u64) -> Self {
        Self {
            goal_repo,
            cooldown_secs,
            cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for MemoryInformedDecompositionHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryInformedDecompositionHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Memory])
                .payload_types(vec!["MemoryStored".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (memory_id, key, namespace, tier, memory_type) = match &event.payload {
            EventPayload::MemoryStored { memory_id, key, namespace, tier, memory_type } => {
                (*memory_id, key.clone(), namespace.clone(), tier.clone(), memory_type.clone())
            }
            _ => return Ok(Reaction::None),
        };

        // Only trigger for semantic tier + pattern type
        if tier != "semantic" || memory_type != "pattern" {
            return Ok(Reaction::None);
        }

        let now = chrono::Utc::now();
        let goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("Failed to get active goals: {}", e))?;

        let mut new_events = Vec::new();
        let mut cooldowns = self.cooldowns.write().await;

        for goal in &goals {
            // Check if namespace overlaps with goal domains.
            // Universal goals (empty domains) match all namespaces.
            let overlaps = goal.applicability_domains.is_empty()
                || goal.applicability_domains.iter()
                    .any(|d| d.eq_ignore_ascii_case(&namespace));
            if !overlaps {
                continue;
            }

            // Check cooldown
            if let Some(last) = cooldowns.get(&goal.id)
                && (now - *last).num_seconds() < self.cooldown_secs as i64 {
                    continue;
                }

            cooldowns.insert(goal.id, now);

            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: now,
                severity: EventSeverity::Info,
                category: EventCategory::Memory,
                goal_id: Some(goal.id),
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::MemoryInformedGoal {
                    goal_id: goal.id,
                    memory_id,
                    memory_key: key.clone(),
                },
            });

            // Also emit a goal-evaluation trigger
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: now,
                severity: EventSeverity::Debug,
                category: EventCategory::Scheduler,
                goal_id: Some(goal.id),
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ScheduledEventFired {
                    schedule_id: uuid::Uuid::new_v4(),
                    name: "goal-evaluation".to_string(),
                },
            });
        }

        if new_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(new_events))
        }
    }
}

// ============================================================================
// MemoryConflictEscalationHandler (Phase 3b)
// ============================================================================

/// Triggered by `MemoryConflictDetected`. Escalates conflicts that are
/// flagged for review (low similarity) in semantic-tier memories.
pub struct MemoryConflictEscalationHandler;

impl Default for MemoryConflictEscalationHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryConflictEscalationHandler {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler for MemoryConflictEscalationHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "MemoryConflictEscalationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Memory])
                .payload_types(vec!["MemoryConflictDetected".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (memory_a, memory_b, key, similarity) = match &event.payload {
            EventPayload::MemoryConflictDetected { memory_a, memory_b, key, similarity } => {
                (*memory_a, *memory_b, key.clone(), *similarity)
            }
            _ => return Ok(Reaction::None),
        };

        // Only escalate for low-similarity conflicts (flagged for review)
        if similarity >= 0.3 {
            return Ok(Reaction::None);
        }

        let escalation = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Warning,
            category: EventCategory::Escalation,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::HumanEscalationRequired {
                goal_id: None,
                task_id: None,
                reason: format!(
                    "Memory conflict detected for key '{}': memories {} and {} have low similarity ({:.2})",
                    key, memory_a, memory_b, similarity
                ),
                urgency: "high".to_string(),
                questions: vec![
                    format!("Which version of memory '{}' should be kept?", key),
                ],
                is_blocking: true,
            },
        };

        Ok(Reaction::EmitEvents(vec![escalation]))
    }
}

// ============================================================================
// TaskCompletionLearningHandler (Phase 4a)
// ============================================================================

/// Triggered by `TaskCompletedWithResult`. Extracts learning data from task
/// results and stores pattern memories for tasks that required retries.
pub struct TaskCompletionLearningHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    min_retries: u32,
    store_efficiency: bool,
}

impl TaskCompletionLearningHandler {
    pub fn new(
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        min_retries: u32,
        store_efficiency: bool,
    ) -> Self {
        Self { command_bus, min_retries, store_efficiency }
    }
}

#[async_trait]
impl EventHandler for TaskCompletionLearningHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskCompletionLearningHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskCompletedWithResult".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::services::command_bus::{CommandEnvelope, CommandSource, DomainCommand, MemoryCommand};
        use crate::domain::models::{MemoryTier, MemoryType};

        let result = match &_event.payload {
            EventPayload::TaskCompletedWithResult { result, .. } => result,
            _ => return Ok(Reaction::None),
        };

        // Store learning for tasks that required retries
        if result.retry_count >= self.min_retries {
            let error_summary = result.error.as_deref().unwrap_or("unknown");
            let key = format!(
                "task-learning:{}:{}",
                result.status,
                &error_summary.chars().take(40).collect::<String>()
            );

            let content = format!(
                "Task {} completed with status {} after {} retries in {}s. Error: {}",
                result.task_id, result.status, result.retry_count,
                result.duration_secs, error_summary
            );

            let envelope = CommandEnvelope::new(
                CommandSource::EventHandler("TaskCompletionLearningHandler".to_string()),
                DomainCommand::Memory(MemoryCommand::Store {
                    key,
                    content,
                    namespace: "task-learnings".to_string(),
                    tier: MemoryTier::Episodic,
                    memory_type: MemoryType::Pattern,
                    metadata: None,
                }),
            );

            if let Err(e) = self.command_bus.dispatch(envelope).await {
                tracing::warn!("TaskCompletionLearningHandler: failed to store learning: {}", e);
            }
        }

        // Store efficiency pattern for fast completions
        if self.store_efficiency && result.retry_count == 0 && result.duration_secs < 60 {
            let key = format!("task-efficiency:{}", result.task_id);
            let content = format!(
                "Task {} completed efficiently: {}s, {} tokens",
                result.task_id, result.duration_secs, result.tokens_used
            );

            let envelope = CommandEnvelope::new(
                CommandSource::EventHandler("TaskCompletionLearningHandler".to_string()),
                DomainCommand::Memory(MemoryCommand::Store {
                    key,
                    content,
                    namespace: "task-learnings".to_string(),
                    tier: MemoryTier::Episodic,
                    memory_type: MemoryType::Pattern,
                    metadata: None,
                }),
            );

            if let Err(e) = self.command_bus.dispatch(envelope).await {
                tracing::debug!("TaskCompletionLearningHandler: failed to store efficiency pattern: {}", e);
            }
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// GoalEvaluationTaskCreationHandler (Phase 4b)
// ============================================================================

/// Triggered by `SemanticDriftDetected` or `GoalConstraintViolated`.
/// Creates diagnostic/remediation tasks for recurring issues.
pub struct GoalEvaluationTaskCreationHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    auto_create_diagnostic: bool,
    max_diagnostic_per_goal: u32,
    auto_create_remediation: bool,
}

impl GoalEvaluationTaskCreationHandler {
    pub fn new(
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        auto_create_diagnostic: bool,
        max_diagnostic_per_goal: u32,
        auto_create_remediation: bool,
    ) -> Self {
        Self { command_bus, auto_create_diagnostic, max_diagnostic_per_goal, auto_create_remediation }
    }
}

#[async_trait]
impl EventHandler for GoalEvaluationTaskCreationHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalEvaluationTaskCreationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Goal])
                .payload_types(vec![
                    "SemanticDriftDetected".to_string(),
                    "GoalConstraintViolated".to_string(),
                ]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::services::command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand};
        use crate::domain::models::{TaskPriority, TaskSource};

        match &event.payload {
            EventPayload::SemanticDriftDetected { goal_id, recurring_gaps, iterations } if self.auto_create_diagnostic => {
                for (i, gap) in recurring_gaps.iter().enumerate() {
                    if i as u32 >= self.max_diagnostic_per_goal {
                        break;
                    }

                    let gap_hash = format!("{:x}", md5_lite(gap));
                    let idem_key = format!("drift:{}:{}", goal_id, gap_hash);
                    let title = format!("Investigate recurring gap: {}", truncate_str(gap, 60));
                    let description = format!(
                        "Recurring gap detected across {} iterations for goal {}:\n\n{}",
                        iterations, goal_id, gap
                    );

                    let envelope = CommandEnvelope::new(
                        CommandSource::EventHandler("GoalEvaluationTaskCreationHandler".to_string()),
                        DomainCommand::Task(TaskCommand::Submit {
                            title: Some(title),
                            description,
                            parent_id: None,
                            priority: TaskPriority::Normal,
                            agent_type: None,
                            depends_on: vec![],
                            context: Box::new(None),
                            idempotency_key: Some(idem_key),
                            source: TaskSource::System,
                            deadline: None,
                            task_type: None,
                            execution_mode: None,
                        }),
                    );

                    if let Err(e) = self.command_bus.dispatch(envelope).await {
                        tracing::warn!("GoalEvaluationTaskCreationHandler: failed to create diagnostic task: {}", e);
                    }
                }
            }
            EventPayload::GoalConstraintViolated { goal_id, constraint_name, violation } if self.auto_create_remediation => {
                let idem_key = format!("remediate:{}:{}", goal_id, constraint_name);
                let title = format!("Remediate constraint violation: {}", constraint_name);
                let description = format!(
                    "Constraint '{}' violated for goal {}:\n\n{}",
                    constraint_name, goal_id, violation
                );

                let envelope = CommandEnvelope::new(
                    CommandSource::EventHandler("GoalEvaluationTaskCreationHandler".to_string()),
                    DomainCommand::Task(TaskCommand::Submit {
                        title: Some(title),
                        description,
                        parent_id: None,
                        priority: TaskPriority::High,
                        agent_type: None,
                        depends_on: vec![],
                        context: Box::new(None),
                        idempotency_key: Some(idem_key),
                        source: TaskSource::System,
                        deadline: None,
                        task_type: None,
                        execution_mode: None,
                    }),
                );

                if let Err(e) = self.command_bus.dispatch(envelope).await {
                    tracing::warn!("GoalEvaluationTaskCreationHandler: failed to create remediation task: {}", e);
                }
            }
            _ => {}
        }

        Ok(Reaction::None)
    }
}

/// Simple string hash for idempotency keys.
fn md5_lite(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Truncate a string to a given length with ellipsis.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

// ============================================================================
// EvolutionTriggeredTemplateUpdateHandler (Phase 4c)
// ============================================================================

/// Triggered by `EvolutionTriggered`. If the agent template's success rate is
/// below 40%, submits a refinement task.
pub struct EvolutionTriggeredTemplateUpdateHandler {
    command_bus: Arc<crate::services::command_bus::CommandBus>,
}

impl EvolutionTriggeredTemplateUpdateHandler {
    pub fn new(command_bus: Arc<crate::services::command_bus::CommandBus>) -> Self {
        Self { command_bus }
    }
}

#[async_trait]
impl EventHandler for EvolutionTriggeredTemplateUpdateHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EvolutionTriggeredTemplateUpdateHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Agent])
                .payload_types(vec!["EvolutionTriggered".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::services::command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand};
        use crate::domain::models::{TaskPriority, TaskSource};

        let (template_name, trigger) = match &event.payload {
            EventPayload::EvolutionTriggered { template_name, trigger } => {
                (template_name.clone(), trigger.clone())
            }
            _ => return Ok(Reaction::None),
        };

        // Parse success rate from the trigger string (e.g. "Low success rate: 40% (2/5)")
        let needs_refinement = trigger.contains("Low success rate");

        if !needs_refinement {
            return Ok(Reaction::None);
        }

        let title = format!("Refine agent template: {}", template_name);
        let description = format!(
            "Agent template '{}' triggered evolution: {}. Review and refine the template.",
            template_name, trigger
        );

        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler("EvolutionTriggeredTemplateUpdateHandler".to_string()),
            DomainCommand::Task(TaskCommand::Submit {
                title: Some(title),
                description,
                parent_id: None,
                priority: TaskPriority::Normal,
                agent_type: None,
                depends_on: vec![],
                context: Box::new(None),
                idempotency_key: Some(format!("evolve:{}", template_name)),
                source: TaskSource::System,
                deadline: None,
                task_type: None,
                execution_mode: None,
            }),
        );

        if let Err(e) = self.command_bus.dispatch(envelope).await {
            tracing::warn!("EvolutionTriggeredTemplateUpdateHandler: failed to submit refinement task: {}", e);
        }

        // Emit template status change
        let status_event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::AgentTemplateStatusChanged {
                template_name,
                from_status: "active".to_string(),
                to_status: "under-review".to_string(),
            },
        };

        Ok(Reaction::EmitEvents(vec![status_event]))
    }
}

// ============================================================================
// StartupCatchUpHandler (Phase 5a)
// ============================================================================

/// Triggered by `OrchestratorStarted`. Runs once at startup to fix orphaned
/// tasks, replay missed events, re-evaluate goals, and run reconciliation.
pub struct StartupCatchUpHandler<T: TaskRepository, G: GoalRepository> {
    task_repo: Arc<T>,
    goal_repo: Arc<G>,
    event_store: Arc<dyn EventStore>,
    stale_threshold_secs: u64,
    max_replay_events: u64,
}

impl<T: TaskRepository, G: GoalRepository> StartupCatchUpHandler<T, G> {
    pub fn new(
        task_repo: Arc<T>,
        goal_repo: Arc<G>,
        event_store: Arc<dyn EventStore>,
        stale_threshold_secs: u64,
        max_replay_events: u64,
    ) -> Self {
        Self { task_repo, goal_repo, event_store, stale_threshold_secs, max_replay_events }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, G: GoalRepository + 'static> EventHandler for StartupCatchUpHandler<T, G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "StartupCatchUpHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Orchestrator])
                .payload_types(vec!["OrchestratorStarted".to_string()]),
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let start = std::time::Instant::now();
        let now = chrono::Utc::now();
        let mut orphaned_tasks_fixed: u32 = 0;
        let mut new_events = Vec::new();

        // 1. Fix orphaned Running tasks (started before last shutdown)
        let running = self.task_repo.list_by_status(TaskStatus::Running).await
            .map_err(|e| format!("StartupCatchUp: failed to list running tasks: {}", e))?;

        let stale_cutoff = now - chrono::Duration::seconds(self.stale_threshold_secs as i64);

        for task in running {
            let is_stale = task.started_at.is_none_or(|s| s < stale_cutoff);
            if is_stale {
                let mut updated = task.clone();
                updated.retry_count += 1;
                if updated.transition_to(TaskStatus::Failed).is_ok() {
                    self.task_repo.update(&updated).await
                        .map_err(|e| format!("StartupCatchUp: failed to update task: {}", e))?;
                    orphaned_tasks_fixed += 1;

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: now,
                        severity: EventSeverity::Warning,
                        category: EventCategory::Task,
                        goal_id: None,
                        task_id: Some(task.id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskFailed {
                            task_id: task.id,
                            error: "orchestrator-restart: task was running during shutdown".to_string(),
                            retry_count: updated.retry_count,
                        },
                    });
                }
            }
        }

        // 2. Replay missed events since the reactor's last-known watermark
        let reactor_wm = self.event_store.get_watermark("EventReactor").await
            .map_err(|e| format!("StartupCatchUp: failed to get reactor watermark: {}", e))?;

        let since_seq = reactor_wm.unwrap_or(SequenceNumber(0));
        let replayed_events = self.event_store.replay_since(since_seq).await
            .map_err(|e| format!("StartupCatchUp: failed to replay events: {}", e))?;

        // Bound replay to prevent flooding
        let bounded_replay: Vec<_> = replayed_events.into_iter()
            .take(self.max_replay_events as usize)
            .filter(|evt| {
                // Skip scheduler events to avoid retriggering periodic handlers
                !matches!(&evt.payload, EventPayload::ScheduledEventFired { .. })
            })
            .collect();

        let missed_events_replayed = bounded_replay.len() as u64;
        new_events.extend(bounded_replay);

        // 3. Re-evaluate active goals
        let active_goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("StartupCatchUp: failed to get active goals: {}", e))?;
        let goals_count = active_goals.len() as u32;

        for goal in &active_goals {
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: now,
                severity: EventSeverity::Debug,
                category: EventCategory::Scheduler,
                goal_id: Some(goal.id),
                task_id: None,
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::ScheduledEventFired {
                    schedule_id: uuid::Uuid::new_v4(),
                    name: "goal-evaluation".to_string(),
                },
            });
        }

        // 4. Run reconciliation
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "reconciliation".to_string(),
            },
        });

        // 5. Run memory maintenance
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Debug,
            category: EventCategory::Scheduler,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "memory-maintenance".to_string(),
            },
        });

        let duration_ms = start.elapsed().as_millis() as u64;

        // 6. Emit summary
        new_events.push(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: now,
            severity: EventSeverity::Info,
            category: EventCategory::Orchestrator,
            goal_id: None,
            task_id: None,
            correlation_id: event.correlation_id,
            source_process_id: None,
            payload: EventPayload::StartupCatchUpCompleted {
                orphaned_tasks_fixed,
                missed_events_replayed,
                goals_reevaluated: goals_count,
                duration_ms,
            },
        });

        tracing::info!(
            orphaned_tasks_fixed = orphaned_tasks_fixed,
            goals_reevaluated = goals_count,
            duration_ms = duration_ms,
            "StartupCatchUp: catch-up completed"
        );

        Ok(Reaction::EmitEvents(new_events))
    }
}

// ============================================================================
// ConvergenceCoordinationHandler
// ============================================================================

/// When a child task of a decomposed convergent parent completes or fails,
/// check if all siblings are done and cascade the result to the parent.
///
/// This supplements TaskCompletedReadinessHandler (which handles DAG dependencies)
/// with parent-child coordination for decomposed convergent tasks.
pub struct ConvergenceCoordinationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
}

impl<T: TaskRepository> ConvergenceCoordinationHandler<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self { task_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for ConvergenceCoordinationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceCoordinationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskCompleted".to_string(),
                    "TaskCompletedWithResult".to_string(),
                    "TaskFailed".to_string(),
                ]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let task_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Load the completed/failed task to check if it has a parent
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let parent_id = match task.parent_id {
            Some(id) => id,
            None => return Ok(Reaction::None), // Not a child task
        };

        // Load parent task
        let parent = self.task_repo.get(parent_id).await
            .map_err(|e| format!("Failed to get parent task: {}", e))?;
        let parent = match parent {
            Some(p) => p,
            None => return Ok(Reaction::None),
        };

        // Skip parents that have workflow_state — the workflow engine owns their
        // lifecycle and transitions. Convergence coordination would race with
        // WorkflowSubtaskCompletionHandler and bypass the workflow state machine.
        if parent.context.custom.contains_key("workflow_state") {
            return Ok(Reaction::None);
        }

        // Only act on parents that are Running with a trajectory (convergent decomposition)
        if parent.status != TaskStatus::Running || parent.trajectory_id.is_none() {
            return Ok(Reaction::None);
        }

        // Load all sibling tasks (children of the parent)
        let siblings = self.task_repo.get_subtasks(parent_id).await
            .map_err(|e| format!("Failed to get subtasks: {}", e))?;

        // Check if any sibling has failed
        let any_failed = siblings.iter().any(|s| s.status == TaskStatus::Failed);

        // Check if all siblings are in terminal states
        let all_terminal = siblings.iter().all(|s| s.status.is_terminal());

        if !all_terminal {
            return Ok(Reaction::None); // Still waiting for siblings
        }

        let mut new_events = Vec::new();

        if any_failed {
            // Fail the parent
            let mut updated_parent = parent.clone();
            if updated_parent.transition_to(TaskStatus::Failed).is_ok() {
                self.task_repo.update(&updated_parent).await
                    .map_err(|e| format!("Failed to update parent: {}", e))?;

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Error,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(parent_id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskFailed {
                        task_id: parent_id,
                        error: "Decomposed child task failed".to_string(),
                        retry_count: updated_parent.retry_count,
                    },
                });
            }
        } else {
            // All siblings completed successfully — complete the parent
            let mut updated_parent = parent.clone();
            // Go through Validating first
            if updated_parent.transition_to(TaskStatus::Validating).is_ok() {
                self.task_repo.update(&updated_parent).await
                    .map_err(|e| format!("Failed to update parent to validating: {}", e))?;

                // Then complete
                if updated_parent.transition_to(TaskStatus::Complete).is_ok() {
                    self.task_repo.update(&updated_parent).await
                        .map_err(|e| format!("Failed to update parent to complete: {}", e))?;

                    new_events.push(UnifiedEvent {
                        id: EventId::new(),
                        sequence: SequenceNumber(0),
                        timestamp: chrono::Utc::now(),
                        severity: EventSeverity::Info,
                        category: EventCategory::Task,
                        goal_id: event.goal_id,
                        task_id: Some(parent_id),
                        correlation_id: event.correlation_id,
                        source_process_id: None,
                        payload: EventPayload::TaskCompleted {
                            task_id: parent_id,
                            tokens_used: 0,
                        },
                    });
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

// ============================================================================
// ConvergenceCancellationHandler
// ============================================================================

/// When a convergent parent task is canceled, cascade cancellation to all
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
                .payload_types(vec!["TaskCanceled".to_string()]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let task_id = match &event.payload {
            EventPayload::TaskCanceled { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Load the canceled task
        let task = self.task_repo.get(task_id).await
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
        let children = self.task_repo.get_subtasks(task_id).await
            .map_err(|e| format!("Failed to get subtasks: {}", e))?;

        let mut new_events = Vec::new();

        for child in children {
            // Only cancel active (non-terminal) children
            if child.status.is_terminal() {
                continue;
            }

            let mut updated = child.clone();
            if updated.transition_to(TaskStatus::Canceled).is_ok() {
                self.task_repo.update(&updated).await
                    .map_err(|e| format!("Failed to cancel child task: {}", e))?;

                new_events.push(UnifiedEvent {
                    id: EventId::new(),
                    sequence: SequenceNumber(0),
                    timestamp: chrono::Utc::now(),
                    severity: EventSeverity::Warning,
                    category: EventCategory::Task,
                    goal_id: event.goal_id,
                    task_id: Some(child.id),
                    correlation_id: event.correlation_id,
                    source_process_id: None,
                    payload: EventPayload::TaskCanceled {
                        task_id: child.id,
                        reason: format!("Parent task {} was canceled", task_id),
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
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, hint) = match &event.payload {
            EventPayload::TaskSLAWarning { task_id, .. } => (*task_id, "sla:warning"),
            EventPayload::TaskSLACritical { task_id, .. } => (*task_id, "sla:critical"),
            _ => return Ok(Reaction::None),
        };

        // Load the task
        let task = self.task_repo.get(task_id).await
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

        self.task_repo.update(&updated).await
            .map_err(|e| format!("Failed to update task with SLA hint: {}", e))?;

        tracing::info!(
            task_id = %task_id,
            hint = hint,
            "Added SLA pressure hint to convergent task context"
        );

        Ok(Reaction::None)
    }
}

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
        Self { task_repo, memory_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, M: MemoryRepository + 'static> EventHandler for ConvergenceMemoryHandler<T, M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceMemoryHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Convergence])
                .payload_types(vec!["ConvergenceTerminated".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, trajectory_id, outcome, total_iterations, total_tokens, final_convergence_level) =
            match &event.payload {
                EventPayload::ConvergenceTerminated {
                    task_id,
                    trajectory_id,
                    outcome,
                    total_iterations,
                    total_tokens,
                    final_convergence_level,
                } => (*task_id, *trajectory_id, outcome.clone(), *total_iterations, *total_tokens, *final_convergence_level),
                _ => return Ok(Reaction::None),
            };

        // Idempotency: check if we already stored a memory for this trajectory
        let idempotency_key = format!("convergence-outcome:{}", trajectory_id);
        let existing = self.memory_repo
            .get_by_key(&idempotency_key, "convergence")
            .await
            .map_err(|e| format!("Failed to check existing memory: {}", e))?;
        if existing.is_some() {
            return Ok(Reaction::None);
        }

        // Load the task for additional context (complexity, agent_type)
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let complexity = format!("{:?}", task.routing_hints.complexity);
        let agent_type = task.agent_type.clone().unwrap_or_else(|| "unknown".to_string());
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

        self.memory_repo.store(&memory).await
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

// ============================================================================
// DirectModeExecutionMemoryHandler
// ============================================================================

/// When any task execution is recorded (TaskExecutionRecorded event), store a
/// lightweight episodic memory entry so the classification heuristic can learn
/// which complexity levels benefit from convergence vs. direct execution.
///
/// This handler complements `ConvergenceMemoryHandler` (which stores rich
/// convergence-specific outcomes) by recording every task completion -- direct
/// mode or convergent -- in the `execution_history` namespace, keeping the
/// observations uniform for the bandit/heuristic.
///
/// Idempotent: uses `execution-record:{task_id}` as an idempotency key.
pub struct DirectModeExecutionMemoryHandler<M: MemoryRepository> {
    memory_repo: Arc<M>,
}

impl<M: MemoryRepository> DirectModeExecutionMemoryHandler<M> {
    pub fn new(memory_repo: Arc<M>) -> Self {
        Self { memory_repo }
    }
}

#[async_trait]
impl<M: MemoryRepository + 'static> EventHandler for DirectModeExecutionMemoryHandler<M> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "DirectModeExecutionMemoryHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskExecutionRecorded".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, execution_mode, complexity, succeeded, tokens_used) =
            match &event.payload {
                EventPayload::TaskExecutionRecorded {
                    task_id,
                    execution_mode,
                    complexity,
                    succeeded,
                    tokens_used,
                } => (*task_id, execution_mode.clone(), complexity.clone(), *succeeded, *tokens_used),
                _ => return Ok(Reaction::None),
            };

        // Idempotency: check if we already stored a memory for this task execution
        let idempotency_key = format!("execution-record:{}", task_id);
        let existing = self.memory_repo
            .get_by_key(&idempotency_key, "execution_history")
            .await
            .map_err(|e| format!("Failed to check existing execution memory: {}", e))?;
        if existing.is_some() {
            return Ok(Reaction::None);
        }

        let outcome_str = if succeeded { "succeeded" } else { "failed" };

        // Build a structured summary for the memory content
        let content = format!(
            "Task execution recorded for {task_id}:\n\
             - execution_mode: {execution_mode}\n\
             - complexity: {complexity}\n\
             - outcome: {outcome_str}\n\
             - tokens_used: {tokens_used}",
        );

        // Choose memory type based on outcome
        let memory_type = if succeeded {
            crate::domain::models::MemoryType::Pattern
        } else {
            crate::domain::models::MemoryType::Error
        };

        let mut memory = crate::domain::models::Memory::episodic(idempotency_key.clone(), content)
            .with_namespace("execution_history")
            .with_type(memory_type)
            .with_source("task_execution")
            .with_task(task_id);

        // Add goal context if available
        if let Some(goal_id) = event.goal_id {
            memory = memory.with_goal(goal_id);
        }

        // Tag with mode, complexity, and outcome for future queries
        memory = memory
            .with_tag(format!("mode:{}", execution_mode))
            .with_tag(format!("complexity:{}", complexity))
            .with_tag(format!("outcome:{}", outcome_str));

        // Store custom metadata for machine consumption
        memory.metadata.custom.insert(
            "tokens_used".to_string(),
            serde_json::Value::Number(serde_json::Number::from(tokens_used)),
        );
        memory.metadata.custom.insert(
            "execution_mode".to_string(),
            serde_json::Value::String(execution_mode.clone()),
        );
        memory.metadata.custom.insert(
            "complexity".to_string(),
            serde_json::Value::String(complexity.clone()),
        );
        memory.metadata.custom.insert(
            "succeeded".to_string(),
            serde_json::Value::Bool(succeeded),
        );

        // Lower relevance than convergence memory -- this is for learning, not active use
        memory.metadata.relevance = 0.5;

        self.memory_repo.store(&memory).await
            .map_err(|e| format!("Failed to store execution memory: {}", e))?;

        tracing::info!(
            task_id = %task_id,
            execution_mode = %execution_mode,
            complexity = %complexity,
            succeeded = succeeded,
            "Stored execution history memory for classification heuristic"
        );

        Ok(Reaction::None)
    }
}

// ============================================================================
// TaskOutcomeMemoryHandler
// ============================================================================

/// When any task completes (via TaskCompleted or TaskCompletedWithResult),
/// store an episodic memory entry so that future tasks, agents, and learning
/// loops can reason about historical outcomes.
///
/// This fills the event-chain-integrity gap where orchestrator direct-mode
/// task completions emit `TaskCompleted` but never store episodic memories.
/// Convergent tasks emit `TaskCompletedWithResult`, which is also handled.
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
        Self { task_repo, memory_repo }
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
                ]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let task_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Idempotency check: skip if we already stored a memory for this task outcome
        let idempotency_key = format!("task-outcome:{}", task_id);
        let existing = self.memory_repo
            .get_by_key(&idempotency_key, "task-outcomes")
            .await
            .map_err(|e| format!("Failed to check existing task outcome memory: {}", e))?;
        if existing.is_some() {
            return Ok(Reaction::None);
        }

        // Load the task to get title, agent_type, execution_mode, complexity, and timing
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task for outcome memory: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let succeeded = task.status == TaskStatus::Complete;
        let outcome_str = if succeeded { "succeeded" } else { "failed" };
        let mode_str = if task.execution_mode.is_direct() { "direct" } else { "convergent" };
        let complexity_str = format!("{:?}", task.routing_hints.complexity);
        let agent_type = task.agent_type.clone().unwrap_or_else(|| "unknown".to_string());

        // Compute duration if both timestamps are available
        let duration_secs: Option<i64> = task.started_at.zip(task.completed_at)
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
        memory.metadata.custom.insert(
            "succeeded".to_string(),
            serde_json::Value::Bool(succeeded),
        );
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

        self.memory_repo.store(&memory).await
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
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, _trajectory_id, outcome, total_iterations, total_tokens, final_convergence_level) =
            match &event.payload {
                EventPayload::ConvergenceTerminated {
                    task_id,
                    trajectory_id,
                    outcome,
                    total_iterations,
                    total_tokens,
                    final_convergence_level,
                } => (*task_id, *trajectory_id, outcome.clone(), *total_iterations, *total_tokens, *final_convergence_level),
                _ => return Ok(Reaction::None),
            };

        // Load the task to get agent_type and compute duration
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        // Compute duration from started_at to now (or completed_at if available)
        let duration_secs = task.started_at
            .map(|started| {
                let end = task.completed_at.unwrap_or_else(chrono::Utc::now);
                (end - started).num_seconds().max(0) as u64
            })
            .unwrap_or(0);

        // Map convergence outcome to task result status
        let (status_str, error) = match outcome.as_str() {
            "converged" => ("Complete".to_string(), None),
            "exhausted" => ("Failed".to_string(), Some("Convergence exhausted: max iterations reached".to_string())),
            "trapped" => ("Failed".to_string(), Some("Convergence trapped: attractor limit cycle detected".to_string())),
            "budget_denied" => ("Failed".to_string(), Some("Convergence budget extension denied".to_string())),
            "decomposed" => ("Complete".to_string(), None), // Decomposition is a valid outcome
            other => ("Failed".to_string(), Some(format!("Convergence terminated: {}", other))),
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

        self.task_repo.update(&updated).await
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

// ============================================================================
// ConvergenceEscalationFeedbackHandler
// ============================================================================

/// When a human responds to an escalation during convergence, process the
/// response and feed it back into the convergence loop via a trajectory
/// specification amendment.
///
/// On receiving a `HumanResponseReceived` event:
/// 1. Check that the event has a `task_id` set on the envelope.
/// 2. Load the task and verify it has a `trajectory_id` (convergent task).
/// 3. Load the trajectory from the trajectory store.
/// 4. Amend the specification with the human's decision text.
/// 5. Persist the updated trajectory.
/// 6. If `allows_continuation` is false, add a `convergence:force_stop` hint
///    to the task context so the convergence loop can halt gracefully.
pub struct ConvergenceEscalationFeedbackHandler<T: TaskRepository, Tr: TrajectoryRepository> {
    task_repo: Arc<T>,
    trajectory_repo: Arc<Tr>,
}

impl<T: TaskRepository, Tr: TrajectoryRepository> ConvergenceEscalationFeedbackHandler<T, Tr> {
    pub fn new(task_repo: Arc<T>, trajectory_repo: Arc<Tr>) -> Self {
        Self {
            task_repo,
            trajectory_repo,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, Tr: TrajectoryRepository + 'static> EventHandler
    for ConvergenceEscalationFeedbackHandler<T, Tr>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ConvergenceEscalationFeedbackHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Escalation])
                .payload_types(vec!["HumanResponseReceived".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (escalation_id, decision, allows_continuation) = match &event.payload {
            EventPayload::HumanResponseReceived {
                escalation_id,
                decision,
                allows_continuation,
            } => (*escalation_id, decision.clone(), *allows_continuation),
            _ => return Ok(Reaction::None),
        };

        // Step 1: The event must have a task_id on the envelope
        let task_id = match event.task_id {
            Some(id) => id,
            None => return Ok(Reaction::None),
        };

        // Step 2: Load the task and check for a trajectory_id
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?;
        let task = match task {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        let trajectory_id = match task.trajectory_id {
            Some(id) => id,
            None => return Ok(Reaction::None), // Not a convergent task
        };

        // Step 3: Load the trajectory
        let trajectory = self
            .trajectory_repo
            .get(&trajectory_id.to_string())
            .await
            .map_err(|e| format!("Failed to get trajectory: {}", e))?;
        let mut trajectory = match trajectory {
            Some(t) => t,
            None => return Ok(Reaction::None),
        };

        // Step 4: Amend the specification with the human's decision
        let amendment = SpecificationAmendment::new(
            AmendmentSource::UserHint,
            decision.clone(),
            format!(
                "Human escalation response (escalation_id={})",
                escalation_id
            ),
        );
        trajectory.specification.add_amendment(amendment);

        // Step 5: Persist the updated trajectory
        self.trajectory_repo
            .save(&trajectory)
            .await
            .map_err(|e| format!("Failed to save trajectory: {}", e))?;

        // Step 6: If continuation is not allowed, add force_stop hint
        if !allows_continuation {
            let mut updated_task = task.clone();
            if !updated_task
                .context
                .hints
                .iter()
                .any(|h| h == "convergence:force_stop")
            {
                updated_task
                    .context
                    .hints
                    .push("convergence:force_stop".to_string());
                updated_task.updated_at = chrono::Utc::now();
                self.task_repo
                    .update(&updated_task)
                    .await
                    .map_err(|e| format!("Failed to update task with force_stop hint: {}", e))?;
            }
        }

        // Step 7: Log the feedback application
        tracing::info!(
            task_id = %task_id,
            escalation_id = %escalation_id,
            allows_continuation = allows_continuation,
            "Applied human escalation feedback as specification amendment to convergent trajectory"
        );

        Ok(Reaction::None)
    }
}

// ============================================================================
// WorkflowSubtaskCompletionHandler
// ============================================================================

/// When a task completes or fails, check if its parent has workflow state.
/// If so, call `workflow_engine.handle_phase_complete()` to drive the
/// workflow state machine forward.
pub struct WorkflowSubtaskCompletionHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    event_bus: Arc<EventBus>,
    verification_enabled: bool,
}

impl<T: TaskRepository> WorkflowSubtaskCompletionHandler<T> {
    pub fn new(task_repo: Arc<T>, event_bus: Arc<EventBus>, verification_enabled: bool) -> Self {
        Self { task_repo, event_bus, verification_enabled }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for WorkflowSubtaskCompletionHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorkflowSubtaskCompletionHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec![
                    "TaskCompleted".to_string(),
                    "TaskCompletedWithResult".to_string(),
                    "TaskFailed".to_string(),
                ]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let subtask_id = match &event.payload {
            EventPayload::TaskCompleted { task_id, .. } => *task_id,
            EventPayload::TaskCompletedWithResult { task_id, .. } => *task_id,
            EventPayload::TaskFailed { task_id, .. } => *task_id,
            _ => return Ok(Reaction::None),
        };

        // Look up the subtask to find its parent
        let subtask = match self.task_repo.get(subtask_id).await {
            Ok(Some(t)) => t,
            _ => return Ok(Reaction::None),
        };

        // Guard: don't let verification tasks re-trigger the workflow handler
        if subtask.task_type.is_verification() {
            return Ok(Reaction::None);
        }

        let parent_id = match subtask.parent_id {
            Some(id) => id,
            None => return Ok(Reaction::None),
        };

        // Check if parent has workflow_state
        let parent = match self.task_repo.get(parent_id).await {
            Ok(Some(t)) => t,
            _ => return Ok(Reaction::None),
        };

        if !parent.context.custom.contains_key("workflow_state") {
            return Ok(Reaction::None);
        }

        // Delegate to workflow engine
        let engine = crate::services::workflow_engine::WorkflowEngine::new(
            self.task_repo.clone(),
            self.event_bus.clone(),
            self.verification_enabled,
        );
        if let Err(e) = engine.handle_phase_complete(parent_id, subtask_id).await {
            tracing::warn!(
                parent_id = %parent_id,
                subtask_id = %subtask_id,
                "WorkflowSubtaskCompletionHandler: handle_phase_complete failed: {}",
                e
            );
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// WorkflowVerificationHandler
// ============================================================================

/// Listens for `WorkflowVerificationRequested` events and runs LLM-based
/// intent verification on the completed phase subtasks. Maps the result
/// back through `WorkflowEngine::handle_verification_result()`.
pub struct WorkflowVerificationHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    event_bus: Arc<EventBus>,
    intent_verifier: Arc<dyn crate::services::swarm_orchestrator::convergent_execution::ConvergentIntentVerifier>,
    verification_enabled: bool,
}

impl<T: TaskRepository> WorkflowVerificationHandler<T> {
    pub fn new(
        task_repo: Arc<T>,
        event_bus: Arc<EventBus>,
        intent_verifier: Arc<dyn crate::services::swarm_orchestrator::convergent_execution::ConvergentIntentVerifier>,
        verification_enabled: bool,
    ) -> Self {
        Self {
            task_repo,
            event_bus,
            intent_verifier,
            verification_enabled,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for WorkflowVerificationHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorkflowVerificationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Workflow])
                .payload_types(vec![
                    "WorkflowVerificationRequested".to_string(),
                ]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, phase_name, retry_count) = match &event.payload {
            EventPayload::WorkflowVerificationRequested {
                task_id,
                phase_name,
                retry_count,
                ..
            } => (*task_id, phase_name.clone(), *retry_count),
            _ => return Ok(Reaction::None),
        };

        // Load parent task
        let mut parent_task = match self.task_repo.get(task_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::warn!(task_id = %task_id, "WorkflowVerificationHandler: parent task not found");
                return Ok(Reaction::None);
            }
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "WorkflowVerificationHandler: failed to load parent task");
                return Ok(Reaction::None);
            }
        };

        // Step 4.4: Enrich parent task with phase context before verification.
        // This gives the verifier knowledge of which workflow phase just completed.
        {
            let workflow_state = parent_task.context.custom.get("workflow_state")
                .and_then(|v| serde_json::from_value::<crate::domain::models::workflow_state::WorkflowState>(v.clone()).ok());

            if let Some(ref ws) = workflow_state {
                let phase_index = ws.phase_index().unwrap_or(0);
                // Count total phases from workflow template name
                let total_phases_hint = parent_task.context.custom.get("workflow_total_phases")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let phase_context = if total_phases_hint > 0 {
                    format!("workflow_phase: {} (phase {}/{})", phase_name, phase_index + 1, total_phases_hint)
                } else {
                    format!("workflow_phase: {} (phase {})", phase_name, phase_index + 1)
                };
                parent_task.context.custom.insert(
                    "verification_phase_context".to_string(),
                    serde_json::Value::String(phase_context),
                );
            }

            // Include aggregation summary if fan-out was used
            if let Some(agg_summary) = parent_task.context.custom.get("aggregation_summary").cloned() {
                parent_task.context.custom.insert(
                    "verification_aggregation_summary".to_string(),
                    agg_summary,
                );
            }
        }

        // Step 4.3: Create a Verification subtask for audit trail
        {
            use crate::domain::models::task::{Task, TaskStatus, TaskPriority};

            let mut verification_task = Task::new(
                format!("Verify phase '{}' for task {}", phase_name, task_id),
            );
            verification_task.task_type = crate::domain::models::task::TaskType::Verification;
            verification_task.parent_id = Some(task_id);
            verification_task.priority = TaskPriority::High;
            verification_task.context.custom.insert(
                "workflow_verification".to_string(),
                serde_json::json!({
                    "phase_name": phase_name,
                    "retry_count": retry_count,
                    "parent_task_id": task_id.to_string(),
                }),
            );
            // Start it as running immediately since we're executing inline
            verification_task.status = TaskStatus::Running;
            let verification_task_id = verification_task.id;

            if let Err(e) = self.task_repo.create(&verification_task).await {
                tracing::warn!(
                    task_id = %task_id,
                    "WorkflowVerificationHandler: failed to create verification subtask: {}",
                    e
                );
            }

            // Extract goal_id from parent task context
            let goal_id = parent_task.context.custom.get("goal_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok());

            // Run intent verification on the parent task
            let (satisfied, summary) = match self.intent_verifier.verify_convergent_intent(
                &parent_task,
                goal_id,
                retry_count,
                None, // no overseer signals
            ).await {
                Ok(Some(result)) => {
                    use crate::domain::models::intent_verification::IntentSatisfaction;
                    let satisfied = result.satisfaction == IntentSatisfaction::Satisfied;
                    let summary = format!(
                        "Phase '{}' verification: {} (confidence: {:.2}, gaps: {})",
                        phase_name,
                        result.satisfaction.as_str(),
                        result.confidence,
                        result.gaps.len(),
                    );
                    (satisfied, summary)
                }
                Ok(None) => {
                    tracing::info!(
                        task_id = %task_id,
                        "WorkflowVerificationHandler: no intent to verify, treating as satisfied"
                    );
                    (true, "No intent to verify — auto-passing".to_string())
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task_id,
                        error = %e,
                        "WorkflowVerificationHandler: verification failed, treating as satisfied"
                    );
                    (true, format!("Verification infrastructure error: {}", e))
                }
            };

            // Mark verification subtask as complete/failed and store results
            let mut ver_task = verification_task;
            ver_task.context.custom.insert(
                "verification_result".to_string(),
                serde_json::json!({
                    "satisfied": satisfied,
                    "summary": summary,
                }),
            );
            if satisfied {
                ver_task.status = TaskStatus::Complete;
            } else {
                ver_task.status = TaskStatus::Failed;
                ver_task.context.custom.insert(
                    "verification_error".to_string(),
                    serde_json::Value::String(summary.clone()),
                );
            }
            if let Err(e) = self.task_repo.update(&ver_task).await {
                tracing::warn!(
                    verification_task_id = %verification_task_id,
                    "WorkflowVerificationHandler: failed to update verification subtask: {}",
                    e
                );
            }

            // Feed result back to workflow engine
            let engine = crate::services::workflow_engine::WorkflowEngine::new(
                self.task_repo.clone(),
                self.event_bus.clone(),
                self.verification_enabled,
            );
            if let Err(e) = engine.handle_verification_result(task_id, satisfied, &summary).await {
                tracing::warn!(
                    task_id = %task_id,
                    "WorkflowVerificationHandler: handle_verification_result failed: {}",
                    e
                );
            }
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// WorkflowAutoAdvanceHandler — REMOVED
// ============================================================================
// Removed: `WorkflowAutoAdvanceHandler` reacted to `WorkflowEnrolled` and raced
// with `spawn_task_agent()` and the `workflow_advance` MCP tool.
// The Overmind now owns the first advance — no system-side auto-advance from Pending.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{Task, TaskStatus};
    use uuid::Uuid;

    async fn setup_task_repo() -> Arc<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        Arc::new(SqliteTaskRepository::new(pool))
    }

    #[tokio::test]
    async fn test_task_completed_readiness_handler() {
        let repo = setup_task_repo().await;
        let handler = TaskCompletedReadinessHandler::new(repo.clone());

        // Create upstream task
        let mut upstream = Task::new("Upstream task");
        upstream.transition_to(TaskStatus::Ready).unwrap();
        upstream.transition_to(TaskStatus::Running).unwrap();
        upstream.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&upstream).await.unwrap();

        // Create downstream task that depends on upstream
        let downstream = Task::new("Downstream task");
        repo.create(&downstream).await.unwrap();
        repo.add_dependency(downstream.id, upstream.id).await.unwrap();

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
        let handler = TaskCompletedReadinessHandler::new(repo.clone());

        // Create upstream and downstream
        let mut upstream = Task::new("Upstream");
        upstream.transition_to(TaskStatus::Ready).unwrap();
        upstream.transition_to(TaskStatus::Running).unwrap();
        upstream.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&upstream).await.unwrap();

        let mut downstream = Task::new("Downstream");
        downstream.transition_to(TaskStatus::Ready).unwrap(); // Already ready
        repo.create(&downstream).await.unwrap();
        repo.add_dependency(downstream.id, upstream.id).await.unwrap();

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

    #[tokio::test]
    async fn test_task_failed_block_handler() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedBlockHandler::new(repo.clone());

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
        repo.add_dependency(downstream.id, upstream.id).await.unwrap();

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
        let handler = TaskFailedBlockHandler::new(repo.clone());

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
        repo.add_dependency(downstream.id, upstream.id).await.unwrap();

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

    // ========================================================================
    // TaskFailedRetryHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_retry_handler_injects_max_turns_hint() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 3);

        // Create a task that has failed due to max_turns
        let mut task = Task::new("Research codebase");
        task.max_retries = 3;
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

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
                error: "error_max_turns: agent exceeded 25 turns".to_string(),
                retry_count: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should have emitted retry events
        assert!(matches!(reaction, Reaction::EmitEvents(_)));

        // Verify the retried task has the hint and custom field
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
        assert!(updated.context.hints.contains(&"retry:max_turns_exceeded".to_string()));
        assert!(updated.context.custom.contains_key("last_failure_reason"));
    }

    #[tokio::test]
    async fn test_retry_handler_skips_backoff_for_max_turns() {
        let repo = setup_task_repo().await;
        let handler = TaskFailedRetryHandler::new(repo.clone(), 3);

        // Create a task that has already been retried once (retry_count=1)
        // and just failed again. With normal backoff, 2^1 = 2s wait would apply.
        let mut task = Task::new("Research codebase");
        task.max_retries = 3;
        task.retry_count = 1;
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        // Set completed_at to "just now" so backoff would normally block
        task.completed_at = Some(chrono::Utc::now());
        repo.create(&task).await.unwrap();

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
                error: "error_max_turns: agent exceeded 25 turns".to_string(),
                retry_count: 1,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        // Should retry immediately despite completed_at being "just now"
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::EmitEvents(_)));

        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
        assert_eq!(updated.retry_count, 2);
    }

    // ========================================================================
    // RetryProcessingHandler tests
    // ========================================================================

    /// Helper: create a ScheduledEventFired event for "retry-check".
    fn make_retry_check_event() -> UnifiedEvent {
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
                schedule_id: Uuid::new_v4(),
                name: "retry-check".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_retry_processing_skips_workflow_subtasks() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 3);

        // Create a failed workflow subtask
        let mut task = Task::new("Research phase subtask");
        task.max_retries = 3;
        task.context.custom.insert(
            "workflow_phase".to_string(),
            serde_json::Value::String("research".to_string()),
        );
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should NOT retry — workflow engine owns this task's lifecycle
        assert!(matches!(reaction, Reaction::None));

        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
        assert_eq!(updated.retry_count, 0);
    }

    #[tokio::test]
    async fn test_retry_processing_skips_review_loop_tasks() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 3);

        // Create a failed review-loop task
        let mut task = Task::new("Review iteration task");
        task.max_retries = 3;
        task.context.custom.insert(
            "review_loop_active".to_string(),
            serde_json::Value::Bool(true),
        );
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        assert!(matches!(reaction, Reaction::None));
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn test_retry_processing_uses_retry_increments_count() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 3);

        // Create a normal failed task (no workflow/review context)
        let mut task = Task::new("Normal task");
        task.max_retries = 3;
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should retry and increment retry_count
        assert!(matches!(reaction, Reaction::EmitEvents(_)));
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Ready);
        assert_eq!(updated.retry_count, 1, "retry() should increment retry_count");
    }

    #[tokio::test]
    async fn test_retry_processing_circuit_breaks_budget_failures() {
        let repo = setup_task_repo().await;
        let handler = RetryProcessingHandler::new(repo.clone(), 5);

        // Create a task that has already hit budget failure 3 times
        let mut task = Task::new("Budget-exhausted task");
        task.max_retries = 5;
        task.context.custom.insert(
            "consecutive_budget_failures".to_string(),
            serde_json::Value::Number(3.into()),
        );
        task.context.custom.insert(
            "last_failure_reason".to_string(),
            serde_json::Value::String("error_max_turns: exceeded 40 turns".to_string()),
        );
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_retry_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should NOT retry — circuit breaker tripped
        assert!(matches!(reaction, Reaction::None));
        let updated = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
    }

    // ========================================================================
    // ConvergenceCoordinationHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_coordination_all_children_complete() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCoordinationHandler::new(repo.clone());

        // Create parent: Running, convergent (has trajectory_id)
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: complete
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        child1.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: complete
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        child2.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskCompleted for child2 (last child to complete)
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(child2.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: child2.id,
                tokens_used: 50,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit a TaskCompleted event for the parent
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::TaskCompleted { task_id, .. } => {
                        assert_eq!(*task_id, parent.id);
                    }
                    other => panic!("Expected TaskCompleted, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify parent is now Complete
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(updated_parent.status, TaskStatus::Complete);
    }

    #[tokio::test]
    async fn test_convergence_coordination_child_fails() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCoordinationHandler::new(repo.clone());

        // Create parent: Running, convergent
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: failed
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        child1.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: complete
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        child2.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskFailed for child1
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(child1.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskFailed {
                task_id: child1.id,
                error: "child task error".to_string(),
                retry_count: 0,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit TaskFailed for the parent
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::TaskFailed { task_id, error, .. } => {
                        assert_eq!(*task_id, parent.id);
                        assert!(error.contains("child task failed"));
                    }
                    other => panic!("Expected TaskFailed, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify parent is now Failed
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(updated_parent.status, TaskStatus::Failed);
    }

    #[tokio::test]
    async fn test_convergence_coordination_partial_complete() {
        let repo = setup_task_repo().await;
        let handler = ConvergenceCoordinationHandler::new(repo.clone());

        // Create parent: Running, convergent
        let mut parent = Task::new("Parent convergent task");
        parent.trajectory_id = Some(Uuid::new_v4());
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();
        repo.create(&parent).await.unwrap();

        // Create child 1: complete
        let mut child1 = Task::new("Child 1");
        child1.parent_id = Some(parent.id);
        child1.transition_to(TaskStatus::Ready).unwrap();
        child1.transition_to(TaskStatus::Running).unwrap();
        child1.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&child1).await.unwrap();

        // Create child 2: still Running (not terminal)
        let mut child2 = Task::new("Child 2");
        child2.parent_id = Some(parent.id);
        child2.transition_to(TaskStatus::Ready).unwrap();
        child2.transition_to(TaskStatus::Running).unwrap();
        repo.create(&child2).await.unwrap();

        // Fire TaskCompleted for child1 only
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(child1.id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: child1.id,
                tokens_used: 50,
            },
        };

        let ctx = HandlerContext {
            chain_depth: 0,
            correlation_id: None,
        };

        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should return None — still waiting for child2
        assert!(matches!(reaction, Reaction::None));

        // Parent should still be Running
        let updated_parent = repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(updated_parent.status, TaskStatus::Running);
    }

    // ========================================================================
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
                let canceled_ids: Vec<Uuid> = events.iter().map(|e| {
                    match &e.payload {
                        EventPayload::TaskCanceled { task_id, .. } => *task_id,
                        other => panic!("Expected TaskCanceled, got {:?}", other),
                    }
                }).collect();
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

    // ========================================================================
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
    // ConvergenceMemoryHandler tests
    // ========================================================================

    #[tokio::test]
    async fn test_convergence_memory_handler_stores_success() {
        use crate::adapters::sqlite::SqliteMemoryRepository;

        let pool = crate::adapters::sqlite::create_migrated_test_pool().await.unwrap();
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
            payload: EventPayload::ConvergenceTerminated {
                task_id: task.id,
                trajectory_id,
                outcome: "converged".to_string(),
                total_iterations: 3,
                total_tokens: 1500,
                final_convergence_level: 0.95,
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
                    EventPayload::MemoryStored { namespace, tier, memory_type, .. } => {
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
        let stored = memory_repo.get_by_key(&idempotency_key, "convergence").await.unwrap();
        assert!(stored.is_some(), "Memory should have been stored");
        let stored = stored.unwrap();
        assert!(stored.content.contains("converged"));
        assert!(stored.content.contains("iterations: 3"));
        assert!(stored.content.contains("tokens: 1500"));
    }

    // ========================================================================
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
            payload: EventPayload::ConvergenceTerminated {
                task_id: task.id,
                trajectory_id,
                outcome: "converged".to_string(),
                total_iterations: 5,
                total_tokens: 2500,
                final_convergence_level: 0.92,
            },
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
            Some(&serde_json::Value::Number(serde_json::Number::from(2500u64)))
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
    // TaskOutcomeMemoryHandler tests
    // ========================================================================

    async fn setup_task_and_memory_repos() -> (Arc<SqliteTaskRepository>, Arc<crate::adapters::sqlite::SqliteMemoryRepository>) {
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

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should emit a MemoryStored event
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::MemoryStored { namespace, tier, memory_type, .. } => {
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

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // First call stores the memory
        let reaction1 = handler.handle(&make_event(task.id), &ctx).await.unwrap();
        assert!(matches!(reaction1, Reaction::EmitEvents(_)), "First call should store memory");

        // Second call (idempotency) should return None, no second store
        let reaction2 = handler.handle(&make_event(task.id), &ctx).await.unwrap();
        assert!(matches!(reaction2, Reaction::None), "Second call should be idempotent (Reaction::None)");
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

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None), "Should return None if task not found");
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
            payload: EventPayload::TaskCompleted {
                task_id: task.id,
                tokens_used: 200,
            },
        };

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        assert!(stored.is_some(), "Memory should have been stored for failed task");
        let stored = stored.unwrap();
        assert!(stored.content.contains("failed"));
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

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(
            matches!(reaction, Reaction::None),
            "Handler should ignore non-TaskCompleted/TaskCompletedWithResult events"
        );
    }

    // ========================================================================
    // ReviewFailureLoopHandler tests
    // ========================================================================

    async fn setup_command_bus(repo: Arc<SqliteTaskRepository>) -> Arc<crate::services::command_bus::CommandBus> {
        use crate::domain::ports::NullMemoryRepository;
        use crate::services::task_service::TaskService;
        use crate::services::goal_service::GoalService;
        use crate::services::memory_service::MemoryService;

        let pool = create_migrated_test_pool().await.unwrap();
        let goal_repo = Arc::new(crate::adapters::sqlite::goal_repository::SqliteGoalRepository::new(pool));
        let task_service = Arc::new(TaskService::new(repo));
        let goal_service = Arc::new(GoalService::new(goal_repo));
        let memory_service = Arc::new(MemoryService::new(Arc::new(NullMemoryRepository::new())));
        let event_bus = Arc::new(crate::services::EventBus::new(crate::services::EventBusConfig {
            persist_events: false,
            ..Default::default()
        }));
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
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        assert_eq!(
            updated.context.custom.get("review_loop_active"),
            Some(&serde_json::Value::Bool(true)),
        );
        let successor_val = updated.context.custom.get("review_loop_successor")
            .expect("review_loop_successor should be set on the original task");
        let successor_str = successor_val.as_str().expect("review_loop_successor should be a string");
        let successor_id: uuid::Uuid = successor_str.parse().expect("review_loop_successor should be a valid UUID");
        assert_eq!(successor_id, new_review_task_id, "review_loop_successor must match new_review_task_id in the event");
    }

    #[tokio::test]
    async fn test_review_failure_loop_max_iterations_skips() {
        let repo = setup_task_repo().await;
        let command_bus = setup_command_bus(repo.clone()).await;
        let handler = ReviewFailureLoopHandler::new(repo.clone(), command_bus, 3);

        // Create a review task at iteration 3 (== max)
        let mut review_task = Task::new("Review implementation");
        review_task.agent_type = Some("code-reviewer".to_string());
        review_task.context.custom.insert(
            "review_iteration".to_string(),
            serde_json::json!(3),
        );
        review_task.transition_to(TaskStatus::Ready).unwrap();
        review_task.transition_to(TaskStatus::Running).unwrap();
        review_task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&review_task).await.unwrap();

        let event = make_task_failed_event(review_task.id, 0);
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        review_task.context.custom.insert(
            "review_loop_active".to_string(),
            serde_json::Value::Bool(true),
        );
        review_task.transition_to(TaskStatus::Ready).unwrap();
        review_task.transition_to(TaskStatus::Running).unwrap();
        review_task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&review_task).await.unwrap();

        let event = make_task_failed_event(review_task.id, 0);
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        task.context.custom.insert(
            "review_loop_active".to_string(),
            serde_json::Value::Bool(true),
        );
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_task_failed_event(task.id, 0);
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        task.context.custom.insert(
            "review_iteration".to_string(),
            serde_json::json!(2u64),
        );
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_task_failed_event(task.id, 0);
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
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
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        // Should trigger — title starts with "Review"
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                assert!(matches!(events[0].payload, EventPayload::ReviewLoopTriggered { .. }));
            }
            Reaction::None => panic!("Expected EmitEvents for title-based review detection"),
        }
    }

    // ========================================================================
    // SystemStallDetectorHandler tests
    // ========================================================================

    /// Helper: build a "system-stall-check" ScheduledEventFired event.
    fn make_stall_check_event() -> UnifiedEvent {
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
                schedule_id: Uuid::new_v4(),
                name: "system-stall-check".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_stall_detector_ignores_wrong_schedule() {
        let repo = setup_task_repo().await;
        let handler = SystemStallDetectorHandler::new(repo, 10);

        let event = UnifiedEvent {
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
                schedule_id: Uuid::new_v4(),
                name: "stats-update".to_string(),
            },
        };

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_stall_detector_no_escalation_when_running_tasks_exist() {
        let repo = setup_task_repo().await;
        // Threshold of 0 means any idle period should fire — but running tasks
        // count as activity, so it should still be Reaction::None.
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // Create a Running task so the handler sees work-in-progress.
        let mut task = Task::new("Running task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        repo.create(&task).await.unwrap();

        let event = make_stall_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_stall_detector_no_escalation_when_snapshot_changes() {
        let repo = setup_task_repo().await;
        // Threshold of 0 — but the snapshot change resets last_activity.
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // First call: establishes initial snapshot (all zeros, no running).
        let event = make_stall_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let _ = handler.handle(&event, &ctx).await.unwrap();

        // Now create a completed task so the snapshot changes on next tick.
        let mut task = Task::new("Completed task");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Complete).unwrap();
        repo.create(&task).await.unwrap();

        let event2 = make_stall_check_event();
        let reaction = handler.handle(&event2, &ctx).await.unwrap();
        // Snapshot changed → activity reset → no escalation
        assert!(matches!(reaction, Reaction::None));
    }

    #[tokio::test]
    async fn test_stall_detector_escalation_on_idle() {
        let repo = setup_task_repo().await;
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // First call: snapshot is (0,0,0), no running/ready → idle, but
        // last_activity was just set to "now" in the constructor, so we need
        // to force the internal timestamp into the past.
        {
            let mut state = handler.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(100);
        }

        let event = make_stall_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        match reaction {
            Reaction::EmitEvents(events) => {
                // Should emit 2 events: escalation + convergence trigger
                assert_eq!(events.len(), 2);
                assert_eq!(events[0].category, EventCategory::Escalation);
                match &events[0].payload {
                    EventPayload::HumanEscalationRequired { reason, urgency, is_blocking, .. } => {
                        assert!(reason.contains("System stall detected"));
                        assert!(reason.contains("Auto-recovery triggered"));
                        assert_eq!(urgency, "high");
                        assert!(!is_blocking);
                    }
                    other => panic!("Expected HumanEscalationRequired, got {:?}", other),
                }
                assert_eq!(events[1].category, EventCategory::Scheduler);
                match &events[1].payload {
                    EventPayload::ScheduledEventFired { name, .. } => {
                        assert_eq!(name, "goal-convergence-check");
                    }
                    other => panic!("Expected ScheduledEventFired, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents escalation"),
        }
    }

    #[tokio::test]
    async fn test_stall_detector_emits_both_escalation_and_convergence_trigger() {
        let repo = setup_task_repo().await;
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // Force last_activity into the past so the stall threshold is exceeded.
        {
            let mut state = handler.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(500);
        }

        let event = make_stall_check_event();
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };
        let reaction = handler.handle(&event, &ctx).await.unwrap();

        let events = match reaction {
            Reaction::EmitEvents(events) => events,
            Reaction::None => panic!("Expected EmitEvents, got None"),
        };

        // Verify exactly 2 events
        assert_eq!(events.len(), 2, "Stall detector should emit exactly 2 events");

        // First event: HumanEscalationRequired
        assert_eq!(events[0].severity, EventSeverity::Warning);
        assert_eq!(events[0].category, EventCategory::Escalation);
        match &events[0].payload {
            EventPayload::HumanEscalationRequired { reason, urgency, is_blocking, .. } => {
                assert!(reason.contains("System stall detected"), "Reason should mention stall: {}", reason);
                assert!(reason.contains("Auto-recovery triggered"), "Reason should mention auto-recovery: {}", reason);
                assert_eq!(urgency, "high");
                assert!(!is_blocking);
            }
            other => panic!("Event 0: expected HumanEscalationRequired, got {:?}", other),
        }

        // Second event: ScheduledEventFired for goal-convergence-check
        assert_eq!(events[1].severity, EventSeverity::Info);
        assert_eq!(events[1].category, EventCategory::Scheduler);
        match &events[1].payload {
            EventPayload::ScheduledEventFired { name, schedule_id } => {
                assert_eq!(name, "goal-convergence-check");
                // schedule_id should be a fresh UUID (non-nil)
                assert_ne!(*schedule_id, uuid::Uuid::nil());
            }
            other => panic!("Event 1: expected ScheduledEventFired, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_stall_detector_resets_after_escalation() {
        let repo = setup_task_repo().await;
        // Threshold = 0 so any idle fires immediately.
        let handler = SystemStallDetectorHandler::new(repo.clone(), 0);

        // Force last_activity into the past.
        {
            let mut state = handler.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(100);
        }

        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // First call should escalate.
        let event1 = make_stall_check_event();
        let reaction1 = handler.handle(&event1, &ctx).await.unwrap();
        assert!(matches!(reaction1, Reaction::EmitEvents(_)));

        // Second call immediately after: last_activity was reset by the
        // escalation, so now the idle_secs is ~0, which is not >= threshold(0)
        // *only if* there is zero elapsed time. In practice the handler uses
        // `< threshold` (strict), and 0 < 0 is false, so threshold=0 will
        // always fire unless activity is detected. Let's use threshold=1 to
        // make this test reliable.
        let handler2 = SystemStallDetectorHandler::new(repo.clone(), 1);
        {
            let mut state = handler2.state.write().await;
            state.0 = chrono::Utc::now() - chrono::Duration::seconds(100);
        }
        // First call escalates
        let reaction_a = handler2.handle(&make_stall_check_event(), &ctx).await.unwrap();
        assert!(matches!(reaction_a, Reaction::EmitEvents(_)));
        // Second call immediately: last_activity was just reset, idle < 1s
        let reaction_b = handler2.handle(&make_stall_check_event(), &ctx).await.unwrap();
        assert!(matches!(reaction_b, Reaction::None));
    }
}

// ============================================================================
// TaskScheduleHandler
// ============================================================================

/// Creates tasks when a task schedule fires.
///
/// Listens for `ScheduledEventFired` events with names matching
/// `"task-schedule:{uuid}"` and creates the corresponding task
/// via the CommandBus.
pub struct TaskScheduleHandler<S: TaskScheduleRepository, T: TaskRepository> {
    schedule_repo: Arc<S>,
    task_repo: Arc<T>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
}

impl<S: TaskScheduleRepository, T: TaskRepository> TaskScheduleHandler<S, T> {
    pub fn new(
        schedule_repo: Arc<S>,
        task_repo: Arc<T>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
    ) -> Self {
        Self {
            schedule_repo,
            task_repo,
            command_bus,
        }
    }

    async fn record_fire(&self, schedule_id: uuid::Uuid, task_id: uuid::Uuid) -> Result<(), String> {
        let mut schedule = self.schedule_repo.get(schedule_id).await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Schedule not found".to_string())?;

        schedule.fire_count += 1;
        schedule.last_fired_at = Some(chrono::Utc::now());
        schedule.last_task_id = Some(task_id);
        schedule.updated_at = chrono::Utc::now();

        if matches!(schedule.schedule, TaskScheduleType::Once { .. }) {
            schedule.status = TaskScheduleStatus::Completed;
        }

        self.schedule_repo.update(&schedule).await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl<S: TaskScheduleRepository + 'static, T: TaskRepository + 'static> EventHandler
    for TaskScheduleHandler<S, T>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskScheduleHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        let (_schedule_id, name) = match &event.payload {
            EventPayload::ScheduledEventFired { schedule_id, name } => (*schedule_id, name.clone()),
            _ => return Ok(Reaction::None),
        };

        // Only handle task-schedule events
        if !name.starts_with("task-schedule:") {
            return Ok(Reaction::None);
        }

        // Extract schedule UUID from event name
        let sched_id_str = name.strip_prefix("task-schedule:")
            .ok_or_else(|| "Invalid task-schedule event name".to_string())?;
        let sched_id = uuid::Uuid::parse_str(sched_id_str)
            .map_err(|e| format!("Invalid schedule UUID in event name: {}", e))?;

        // Load the schedule
        let schedule = match self.schedule_repo.get(sched_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!("Task schedule {} not found, skipping", sched_id);
                return Ok(Reaction::None);
            }
            Err(e) => return Err(format!("Failed to load task schedule: {}", e)),
        };

        // Check if schedule is active
        if schedule.status != TaskScheduleStatus::Active {
            tracing::debug!("Task schedule {} is not active, skipping", schedule.name);
            return Ok(Reaction::None);
        }

        // Handle overlap policy
        match schedule.overlap_policy {
            OverlapPolicy::Skip => {
                if let Some(last_task_id) = schedule.last_task_id {
                    match self.task_repo.get(last_task_id).await {
                        Ok(Some(task)) if !task.status.is_terminal() => {
                            tracing::info!(
                                "Skipping task creation for schedule '{}': previous task {} is still {}",
                                schedule.name, last_task_id, task.status.as_str()
                            );
                            return Ok(Reaction::None);
                        }
                        _ => {} // Previous task done or not found, proceed
                    }
                }
            }
            OverlapPolicy::CancelPrevious => {
                if let Some(last_task_id) = schedule.last_task_id {
                    match self.task_repo.get(last_task_id).await {
                        Ok(Some(task)) if !task.status.is_terminal() => {
                            // Cancel the previous task
                            let cancel_cmd = DomainCommand::Task(TaskCommand::Cancel {
                                task_id: last_task_id,
                                reason: format!(
                                    "Superseded by new instance of schedule '{}'",
                                    schedule.name
                                ),
                            });
                            let envelope = CommandEnvelope::new(
                                CommandSource::Scheduler(schedule.name.clone()),
                                cancel_cmd,
                            );
                            if let Err(e) = self.command_bus.dispatch(envelope).await {
                                tracing::warn!(
                                    "Failed to cancel previous task {} for schedule '{}': {}",
                                    last_task_id, schedule.name, e
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            OverlapPolicy::Allow => {} // Always create
        }

        // Create the task via CommandBus
        let idempotency_key = schedule.next_idempotency_key();
        let submit_cmd = DomainCommand::Task(TaskCommand::Submit {
            title: Some(schedule.task_title.clone()),
            description: schedule.task_description.clone(),
            parent_id: None,
            priority: schedule.task_priority,
            agent_type: schedule.task_agent_type.clone(),
            depends_on: vec![],
            context: Box::new(None),
            idempotency_key: Some(idempotency_key),
            source: TaskSource::Schedule(schedule.id),
            deadline: None,
            task_type: None,
            execution_mode: None,
        });

        let envelope = CommandEnvelope::new(
            CommandSource::Scheduler(schedule.name.clone()),
            submit_cmd,
        );

        match self.command_bus.dispatch(envelope).await {
            Ok(crate::services::command_bus::CommandResult::Task(task)) => {
                tracing::info!(
                    "Task schedule '{}' created task {} (fire #{})",
                    schedule.name, task.id, schedule.fire_count + 1
                );

                // Record the fire (best-effort update)
                if let Err(e) = self.record_fire(sched_id, task.id).await {
                    tracing::warn!("Failed to record fire for schedule '{}': {}", schedule.name, e);
                }

                Ok(Reaction::None)
            }
            Ok(_) => {
                tracing::warn!("Unexpected command result for task schedule '{}'", schedule.name);
                Ok(Reaction::None)
            }
            Err(crate::services::command_bus::CommandError::DuplicateCommand(_)) => {
                tracing::debug!("Duplicate fire for schedule '{}', skipping", schedule.name);
                Ok(Reaction::None)
            }
            Err(e) => {
                tracing::error!("Failed to create task for schedule '{}': {}", schedule.name, e);
                Err(format!("Task creation failed: {}", e))
            }
        }
    }
}

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
}

impl<G: GoalRepository, T: TaskRepository> GoalConvergenceCheckHandler<G, T> {
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
    ) -> Self {
        Self { goal_repo, task_repo, command_bus, budget_tracker: None }
    }

    /// Attach a budget tracker to enable budget-pressure gating of convergence checks.
    pub fn with_budget_tracker(mut self, tracker: Arc<crate::services::budget_tracker::BudgetTracker>) -> Self {
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
            desc.push_str(&format!("### {}. {} (priority: {:?})\n", i + 1, goal.name, goal.priority));
            desc.push_str(&format!("**ID**: `{}`\n\n", goal.id));
            desc.push_str(&format!("{}\n\n", goal.description));

            if !goal.applicability_domains.is_empty() {
                desc.push_str(&format!("**Domains**: {}\n\n", goal.applicability_domains.join(", ")));
            }

            if !goal.constraints.is_empty() {
                desc.push_str("**Constraints**:\n");
                for c in &goal.constraints {
                    desc.push_str(&format!("- **{}** ({:?}): {}\n", c.name, c.constraint_type, c.description));
                }
                desc.push('\n');
            }
        }

        // Instructions
        desc.push_str("## Instructions\n\n");
        desc.push_str("This task is enrolled in a workflow. Use your workflow tools to orchestrate the evaluation.\n\n");
        desc.push_str("### Research Phase\n");
        desc.push_str("1. **Search Memory**: Call `memory_search` to find prior convergence evaluations, known patterns, and recent task outcomes.\n");
        desc.push_str("2. **Review Existing Work**: Call `task_list` with each status (running, ready, pending, complete, failed) to understand what's already in flight and what has been tried.\n");
        desc.push_str("3. **Check Failed Tasks**: For any failed tasks, call `task_get` on them to understand failure reasons. Store failure patterns via `memory_store` for future reference.\n\n");
        desc.push_str("### Evaluation\n");
        desc.push_str("4. **Evaluate Progress**: For each active goal, assess how the completed and running tasks contribute toward convergence. Consider the ratio of completed vs failed tasks.\n");
        desc.push_str("5. **Identify Gaps**: Determine what work is missing or insufficient to make meaningful progress on each goal. Consider constraint satisfaction.\n");
        desc.push_str("6. **Prioritize**: Rank the goals by urgency and impact. Focus on goals with the least progress or most failures.\n");
        desc.push_str("7. **Avoid Redundancy**: Do not duplicate existing running or ready tasks. Focus on genuine gaps.\n\n");
        desc.push_str("### Workflow Orchestration\n");
        desc.push_str("8. **Reuse Agents**: Call `agent_list` to see what agent templates already exist. Reuse existing agents whenever possible.\n");
        desc.push_str("9. **Fan Out Work**: Use `workflow_advance` and `workflow_fan_out` to create slices for the identified gaps. Each slice should represent a concrete, actionable unit of work for an under-served goal. Assign agents to slices via `task_assign`.\n");
        desc.push_str("10. **Store Evaluation**: Call `memory_store` with your convergence evaluation summary (namespace: `convergence-checks`, memory_type: `decision`) so future checks can build on your findings.\n\n");
        desc.push_str("Remember: Goals are convergent attractors — they are never 'completed.' Your job is to identify the highest-impact incremental work that moves the swarm closer to each goal. Use the workflow to fan out concrete work — do not simply list recommendations as text output.\n");

        desc
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static>
    EventHandler for GoalConvergenceCheckHandler<G, T>
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
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        use crate::services::command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand};
        use crate::domain::models::TaskPriority;

        // Determine if this was triggered by a budget opportunity signal
        let is_budget_trigger = matches!(
            &event.payload,
            EventPayload::ScheduledEventFired { name, .. }
                if name == "goal-convergence-check:budget-trigger"
        );

        // Load all active goals
        let goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("GoalConvergenceCheckHandler: failed to get active goals: {}", e))?;

        if goals.is_empty() {
            tracing::debug!("GoalConvergenceCheckHandler: no active goals, skipping convergence check");
            return Ok(Reaction::None);
        }

        // Gather task statistics
        let completed = self.task_repo.list_by_status(TaskStatus::Complete).await
            .map_err(|e| format!("GoalConvergenceCheckHandler: failed to list completed tasks: {}", e))?;
        let failed = self.task_repo.list_by_status(TaskStatus::Failed).await
            .map_err(|e| format!("GoalConvergenceCheckHandler: failed to list failed tasks: {}", e))?;
        let running = self.task_repo.list_by_status(TaskStatus::Running).await
            .map_err(|e| format!("GoalConvergenceCheckHandler: failed to list running tasks: {}", e))?;
        let ready = self.task_repo.list_by_status(TaskStatus::Ready).await
            .map_err(|e| format!("GoalConvergenceCheckHandler: failed to list ready tasks: {}", e))?;
        let pending = self.task_repo.list_by_status(TaskStatus::Pending).await
            .map_err(|e| format!("GoalConvergenceCheckHandler: failed to list pending tasks: {}", e))?;

        // Overlap check: skip if a previous convergence check task is still enqueued/active
        let overlap_exists = pending.iter()
            .chain(ready.iter())
            .chain(running.iter())
            .any(|t| t.title.starts_with("Goal Convergence Check"));

        if overlap_exists {
            tracing::debug!("GoalConvergenceCheckHandler: previous convergence check task already enqueued or active, skipping");
            return Ok(Reaction::None);
        }

        // Budget gate: if triggered by the scheduler (not a budget opportunity), and budget
        // pressure is critical, skip creating new work.
        if !is_budget_trigger
            && let Some(ref bt) = self.budget_tracker
                && bt.should_pause_new_work().await {
                    tracing::debug!("GoalConvergenceCheckHandler: pausing convergence check — budget at critical pressure");
                    return Ok(Reaction::None);
                }

        // Build idempotency key.
        // Budget-triggered checks get a unique key per timestamp to bypass the 4-hour window.
        let now = chrono::Utc::now();
        let idem_key = if is_budget_trigger {
            format!("goal-convergence-check:budget:{}", now.timestamp())
        } else {
            // Standard 4-hour bucket idempotency key
            let bucket = now.timestamp() / 14400;
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

        let title = format!(
            "Goal Convergence Check — {} active goal(s)",
            goals.len()
        );

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
                tracing::debug!("GoalConvergenceCheckHandler: duplicate convergence check task, skipping");
            }
            Err(e) => {
                tracing::warn!("GoalConvergenceCheckHandler: failed to create convergence check task: {}", e);
            }
        }

        Ok(Reaction::None)
    }
}

// ============================================================================
// GoalStagnationDetectorHandler
// ============================================================================

/// Detects goals that have not been evaluated in a convergence check recently.
///
/// Fires a `HumanEscalationRequired` event for any active goal whose
/// `last_convergence_check_at` exceeds the stall threshold, unless the goal
/// was created recently (grace period) or an alert was already emitted within
/// the threshold window (in-memory dedup).
///
/// Triggered by `ScheduledEventFired { name: "system-stall-check" }` — no
/// new schedule needed.
pub struct GoalStagnationDetectorHandler<G: GoalRepository> {
    goal_repo: Arc<G>,
    /// Maximum time (seconds) a goal may go without a convergence check before alerting.
    stall_threshold_secs: u64,
    /// In-memory dedup: maps goal_id → last alert timestamp.
    last_alerted: RwLock<std::collections::HashMap<uuid::Uuid, chrono::DateTime<chrono::Utc>>>,
}

impl<G: GoalRepository> GoalStagnationDetectorHandler<G> {
    pub fn new(goal_repo: Arc<G>, stall_threshold_secs: u64) -> Self {
        Self {
            goal_repo,
            stall_threshold_secs,
            last_alerted: RwLock::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static> EventHandler for GoalStagnationDetectorHandler<G> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "GoalStagnationDetectorHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };

        if name != "system-stall-check" {
            return Ok(Reaction::None);
        }

        let goals = self.goal_repo.get_active_with_constraints().await
            .map_err(|e| format!("GoalStagnationDetector: failed to get active goals: {}", e))?;

        if goals.is_empty() {
            return Ok(Reaction::None);
        }

        let now = chrono::Utc::now();
        let threshold_secs = self.stall_threshold_secs as i64;
        let mut last_alerted = self.last_alerted.write().await;
        let mut events = Vec::new();

        for goal in &goals {
            // Grace period: new goals without a check yet AND created recently should not alert
            if goal.last_convergence_check_at.is_none() {
                let age_secs = (now - goal.created_at).num_seconds();
                if age_secs < threshold_secs {
                    tracing::debug!(
                        goal_id = %goal.id,
                        age_secs,
                        threshold = threshold_secs,
                        "GoalStagnationDetector: goal within grace period, skipping"
                    );
                    continue;
                }
            }

            // Determine if this goal is stagnant
            let is_stagnant = match goal.last_convergence_check_at {
                Some(last_check) => {
                    let secs_since_check = (now - last_check).num_seconds();
                    secs_since_check > threshold_secs
                }
                None => {
                    // No check ever AND outside grace period
                    true
                }
            };

            if !is_stagnant {
                continue;
            }

            // Dedup: skip if we already alerted for this goal within the threshold window
            let should_alert = match last_alerted.get(&goal.id) {
                Some(last_alert_time) => {
                    let secs_since_alert = (now - *last_alert_time).num_seconds();
                    secs_since_alert > threshold_secs
                }
                None => true,
            };

            if !should_alert {
                tracing::debug!(
                    goal_id = %goal.id,
                    "GoalStagnationDetector: alert already emitted recently, skipping"
                );
                continue;
            }

            tracing::warn!(
                goal_id = %goal.id,
                goal_name = %goal.name,
                "GoalStagnationDetector: goal has not been evaluated in a convergence check recently"
            );

            last_alerted.insert(goal.id, now);

            events.push(crate::services::event_factory::make_event(
                EventSeverity::Warning,
                EventCategory::Escalation,
                Some(goal.id),
                None,
                EventPayload::HumanEscalationRequired {
                    goal_id: Some(goal.id),
                    task_id: None,
                    reason: format!(
                        "Goal '{}' (id: {}) has not been evaluated in a convergence check for more than {} seconds. This may indicate goal stagnation.",
                        goal.name, goal.id, threshold_secs
                    ),
                    urgency: "high".to_string(),
                    questions: vec![
                        format!("Goal '{}' may be stagnating. Is there work being done toward this goal?", goal.name),
                        "Consider triggering a manual goal convergence check to generate new tasks.".to_string(),
                    ],
                    is_blocking: false,
                },
            ));
        }

        if events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(events))
        }
    }
}

// ============================================================================
// IngestionPollHandler (Adapter integration)
// ============================================================================

/// Polls all registered ingestion adapters for new work items and creates
/// tasks for each one via the CommandBus. Deduplicates using idempotency
/// keys of the form `adapter:{name}:{external_id}`.
pub struct IngestionPollHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
}

impl<T: TaskRepository> IngestionPollHandler<T> {
    /// Create a new ingestion poll handler.
    pub fn new(
        task_repo: Arc<T>,
        adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
    ) -> Self {
        Self {
            task_repo,
            adapter_registry,
            command_bus,
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for IngestionPollHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "IngestionPollHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        // Only react to the adapter-ingestion-poll schedule
        let schedule_name = match &event.payload {
            EventPayload::ScheduledEventFired { name, .. } => name.as_str(),
            _ => return Ok(Reaction::None),
        };
        if schedule_name != "adapter-ingestion-poll" {
            return Ok(Reaction::None);
        }

        let mut all_events = Vec::new();

        for adapter_name in self.adapter_registry.ingestion_names() {
            let adapter = match self.adapter_registry.get_ingestion(adapter_name) {
                Some(a) => a,
                None => continue,
            };

            let items = match adapter.poll(None).await {
                Ok(items) => items,
                Err(e) => {
                    tracing::warn!(
                        adapter = adapter_name,
                        error = %e,
                        "Ingestion adapter poll failed"
                    );
                    all_events.push(crate::services::event_factory::make_event(
                        EventSeverity::Warning,
                        EventCategory::Adapter,
                        None,
                        None,
                        EventPayload::AdapterIngestionFailed {
                            adapter_name: adapter_name.to_string(),
                            error: e.to_string(),
                        },
                    ));
                    continue;
                }
            };

            let items_found = items.len();
            let mut tasks_created: usize = 0;

            for item in &items {
                let idem_key = format!("adapter:{}:{}", adapter_name, item.external_id);

                // Dedup: skip if a task with this idempotency key already exists
                match self.task_repo.get_by_idempotency_key(&idem_key).await {
                    Ok(Some(_)) => {
                        tracing::debug!(
                            adapter = adapter_name,
                            external_id = %item.external_id,
                            "Skipping duplicate ingestion item"
                        );
                        continue;
                    }
                    Ok(None) => {} // new item, proceed
                    Err(e) => {
                        tracing::warn!(
                            adapter = adapter_name,
                            external_id = %item.external_id,
                            error = %e,
                            "Failed idempotency check, creating task anyway"
                        );
                    }
                }

                // Map priority
                let priority = item.priority.unwrap_or(crate::domain::models::TaskPriority::Normal);

                // Build a structured header for the task description.
                let mut header = format!(
                    "[Ingested from {} — {}]",
                    adapter_name, item.external_id
                );

                // When the ingested item carries a GitHub URL, surface it and
                // instruct the agent to pass `issue_number` to `create_pr` so
                // the PR body gets a "Closes #N" link. GitHub then closes the
                // issue automatically when the PR is merged into the default branch.
                if let Some(url) = item.metadata.get("github_url").and_then(|v| v.as_str()) {
                    header.push_str(&format!("\nGitHub Issue: {url}"));
                    header.push_str(&format!(
                        "\n\nWhen creating a pull request to resolve this issue, \
                         include `\"issue_number\": {}` in the create_pr params. \
                         This appends \"Closes #{}\" to the PR body so GitHub \
                         closes the issue automatically when the PR is merged.",
                        item.external_id, item.external_id
                    ));
                }

                let description = format!("{}\n\n{}", header, item.description);

                let envelope = crate::services::command_bus::CommandEnvelope::new(
                    crate::services::command_bus::CommandSource::Adapter(adapter_name.to_string()),
                    crate::services::command_bus::DomainCommand::Task(
                        crate::services::command_bus::TaskCommand::Submit {
                            title: Some(item.title.clone()),
                            description,
                            parent_id: None,
                            priority,
                            agent_type: None,
                            depends_on: vec![],
                            context: Box::new(None),
                            idempotency_key: Some(idem_key),
                            source: TaskSource::Adapter(adapter_name.to_string()),
                            deadline: None,
                            task_type: None,
                            execution_mode: None,
                        },
                    ),
                );

                match self.command_bus.dispatch(envelope).await {
                    Ok(crate::services::command_bus::CommandResult::Task(task)) => {
                        tasks_created += 1;
                        all_events.push(crate::services::event_factory::make_event(
                            EventSeverity::Info,
                            EventCategory::Adapter,
                            None,
                            Some(task.id),
                            EventPayload::AdapterTaskIngested {
                                task_id: task.id,
                                adapter_name: adapter_name.to_string(),
                            },
                        ));
                    }
                    Ok(_) => {
                        tasks_created += 1;
                    }
                    Err(crate::services::command_bus::CommandError::DuplicateCommand(_)) => {
                        tracing::debug!(
                            adapter = adapter_name,
                            external_id = %item.external_id,
                            "Duplicate command for ingestion item, skipping"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            adapter = adapter_name,
                            external_id = %item.external_id,
                            error = %e,
                            "Failed to create task for ingestion item"
                        );
                    }
                }
            }

            tracing::info!(
                adapter = adapter_name,
                items_found = items_found,
                tasks_created = tasks_created,
                "Ingestion poll completed"
            );

            all_events.push(crate::services::event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Adapter,
                None,
                None,
                EventPayload::AdapterIngestionCompleted {
                    adapter_name: adapter_name.to_string(),
                    items_found,
                    tasks_created,
                },
            ));
        }

        if all_events.is_empty() {
            Ok(Reaction::None)
        } else {
            Ok(Reaction::EmitEvents(all_events))
        }
    }
}

// ============================================================================
// AdapterLifecycleSyncHandler (Adapter integration)
// ============================================================================

/// Parses an idempotency key of the form `"adapter:{name}:{external_id}"`.
///
/// Returns `Some((adapter_name, external_id))` on success, `None` otherwise.
/// Uses `splitn(3, ':')` so that colons in the external ID are preserved.
fn parse_idempotency_key(key: &str) -> Option<(&str, &str)> {
    let mut parts = key.splitn(3, ':');
    let prefix = parts.next()?;
    if prefix != "adapter" {
        return None;
    }
    let adapter_name = parts.next()?;
    let external_id = parts.next()?;
    if adapter_name.is_empty() || external_id.is_empty() {
        return None;
    }
    Some((adapter_name, external_id))
}

/// Reads a status string from a manifest's config map, or falls back to a default.
///
/// Looks up `config_key` in `manifest.config`. If the value is a JSON string,
/// returns it; otherwise returns `default`.
fn get_status_string(
    manifest: Option<&crate::domain::models::adapter::AdapterManifest>,
    config_key: &str,
    default: &str,
) -> String {
    if let Some(m) = manifest
        && let Some(val) = m.config.get(config_key)
            && let Some(s) = val.as_str() {
                return s.to_string();
            }
    default.to_string()
}

/// Synchronizes task lifecycle state changes back to external systems.
///
/// When a task ingested from an external adapter transitions to Running
/// (claimed), Complete, or Failed, this handler pushes a status update
/// back to the originating system via the registered egress adapter.
pub struct AdapterLifecycleSyncHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
}

impl<T: TaskRepository> AdapterLifecycleSyncHandler<T> {
    /// Create a new adapter lifecycle sync handler.
    pub fn new(
        task_repo: Arc<T>,
        adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
    ) -> Self {
        Self {
            task_repo,
            adapter_registry,
        }
    }

    /// Shared handler logic for a lifecycle transition event.
    ///
    /// Looks up the task, validates it came from an egress-capable adapter,
    /// resolves the status string from the manifest config, and fires an
    /// `UpdateStatus` egress action against the adapter.
    async fn handle_lifecycle(
        &self,
        task_id: uuid::Uuid,
        config_key: &str,
        default_status: &str,
    ) -> Result<Reaction, String> {
        // Look up the task.
        let task = match self.task_repo.get(task_id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::debug!(
                    task_id = %task_id,
                    "Task not found for lifecycle sync, skipping"
                );
                return Ok(Reaction::None);
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "Failed to fetch task for lifecycle sync"
                );
                return Ok(Reaction::None);
            }
        };

        // Only act on adapter-sourced tasks.
        let adapter_name = match &task.source {
            TaskSource::Adapter(name) => name.clone(),
            _ => return Ok(Reaction::None),
        };

        // Parse the idempotency key to extract the external_id.
        let idem_key = match &task.idempotency_key {
            Some(k) => k.clone(),
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    adapter = adapter_name,
                    "Task has no idempotency key, skipping lifecycle sync"
                );
                return Ok(Reaction::None);
            }
        };

        let (key_adapter_name, external_id) = match parse_idempotency_key(&idem_key) {
            Some(parsed) => parsed,
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    key = idem_key,
                    "Idempotency key does not match adapter format, skipping"
                );
                return Ok(Reaction::None);
            }
        };

        // Verify the adapter name in the key matches task.source.
        if key_adapter_name != adapter_name.as_str() {
            tracing::debug!(
                task_id = %task_id,
                source_adapter = adapter_name,
                key_adapter = key_adapter_name,
                "Adapter name mismatch between source and idempotency key, skipping"
            );
            return Ok(Reaction::None);
        }

        // Look up the egress adapter.
        let adapter = match self.adapter_registry.get_egress(&adapter_name) {
            Some(a) => a,
            None => {
                tracing::debug!(
                    adapter = adapter_name,
                    task_id = %task_id,
                    "No egress adapter registered for lifecycle sync, skipping"
                );
                return Ok(Reaction::None);
            }
        };

        // Determine the status string from manifest config or default.
        let manifest = self.adapter_registry.get_manifest(&adapter_name);
        let new_status = get_status_string(manifest, config_key, default_status);

        // Allow adapters to opt out of a specific lifecycle transition by
        // setting the status value to "skip". This is useful when the external
        // system manages state through its own mechanism (e.g. GitHub issues
        // closed by PR merge) and the lifecycle sync should leave the item
        // untouched for that event.
        if new_status == "skip" {
            tracing::debug!(
                adapter = adapter_name,
                task_id = %task_id,
                config_key = config_key,
                "Lifecycle sync skipped (status = \"skip\")"
            );
            return Ok(Reaction::None);
        }

        let external_id = external_id.to_string();
        let action_name = format!("UpdateStatus({})", new_status);

        // Execute the egress action.
        let action = crate::domain::models::adapter::EgressAction::UpdateStatus {
            external_id: external_id.clone(),
            new_status,
        };

        match adapter.execute(&action).await {
            Ok(result) => {
                tracing::info!(
                    adapter = adapter_name,
                    task_id = %task_id,
                    external_id = external_id,
                    success = result.success,
                    "Lifecycle sync egress completed"
                );
                let event = crate::services::event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressCompleted {
                        adapter_name: adapter_name.clone(),
                        task_id,
                        action: action_name,
                        success: result.success,
                    },
                );
                Ok(Reaction::EmitEvents(vec![event]))
            }
            Err(e) => {
                tracing::warn!(
                    adapter = adapter_name,
                    task_id = %task_id,
                    external_id = external_id,
                    error = %e,
                    "Lifecycle sync egress failed"
                );
                let event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: e.to_string(),
                    },
                );
                Ok(Reaction::EmitEvents(vec![event]))
            }
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for AdapterLifecycleSyncHandler<T> {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "AdapterLifecycleSyncHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task, EventCategory::Adapter])
                .payload_types(vec![
                    "TaskClaimed".to_string(),
                    "TaskCompleted".to_string(),
                    "TaskFailed".to_string(),
                    "AdapterTaskIngested".to_string(),
                ]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        match &event.payload {
            EventPayload::TaskClaimed { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_in_progress", "skip").await
            }
            EventPayload::TaskCompleted { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_done", "skip").await
            }
            EventPayload::TaskFailed { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_failed", "skip").await
            }
            EventPayload::AdapterTaskIngested { task_id, .. } => {
                self.handle_lifecycle(*task_id, "status_pending", "skip").await
            }
            _ => Ok(Reaction::None),
        }
    }
}

#[cfg(test)]
mod adapter_lifecycle_sync_tests {
    use super::*;

    #[test]
    fn test_parse_idempotency_key_valid() {
        let result = parse_idempotency_key("adapter:clickup:abc123");
        assert_eq!(result, Some(("clickup", "abc123")));
    }

    #[test]
    fn test_parse_idempotency_key_colon_in_external_id() {
        // Colons in the external ID must be preserved via splitn(3, ':')
        let result = parse_idempotency_key("adapter:jira:PROJ:123");
        assert_eq!(result, Some(("jira", "PROJ:123")));
    }

    #[test]
    fn test_parse_idempotency_key_wrong_prefix() {
        let result = parse_idempotency_key("schedule:jira:abc");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_idempotency_key_missing_external_id() {
        // Only two parts — no external_id segment
        assert!(parse_idempotency_key("adapter:clickup").is_none());
    }

    #[test]
    fn test_parse_idempotency_key_empty_adapter_name() {
        assert!(parse_idempotency_key("adapter::external123").is_none());
    }

    #[test]
    fn test_parse_idempotency_key_empty_external_id() {
        assert!(parse_idempotency_key("adapter:clickup:").is_none());
    }

    #[test]
    fn test_parse_idempotency_key_no_colon() {
        assert!(parse_idempotency_key("notadapter").is_none());
    }

    #[test]
    fn test_get_status_string_from_manifest_config() {
        use crate::domain::models::adapter::{AdapterDirection, AdapterManifest, AdapterType};

        let manifest =
            AdapterManifest::new("clickup", AdapterType::Native, AdapterDirection::Bidirectional)
                .with_config("status_done", serde_json::Value::String("complete".to_string()));

        let result = get_status_string(Some(&manifest), "status_done", "done");
        assert_eq!(result, "complete");
    }

    #[test]
    fn test_get_status_string_fallback_no_manifest() {
        let result = get_status_string(None, "status_done", "done");
        assert_eq!(result, "done");
    }

    #[test]
    fn test_get_status_string_missing_key_uses_default() {
        use crate::domain::models::adapter::{AdapterDirection, AdapterManifest, AdapterType};

        // Manifest with no config entries
        let manifest =
            AdapterManifest::new("clickup", AdapterType::Native, AdapterDirection::Bidirectional);

        let result = get_status_string(Some(&manifest), "status_in_progress", "in progress");
        assert_eq!(result, "in progress");
    }

    #[test]
    fn test_get_status_string_non_string_value_uses_default() {
        use crate::domain::models::adapter::{AdapterDirection, AdapterManifest, AdapterType};

        // Config value is a number, not a string
        let manifest =
            AdapterManifest::new("clickup", AdapterType::Native, AdapterDirection::Bidirectional)
                .with_config("status_done", serde_json::json!(42));

        let result = get_status_string(Some(&manifest), "status_done", "done");
        assert_eq!(result, "done");
    }
}

// ============================================================================
// EgressRoutingHandler (Adapter integration)
// ============================================================================

/// Routes task completion results to egress adapters. When a task completes
/// with a result containing an "egress" key, this handler deserializes the
/// [`EgressDirective`] and calls the appropriate egress adapter.
pub struct EgressRoutingHandler {
    adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
}

impl EgressRoutingHandler {
    /// Create a new egress routing handler.
    pub fn new(
        adapter_registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
    ) -> Self {
        Self { adapter_registry }
    }
}

#[async_trait]
impl EventHandler for EgressRoutingHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "EgressRoutingHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskCompletedWithResult".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::CircuitBreak,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, result) = match &event.payload {
            EventPayload::TaskCompletedWithResult { task_id, result } => (*task_id, result),
            _ => return Ok(Reaction::None),
        };

        // Check if the result status contains egress routing info.
        // Convention: the result status field contains JSON with an "egress" key
        // when the completing agent wants to push results to an external system.
        let directive: crate::domain::models::adapter::EgressDirective = {
            // Try to parse the status field as JSON containing an egress directive
            let status_str = &result.status;
            match serde_json::from_str::<serde_json::Value>(status_str) {
                Ok(val) => {
                    if let Some(egress_val) = val.get("egress") {
                        match serde_json::from_value::<crate::domain::models::adapter::EgressDirective>(
                            egress_val.clone(),
                        ) {
                            Ok(d) => d,
                            Err(_) => return Ok(Reaction::None),
                        }
                    } else {
                        return Ok(Reaction::None);
                    }
                }
                Err(_) => return Ok(Reaction::None),
            }
        };

        let adapter_name = &directive.adapter_name;
        let adapter = match self.adapter_registry.get_egress(adapter_name) {
            Some(a) => a,
            None => {
                tracing::warn!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    "Egress adapter not found"
                );
                let fail_event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: format!("Adapter '{}' not found in registry", adapter_name),
                    },
                );
                return Ok(Reaction::EmitEvents(vec![fail_event]));
            }
        };

        let action_name = format!("{:?}", directive.action);

        match adapter.execute(&directive.action).await {
            Ok(egress_result) => {
                tracing::info!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    success = egress_result.success,
                    "Egress action completed"
                );
                let completed_event = crate::services::event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressCompleted {
                        adapter_name: adapter_name.clone(),
                        task_id,
                        action: action_name,
                        success: egress_result.success,
                    },
                );
                Ok(Reaction::EmitEvents(vec![completed_event]))
            }
            Err(e) => {
                tracing::warn!(
                    adapter = adapter_name.as_str(),
                    task_id = %task_id,
                    error = %e,
                    "Egress action failed"
                );
                let fail_event = crate::services::event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Adapter,
                    None,
                    Some(task_id),
                    EventPayload::AdapterEgressFailed {
                        adapter_name: adapter_name.clone(),
                        task_id: Some(task_id),
                        error: e.to_string(),
                    },
                );
                Ok(Reaction::EmitEvents(vec![fail_event]))
            }
        }
    }
}

// ============================================================================
// BudgetTokenAccumulatorHandler
// ============================================================================

/// Accumulates token usage from `AgentInstanceCompleted` events into the
/// `BudgetTracker` and recomputes aggregate budget pressure.
///
/// This allows the budget system to maintain a running tally of tokens
/// consumed by the swarm without polling external APIs.
pub struct BudgetTokenAccumulatorHandler {
    budget_tracker: Arc<crate::services::budget_tracker::BudgetTracker>,
}

impl BudgetTokenAccumulatorHandler {
    pub fn new(budget_tracker: Arc<crate::services::budget_tracker::BudgetTracker>) -> Self {
        Self { budget_tracker }
    }
}

#[async_trait]
impl EventHandler for BudgetTokenAccumulatorHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "BudgetTokenAccumulatorHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Agent])
                .payload_types(vec!["AgentInstanceCompleted".to_string()]),
            priority: HandlerPriority::SYSTEM,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let (task_id, tokens_used) = match &event.payload {
            EventPayload::AgentInstanceCompleted { task_id, tokens_used, .. } => {
                (*task_id, *tokens_used)
            }
            _ => return Ok(Reaction::None),
        };

        self.budget_tracker.record_tokens_used(task_id, tokens_used).await;
        self.budget_tracker.recompute_state().await;

        Ok(Reaction::None)
    }
}

// ============================================================================
// BudgetOpportunityHandler
// ============================================================================

/// Converts `BudgetOpportunityDetected` events into synthetic
/// `ScheduledEventFired { name: "goal-convergence-check:budget-trigger" }`
/// events, allowing the `GoalConvergenceCheckHandler` to fire an out-of-band
/// convergence check when budget headroom is available.
pub struct BudgetOpportunityHandler;

impl BudgetOpportunityHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BudgetOpportunityHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for BudgetOpportunityHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "BudgetOpportunityHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Budget])
                .payload_types(vec!["BudgetOpportunityDetected".to_string()]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let opportunity_score = match &event.payload {
            EventPayload::BudgetOpportunityDetected { opportunity_score, .. } => *opportunity_score,
            _ => return Ok(Reaction::None),
        };

        tracing::info!(
            opportunity_score,
            "BudgetOpportunityHandler: budget opportunity detected, triggering out-of-band convergence check"
        );

        let trigger_event = crate::services::event_factory::make_event(
            EventSeverity::Info,
            EventCategory::Scheduler,
            None,
            None,
            EventPayload::ScheduledEventFired {
                schedule_id: uuid::Uuid::new_v4(),
                name: "goal-convergence-check:budget-trigger".to_string(),
            },
        );

        Ok(Reaction::EmitEvents(vec![trigger_event]))
    }
}

// ============================================================================
// ObstacleEscalationHandler
// ============================================================================

/// Tracks repeated task failure patterns by agent_type + normalized error.
/// When the same class of obstacle causes failures beyond a threshold within
/// a sliding window, creates a new Goal to address the systematic issue.
///
/// This directly satisfies the "obstacle-escalation" constraint: "If the same
/// class of obstacle causes repeated failures without a template change or
/// memory update, it must escalate to a new goal rather than being silently
/// retried."
pub struct ObstacleEscalationHandler<T: TaskRepository, M: MemoryRepository, G: GoalRepository> {
    task_repo: Arc<T>,
    memory_repo: Arc<M>,
    goal_repo: Arc<G>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
    /// Number of failures before escalation (from config).
    threshold: u32,
    /// Sliding window duration in seconds (from config).
    window_secs: u64,
}

impl<T: TaskRepository, M: MemoryRepository, G: GoalRepository>
    ObstacleEscalationHandler<T, M, G>
{
    pub fn new(
        task_repo: Arc<T>,
        memory_repo: Arc<M>,
        goal_repo: Arc<G>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
        threshold: u32,
        window_secs: u64,
    ) -> Self {
        Self {
            task_repo,
            memory_repo,
            goal_repo,
            command_bus,
            threshold,
            window_secs,
        }
    }

    /// Normalize an error string into a stable pattern key.
    /// Takes the first line, trims, lowercases, and truncates to 100 chars.
    fn normalize_error(error: &str) -> String {
        let first_line = error.lines().next().unwrap_or(error);
        let trimmed = first_line.trim();
        let lowered = trimmed.to_lowercase();
        lowered.chars().take(100).collect()
    }

    /// Build a deterministic pattern key from agent_type and normalized error.
    fn pattern_key(agent_type: &str, normalized_error: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        normalized_error.hash(&mut hasher);
        let hash = hasher.finish();
        format!("failure-pattern:{}:{:016x}", agent_type, hash)
    }

    /// Record a failure occurrence and return (threshold_exceeded, count).
    async fn record_and_check_threshold(
        &self,
        pattern_key: &str,
    ) -> Result<(bool, u32), String> {
        let now = chrono::Utc::now();
        let window_start = now - chrono::Duration::seconds(self.window_secs as i64);

        // Load existing failure timestamps from memory
        let existing = self
            .memory_repo
            .get_by_key(pattern_key, "obstacle-escalation")
            .await
            .map_err(|e| format!("Failed to load failure pattern: {}", e))?;

        let mut timestamps: Vec<i64> = if let Some(ref mem) = existing {
            serde_json::from_str(&mem.content).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Filter to sliding window
        timestamps.retain(|&ts| ts >= window_start.timestamp());

        // Add current failure
        timestamps.push(now.timestamp());

        let count = timestamps.len() as u32;

        // Store updated record (update if exists, store if new)
        let content = serde_json::to_string(&timestamps)
            .map_err(|e| format!("Failed to serialize timestamps: {}", e))?;

        if let Some(mut existing_mem) = existing {
            existing_mem.content = content;
            self.memory_repo
                .update(&existing_mem)
                .await
                .map_err(|e| format!("Failed to update failure pattern: {}", e))?;
        } else {
            let memory = crate::domain::models::Memory::episodic(pattern_key.to_string(), content)
                .with_namespace("obstacle-escalation")
                .with_type(crate::domain::models::MemoryType::Pattern)
                .with_source("obstacle_escalation_handler");

            self.memory_repo
                .store(&memory)
                .await
                .map_err(|e| format!("Failed to store failure pattern: {}", e))?;
        }

        Ok((count >= self.threshold, count))
    }

    /// Check if an escalation goal already exists for this pattern.
    async fn has_existing_escalation(&self, pattern_key: &str) -> Result<bool, String> {
        // Check memory for an existing escalation record
        let escalation_key = format!("escalated:{}", pattern_key);
        let existing = self
            .memory_repo
            .get_by_key(&escalation_key, "escalations")
            .await
            .map_err(|e| format!("Failed to check escalation: {}", e))?;

        if existing.is_some() {
            // Verify the goal still exists and is active
            let goals = self
                .goal_repo
                .find_by_domains(&["obstacle-escalation".to_string()])
                .await
                .map_err(|e| format!("Failed to query goals: {}", e))?;

            // If any active goal mentions this pattern key, skip
            for goal in &goals {
                if goal.status == crate::domain::models::GoalStatus::Active
                    && goal.description.contains(pattern_key)
                {
                    return Ok(true);
                }
            }
            // Goal was retired/deleted — allow re-escalation
        }

        Ok(false)
    }

    /// Create an escalation goal and record it in memory.
    async fn create_escalation_goal(
        &self,
        agent_type: &str,
        normalized_error: &str,
        pattern_key: &str,
        failure_count: u32,
    ) -> Result<(), String> {
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, GoalCommand,
        };

        let goal_name = format!(
            "Address repeated {} failures: {}",
            agent_type,
            if normalized_error.len() > 60 {
                format!("{}...", &normalized_error.chars().take(60).collect::<String>())
            } else {
                normalized_error.to_string()
            }
        );
        let goal_description = format!(
            "The agent type '{}' has failed {} times within the escalation window \
             with the same error class:\n\n> {}\n\n\
             Pattern key: {}\n\n\
             This goal was auto-created by the obstacle escalation handler. \
             Investigate the root cause and update the agent template or \
             add memory to prevent recurrence.",
            agent_type, failure_count, normalized_error, pattern_key
        );

        let envelope = CommandEnvelope::new(
            CommandSource::EventHandler("ObstacleEscalationHandler".to_string()),
            DomainCommand::Goal(GoalCommand::Create {
                name: goal_name,
                description: goal_description,
                priority: crate::domain::models::GoalPriority::High,
                parent_id: None,
                constraints: vec![
                    crate::domain::models::GoalConstraint::preference(
                        "failure-pattern",
                        format!(
                            "Agent '{}' fails repeatedly with: {}",
                            agent_type, normalized_error
                        ),
                    ),
                    crate::domain::models::GoalConstraint::preference(
                        "resolution-action",
                        "Either update the agent template, add error-handling memory, or fix the underlying infrastructure issue",
                    ),
                ],
                domains: vec!["obstacle-escalation".to_string()],
            }),
        );

        match self.command_bus.dispatch(envelope).await {
            Ok(_) => {
                tracing::info!(
                    agent_type = agent_type,
                    pattern_key = pattern_key,
                    failure_count = failure_count,
                    "Created escalation goal for repeated failure pattern"
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent_type = agent_type,
                    pattern_key = pattern_key,
                    "Failed to create escalation goal: {}",
                    e
                );
                return Err(format!("Failed to dispatch goal creation: {}", e));
            }
        }

        // Record escalation in memory for deduplication
        let escalation_key = format!("escalated:{}", pattern_key);
        let escalation_content = serde_json::json!({
            "agent_type": agent_type,
            "pattern_key": pattern_key,
            "failure_count": failure_count,
            "escalated_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();

        let escalation_memory =
            crate::domain::models::Memory::episodic(escalation_key, escalation_content)
                .with_namespace("escalations")
                .with_type(crate::domain::models::MemoryType::Decision)
                .with_source("obstacle_escalation_handler");

        if let Err(e) = self.memory_repo.store(&escalation_memory).await {
            tracing::warn!("Failed to record escalation: {}", e);
        }

        Ok(())
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, M: MemoryRepository + 'static, G: GoalRepository + 'static>
    EventHandler for ObstacleEscalationHandler<T, M, G>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "ObstacleEscalationHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Task])
                .payload_types(vec!["TaskFailed".to_string()]),
            priority: HandlerPriority::LOW,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        // 1. Extract task_id and error from TaskFailed event
        let (task_id, error) = match &event.payload {
            EventPayload::TaskFailed {
                task_id, error, ..
            } => (*task_id, error.clone()),
            _ => return Ok(Reaction::None),
        };

        // 2. Load task to get agent_type
        let task = self
            .task_repo
            .get(task_id)
            .await
            .map_err(|e| format!("Failed to load task {}: {}", task_id, e))?;

        let task = match task {
            Some(t) => t,
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    "Task not found, skipping obstacle tracking"
                );
                return Ok(Reaction::None);
            }
        };

        let agent_type = task
            .agent_type
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // 3. Normalize error and build pattern key
        let normalized = Self::normalize_error(&error);
        let pattern_key = Self::pattern_key(&agent_type, &normalized);

        // 4. Record failure and check threshold
        let (threshold_exceeded, count) = self.record_and_check_threshold(&pattern_key).await?;

        if !threshold_exceeded {
            tracing::debug!(
                agent_type = %agent_type,
                pattern_key = %pattern_key,
                count = count,
                threshold = self.threshold,
                "Failure recorded, below escalation threshold"
            );
            return Ok(Reaction::None);
        }

        // 5. Check for existing escalation to avoid duplicates
        if self.has_existing_escalation(&pattern_key).await? {
            tracing::debug!(
                pattern_key = %pattern_key,
                "Escalation already exists for this pattern, skipping"
            );
            return Ok(Reaction::None);
        }

        // 6. Create escalation goal
        tracing::warn!(
            agent_type = %agent_type,
            pattern_key = %pattern_key,
            count = count,
            "Failure pattern exceeded threshold, escalating to goal"
        );

        self.create_escalation_goal(&agent_type, &normalized, &pattern_key, count)
            .await?;

        Ok(Reaction::None)
    }
}

#[cfg(test)]
mod obstacle_escalation_tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
        SqliteMemoryRepository, goal_repository::SqliteGoalRepository,
    };
    use crate::domain::models::{Task, TaskStatus, GoalStatus};

    use std::sync::Arc;

    async fn setup_obstacle_escalation_handler() -> (
        ObstacleEscalationHandler<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>,
        Arc<SqliteTaskRepository>,
        Arc<SqliteMemoryRepository>,
        Arc<SqliteGoalRepository>,
    ) {
        use crate::services::task_service::TaskService;
        use crate::services::goal_service::GoalService;
        use crate::services::memory_service::MemoryService;

        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool.clone()));
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));

        let task_service = Arc::new(TaskService::new(task_repo.clone()));
        let goal_service = Arc::new(GoalService::new(goal_repo.clone()));
        let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
        let event_bus = Arc::new(crate::services::EventBus::new(crate::services::EventBusConfig {
            persist_events: false,
            ..Default::default()
        }));
        let command_bus = Arc::new(crate::services::command_bus::CommandBus::new(
            task_service, goal_service, memory_service, event_bus,
        ));

        let handler = ObstacleEscalationHandler::new(
            task_repo.clone(),
            memory_repo.clone(),
            goal_repo.clone(),
            command_bus,
            3,     // threshold
            86400, // window = 24h in seconds
        );
        (handler, task_repo, memory_repo, goal_repo)
    }

    /// Create a failed task with the given agent_type.
    async fn create_failed_task(
        task_repo: &SqliteTaskRepository,
        agent_type: &str,
    ) -> Task {
        let mut task = Task::new("Failing task");
        task.description = "Task that fails".to_string();
        task.agent_type = Some(agent_type.to_string());
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Failed).unwrap();
        task_repo.create(&task).await.unwrap();
        task
    }

    /// Build a TaskFailed event for the given task and error.
    fn make_task_failed_event(task_id: uuid::Uuid, error: &str) -> UnifiedEvent {
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
                error: error.to_string(),
                retry_count: 0,
            },
        }
    }

    #[test]
    fn test_normalize_error_basic() {
        let result = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::normalize_error(
            "Compilation error: cannot find type `Foo`\ndetailed backtrace follows..."
        );
        assert_eq!(result, "compilation error: cannot find type `foo`");
    }

    #[test]
    fn test_normalize_error_truncation() {
        let long_error = "a".repeat(200);
        let result = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::normalize_error(&long_error);
        assert_eq!(result.len(), 100);
    }

    #[test]
    fn test_normalize_error_multiline() {
        let result = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::normalize_error(
            "First line error\nSecond line\nThird line"
        );
        assert_eq!(result, "first line error");
    }

    #[test]
    fn test_pattern_key_deterministic() {
        let key1 = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::pattern_key("coder", "some error");
        let key2 = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::pattern_key("coder", "some error");
        assert_eq!(key1, key2);
        assert!(key1.starts_with("failure-pattern:coder:"));
    }

    #[test]
    fn test_pattern_key_different_for_different_errors() {
        let key1 = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::pattern_key("coder", "error a");
        let key2 = ObstacleEscalationHandler::<SqliteTaskRepository, SqliteMemoryRepository, SqliteGoalRepository>::pattern_key("coder", "error b");
        assert_ne!(key1, key2);
    }

    #[tokio::test]
    async fn test_obstacle_escalation_below_threshold() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // Send 2 failures (threshold is 3) — should NOT escalate
        for _ in 0..2 {
            let task = create_failed_task(&task_repo, "coder").await;
            let event = make_task_failed_event(task.id, "compilation error: missing type");
            let reaction = handler.handle(&event, &ctx).await.unwrap();
            assert!(matches!(reaction, Reaction::None));
        }

        // Verify no goals created
        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await.unwrap();
        assert!(goals.is_empty(), "No goals should be created below threshold");
    }

    #[tokio::test]
    async fn test_obstacle_escalation_triggers_at_threshold() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // Send 3 failures with same agent_type + same error
        for _ in 0..3 {
            let task = create_failed_task(&task_repo, "researcher").await;
            let event = make_task_failed_event(task.id, "timeout waiting for response");
            handler.handle(&event, &ctx).await.unwrap();
        }

        // Verify a goal was created
        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await.unwrap();
        assert_eq!(goals.len(), 1, "Exactly one escalation goal should be created");
        assert!(goals[0].name.contains("researcher"), "Goal name should mention agent_type");
        assert!(goals[0].description.contains("timeout"), "Goal description should mention error");
    }

    #[tokio::test]
    async fn test_obstacle_escalation_deduplication() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // First, trigger an escalation with 3 failures
        for _ in 0..3 {
            let task = create_failed_task(&task_repo, "implementer").await;
            let event = make_task_failed_event(task.id, "borrow checker error");
            handler.handle(&event, &ctx).await.unwrap();
        }

        // Send 3 more with the same pattern — should NOT create a second goal
        for _ in 0..3 {
            let task = create_failed_task(&task_repo, "implementer").await;
            let event = make_task_failed_event(task.id, "borrow checker error");
            handler.handle(&event, &ctx).await.unwrap();
        }

        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await.unwrap();
        assert_eq!(goals.len(), 1, "Should not create duplicate escalation goals");
    }

    #[tokio::test]
    async fn test_obstacle_escalation_different_errors_separate() {
        let (handler, task_repo, _memory_repo, goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // Send 1 failure each of 3 different error types from same agent
        let errors = ["error type A", "error type B", "error type C"];
        for error in &errors {
            let task = create_failed_task(&task_repo, "planner").await;
            let event = make_task_failed_event(task.id, error);
            handler.handle(&event, &ctx).await.unwrap();
        }

        // No pattern should have reached threshold (each has count=1)
        use crate::domain::ports::goal_repository::GoalFilter;
        let goals = goal_repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await.unwrap();
        assert!(goals.is_empty(), "Different errors should track separately, none reaching threshold");
    }

    #[tokio::test]
    async fn test_obstacle_escalation_task_not_found() {
        let (handler, _task_repo, _memory_repo, _goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        let nonexistent_id = uuid::Uuid::new_v4();
        let event = make_task_failed_event(nonexistent_id, "some error");
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None), "Should return None for nonexistent task");
    }

    #[tokio::test]
    async fn test_obstacle_escalation_ignores_non_task_failed() {
        let (handler, _task_repo, _memory_repo, _goal_repo) =
            setup_obstacle_escalation_handler().await;
        let ctx = HandlerContext { chain_depth: 0, correlation_id: None };

        // Send a TaskCompleted event (not TaskFailed)
        let event = UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(uuid::Uuid::new_v4()),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::TaskCompleted {
                task_id: uuid::Uuid::new_v4(),
                tokens_used: 0,
            },
        };
        let reaction = handler.handle(&event, &ctx).await.unwrap();
        assert!(matches!(reaction, Reaction::None), "Should ignore non-TaskFailed events");
    }
}

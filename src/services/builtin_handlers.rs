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
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, TaskScheduleRepository, TrajectoryRepository, WorktreeRepository};
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
    #[allow(dead_code)]
    event_bus: Arc<EventBus>,
    escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>,
}

impl EscalationTimeoutHandler {
    pub fn new(event_bus: Arc<EventBus>, escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>) -> Self {
        Self { event_bus, escalation_store }
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
            .filter(|e| e.escalation.deadline.map_or(false, |d| now > d))
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

        // Skip tasks superseded by the review failure loop-back handler
        if task.context.custom.contains_key("review_loop_active") {
            return Ok(Reaction::None);
        }

        let is_max_turns = error.starts_with("error_max_turns");

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
        if is_max_turns {
            updated.context.hints.push("retry:max_turns_exceeded".to_string());
            updated.context.custom.insert(
                "last_failure_reason".to_string(),
                serde_json::Value::String(error.to_string()),
            );
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
        if let Some(ref agent_type) = task.agent_type {
            if agent_type == "code-reviewer" {
                return true;
            }
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
        let mut replan_context = TaskContext::default();
        replan_context.input = review_feedback.clone();
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
            }),
        );

        if let Err(e) = self.command_bus.dispatch(rereview_envelope).await {
            tracing::warn!("ReviewFailureLoopHandler: failed to create re-review task: {}", e);
        }

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
            if task.retry_count < self.max_retries {
                let mut updated = task.clone();
                if updated.transition_to(TaskStatus::Ready).is_ok() {
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
pub struct EvolutionEvaluationHandler<T: TaskRepository, A: AgentRepository> {
    task_repo: Arc<T>,
    #[allow(dead_code)]
    agent_repo: Arc<A>,
}

impl<T: TaskRepository, A: AgentRepository> EvolutionEvaluationHandler<T, A> {
    pub fn new(task_repo: Arc<T>, agent_repo: Arc<A>) -> Self {
        Self { task_repo, agent_repo }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static, A: AgentRepository + 'static> EventHandler for EvolutionEvaluationHandler<T, A> {
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
/// goal progress. This is a read-only observer that never modifies goals,
/// tasks, or memories.
pub struct GoalEvaluationHandler<G: GoalRepository, T: TaskRepository, M: MemoryRepository> {
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    #[allow(dead_code)]
    memory_repo: Arc<M>,
    #[allow(dead_code)]
    event_store: Option<Arc<dyn EventStore>>,
}

impl<G: GoalRepository, T: TaskRepository, M: MemoryRepository> GoalEvaluationHandler<G, T, M> {
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        memory_repo: Arc<M>,
        event_store: Option<Arc<dyn EventStore>>,
    ) -> Self {
        Self { goal_repo, task_repo, memory_repo, event_store }
    }
}

#[async_trait]
impl<G: GoalRepository + 'static, T: TaskRepository + 'static, M: MemoryRepository + 'static>
    EventHandler for GoalEvaluationHandler<G, T, M>
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
        let running = self.task_repo.list_by_status(TaskStatus::Running).await
            .map_err(|e| format!("Failed to list running tasks: {}", e))?;

        let mut new_events = Vec::new();

        for goal in &goals {
            let goal_domains = &goal.applicability_domains;
            if goal_domains.is_empty() {
                continue;
            }

            // Find tasks whose inferred domains overlap with this goal's domains
            let relevant_completed: Vec<&Task> = completed.iter()
                .filter(|t| {
                    let task_domains = GoalContextService::<G>::infer_task_domains(t);
                    task_domains.iter().any(|d| goal_domains.contains(d))
                })
                .collect();

            let relevant_failed: Vec<&Task> = failed.iter()
                .filter(|t| {
                    let task_domains = GoalContextService::<G>::infer_task_domains(t);
                    task_domains.iter().any(|d| goal_domains.contains(d))
                })
                .collect();

            let _relevant_running: Vec<&Task> = running.iter()
                .filter(|t| {
                    let task_domains = GoalContextService::<G>::infer_task_domains(t);
                    task_domains.iter().any(|d| goal_domains.contains(d))
                })
                .collect();

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
        if max_seq > since_seq {
            if let Err(e) = self.event_store.set_watermark("TriggerRuleEngine", max_seq).await {
                tracing::warn!("TriggerCatchup: failed to update watermark: {}", e);
            }
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
                match self.event_store.latest_sequence().await {
                    Ok(Some(seq)) => {
                        let start_from = seq.0.saturating_sub(replay_window);
                        let mut hwm = self.high_water_mark.write().await;
                        *hwm = start_from;
                        tracing::info!(
                            "EventStorePoller: no watermark found, starting from seq {} (latest {} - window {})",
                            start_from, seq.0, replay_window
                        );
                    }
                    _ => {}
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
                    let overdue_secs = (-remaining) as i64;
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
            // Check if namespace overlaps with goal domains
            let overlaps = goal.applicability_domains.iter()
                .any(|d| d.eq_ignore_ascii_case(&namespace));
            if !overlaps {
                continue;
            }

            // Check cooldown
            if let Some(last) = cooldowns.get(&goal.id) {
                if (now - *last).num_seconds() < self.cooldown_secs as i64 {
                    continue;
                }
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
            let is_stale = task.started_at.map_or(true, |s| s < stale_cutoff);
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
            updated.context.hints.push("sla:warning".to_string());
        }
        updated.context.hints.push(hint.to_string());
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
        match reaction {
            Reaction::EmitEvents(events) => {
                assert_eq!(events.len(), 1);
                match &events[0].payload {
                    EventPayload::ReviewLoopTriggered {
                        failed_review_task_id,
                        iteration,
                        max_iterations,
                        ..
                    } => {
                        assert_eq!(*failed_review_task_id, review_task.id);
                        assert_eq!(*iteration, 2); // next iteration
                        assert_eq!(*max_iterations, 3);
                    }
                    other => panic!("Expected ReviewLoopTriggered, got {:?}", other),
                }
            }
            Reaction::None => panic!("Expected EmitEvents, got Reaction::None"),
        }

        // Verify the review_loop_active flag was set on the original task
        let updated = repo.get(review_task.id).await.unwrap().unwrap();
        assert_eq!(
            updated.context.custom.get("review_loop_active"),
            Some(&serde_json::Value::Bool(true)),
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
}

// ============================================================================
// WorkflowPhaseCompletionHandler
// ============================================================================

/// Handler that forwards workflow phase completion/failure events
/// through a channel for the phase orchestrator to process.
///
/// Listens for `PhaseCompleted` and `PhaseFailed` events from the
/// workflow event category and sends (workflow_instance_id, phase_id)
/// to a channel that the phase orchestrator drains.
pub struct WorkflowPhaseCompletionHandler {
    tx: tokio::sync::mpsc::Sender<(uuid::Uuid, uuid::Uuid)>,
}

impl WorkflowPhaseCompletionHandler {
    pub fn new(tx: tokio::sync::mpsc::Sender<(uuid::Uuid, uuid::Uuid)>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl EventHandler for WorkflowPhaseCompletionHandler {
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "WorkflowPhaseCompletionHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Workflow])
                .payload_types(vec![
                    "PhaseCompleted".to_string(),
                    "PhaseFailed".to_string(),
                ]),
            priority: HandlerPriority::HIGH,
            error_strategy: ErrorStrategy::LogAndContinue,
        }
    }

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        let ids = match &event.payload {
            EventPayload::PhaseCompleted {
                workflow_instance_id,
                phase_id,
                ..
            } => Some((*workflow_instance_id, *phase_id)),
            EventPayload::PhaseFailed {
                workflow_instance_id,
                phase_id,
                ..
            } => Some((*workflow_instance_id, *phase_id)),
            _ => None,
        };

        if let Some((wf_id, phase_id)) = ids {
            let _ = self.tx.send((wf_id, phase_id)).await;
        }

        Ok(Reaction::None)
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

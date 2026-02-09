//! Built-in reactive event handlers.
//!
//! All handlers are **idempotent** — safe to run even if the poll loop already
//! handled the same state change. They check current state before acting.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::domain::models::{Goal, HumanEscalationEvent, Task, TaskStatus};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::event_store::EventStore;
use crate::services::goal_context_service::GoalContextService;
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber,
    SwarmStatsPayload, UnifiedEvent,
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
        let report = self.memory_service.run_maintenance().await
            .map_err(|e| format!("Memory maintenance failed: {}", e))?;

        let mut events = Vec::new();

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

/// When a goal is retired, log the event and refresh caches.
///
/// Design constraint: Goals and tasks are intentionally decoupled.
/// Goals do NOT trigger task creation or cancellation. The Task model
/// does not carry a goal_id field. This handler only logs the retirement
/// event for observability.
pub struct GoalRetiredHandler {
    _placeholder: (),
}

impl GoalRetiredHandler {
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}

#[async_trait]
impl EventHandler for GoalRetiredHandler {
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
        if let Some(goal_id) = event.goal_id {
            tracing::info!("GoalRetiredHandler: goal {} retired", goal_id);
        }
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

    async fn handle(&self, _event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        // Check escalation deadlines from the shared store
        let now = chrono::Utc::now();
        let store = self.escalation_store.read().await;
        let timed_out_count = store.iter()
            .filter(|e| e.escalation.deadline.map_or(false, |d| now > d))
            .count();
        drop(store);

        if timed_out_count > 0 {
            tracing::info!("EscalationTimeoutHandler: {} escalation(s) past deadline", timed_out_count);
        }

        Ok(Reaction::None)
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
        let (task_id, retry_count) = match &event.payload {
            EventPayload::TaskFailed { task_id, retry_count, .. } => (*task_id, *retry_count),
            _ => return Ok(Reaction::None),
        };

        if retry_count >= self.max_retries {
            return Ok(Reaction::None);
        }

        // Re-fetch task to check it's still Failed (idempotency)
        let task = self.task_repo.get(task_id).await
            .map_err(|e| format!("Failed to get task: {}", e))?
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if task.status != TaskStatus::Failed {
            return Ok(Reaction::None);
        }

        // Exponential backoff: 2^retry_count seconds minimum wait
        let backoff_secs = 2u64.pow(task.retry_count.min(10));
        if let Some(completed_at) = task.completed_at {
            let elapsed = (chrono::Utc::now() - completed_at).num_seconds();
            if elapsed < backoff_secs as i64 {
                // Not ready to retry yet; the scheduled retry-check will try again
                return Ok(Reaction::None);
            }
        }

        let mut updated = task.clone();
        if updated.transition_to(TaskStatus::Ready).is_ok() {
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
                        attempt: retry_count + 1,
                        max_attempts: self.max_retries,
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

        // Check Blocked tasks that might now be unblocked
        let blocked = self.task_repo.list_by_status(TaskStatus::Blocked).await
            .map_err(|e| format!("Failed to list blocked tasks: {}", e))?;

        for task in &blocked {
            let deps = self.task_repo.get_dependencies(task.id).await
                .map_err(|e| format!("Failed to get deps: {}", e))?;

            if deps.iter().any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled) {
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
        let running = self.task_repo.list_by_status(TaskStatus::Running).await
            .map_err(|e| format!("Failed to list running tasks: {}", e))?;

        let now = chrono::Utc::now();
        let timeout = chrono::Duration::seconds(self.stale_task_timeout_secs as i64);

        for task in &running {
            if let Some(started_at) = task.started_at {
                if now - started_at > timeout {
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
/// Polls the A2A gateway for pending inbound delegations, creates tasks,
/// and emits TaskSubmitted for each.
pub struct A2APollHandler<T: TaskRepository> {
    task_repo: Arc<T>,
    a2a_gateway_url: String,
}

impl<T: TaskRepository> A2APollHandler<T> {
    pub fn new(task_repo: Arc<T>, a2a_gateway_url: String) -> Self {
        Self { task_repo, a2a_gateway_url }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> EventHandler for A2APollHandler<T> {
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

    async fn handle(&self, event: &UnifiedEvent, _ctx: &HandlerContext) -> Result<Reaction, String> {
        // Poll A2A gateway for pending inbound delegations
        let url = format!("{}/tasks/pending", self.a2a_gateway_url);
        let client = reqwest::Client::new();

        let response = match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::debug!("A2APollHandler: gateway unreachable: {}", e);
                return Ok(Reaction::None);
            }
        };

        if !response.status().is_success() {
            return Ok(Reaction::None);
        }

        let delegations: Vec<serde_json::Value> = match response.json().await {
            Ok(d) => d,
            Err(_) => return Ok(Reaction::None),
        };

        let mut new_events = Vec::new();

        for delegation in delegations {
            let title = delegation.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("A2A Delegated Task")
                .to_string();
            let description = delegation.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let mut task = Task::new(&description);
            task.title = title.clone();
            task.source = crate::domain::models::TaskSource::System;

            if let Err(e) = self.task_repo.create(&task).await {
                tracing::warn!("A2APollHandler: failed to create task: {}", e);
                continue;
            }

            let goal_id = uuid::Uuid::new_v4(); // Placeholder for A2A delegations
            new_events.push(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Task,
                goal_id: Some(goal_id),
                task_id: Some(task.id),
                correlation_id: event.correlation_id,
                source_process_id: None,
                payload: EventPayload::TaskSubmitted {
                    task_id: task.id,
                    task_title: title,
                    goal_id,
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

        let report = self.memory_service.run_maintenance().await
            .map_err(|e| format!("Memory reconciliation failed: {}", e))?;

        tracing::info!(
            expired = report.expired_pruned,
            decayed = report.decayed_pruned,
            promoted = report.promoted,
            conflicts = report.conflicts_resolved,
            "MemoryReconciliationHandler: maintenance complete"
        );

        Ok(Reaction::None)
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
                tracing::warn!(
                    worktree_id = %wt.id,
                    task_id = %wt.task_id,
                    path = %wt.path,
                    "WorktreeReconciliation: orphaned worktree detected (task is terminal or missing)"
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
                    payload: EventPayload::ReconciliationCompleted {
                        corrections_made: 0, // Flagging only, not correcting
                    },
                });
            }
        }

        if orphan_count > 0 {
            tracing::info!(
                "WorktreeReconciliation: {} orphaned worktree(s) detected",
                orphan_count
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

        for name in &self.handler_names {
            let wm = self.event_store.get_watermark(name).await
                .map_err(|e| format!("WatermarkAudit: failed to get watermark for {}: {}", name, e))?;

            let handler_seq = wm.map(|s| s.0).unwrap_or(0);
            let lag = latest_seq.saturating_sub(handler_seq);

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
        } else {
            tracing::debug!(
                "WatermarkAudit: all handlers within 100 events of sequence {}",
                latest_seq
            );
        }

        Ok(Reaction::None)
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
    pub async fn initialize_watermark(&self) {
        match self.event_store.get_watermark("EventStorePollerHandler").await {
            Ok(Some(seq)) => {
                let mut hwm = self.high_water_mark.write().await;
                *hwm = seq.0;
                tracing::info!("EventStorePoller: initialized watermark at {}", seq.0);
            }
            Ok(None) => {
                // No watermark yet — start from the latest sequence to avoid replaying history
                match self.event_store.latest_sequence().await {
                    Ok(Some(seq)) => {
                        let mut hwm = self.high_water_mark.write().await;
                        *hwm = seq.0;
                        tracing::info!("EventStorePoller: no watermark found, starting from latest seq {}", seq.0);
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
            // Re-fetch the original event from the store
            let original = self.event_store
                .get_by_sequence(SequenceNumber(entry.event_sequence))
                .await
                .map_err(|e| format!("DeadLetterRetry: failed to get event seq {}: {}", entry.event_sequence, e))?;

            match original {
                Some(evt) => {
                    events_to_replay.push(evt);

                    // Calculate next retry with exponential backoff
                    let backoff_secs = 2i64.pow((entry.retry_count + 1).min(10));
                    let next_retry = chrono::Utc::now() + chrono::Duration::seconds(backoff_secs);

                    if let Err(e) = self.event_store.increment_dead_letter_retry(&entry.id, next_retry).await {
                        tracing::warn!("DeadLetterRetry: failed to increment retry for {}: {}", entry.id, e);
                    }
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

            // If retry_count + 1 >= max_retries, resolve it (this was the last attempt)
            if entry.retry_count + 1 >= entry.max_retries {
                tracing::info!(
                    "DeadLetterRetry: max retries ({}) reached for handler '{}' on event seq {}, resolving",
                    entry.max_retries, entry.handler_name, entry.event_sequence
                );
                if let Err(e) = self.event_store.resolve_dead_letter(&entry.id).await {
                    tracing::warn!("DeadLetterRetry: failed to resolve entry {}: {}", entry.id, e);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::domain::models::{Task, TaskStatus};

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
}

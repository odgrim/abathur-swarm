//! Handler registration and schedule setup for the event-driven architecture.
//!
//! Registers all built-in handlers with the EventReactor and all scheduled
//! events with the EventScheduler.

use std::sync::Arc;
use std::time::Duration;

use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository,
};
use crate::services::builtin_handlers::{
    A2APollHandler, EscalationTimeoutHandler, EvolutionEvaluationHandler,
    GoalCreatedHandler, GoalEvaluationHandler, GoalRetiredHandler,
    MemoryMaintenanceHandler, ReconciliationHandler, RetryProcessingHandler,
    SpecialistCheckHandler, StatsUpdateHandler, TaskCompletedReadinessHandler,
    TaskFailedBlockHandler, TaskFailedRetryHandler, TaskReadySpawnHandler,
};
use crate::services::event_bus::EventCategory;
use crate::services::event_bus::EventSeverity;
use crate::services::event_scheduler::interval_schedule;
use crate::services::memory_service::MemoryService;

use super::SwarmOrchestrator;

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Register all built-in event handlers with the reactor.
    ///
    /// Called in `run()` after reactor start but before the main loop.
    pub(super) async fn register_builtin_handlers(&self) {
        let reactor = &self.event_reactor;

        // TaskCompletedReadinessHandler (SYSTEM) — cascade readiness on completion
        reactor
            .register(Arc::new(TaskCompletedReadinessHandler::new(
                self.task_repo.clone(),
            )))
            .await;

        // TaskFailedBlockHandler (SYSTEM) — block dependents on failure/cancel
        reactor
            .register(Arc::new(TaskFailedBlockHandler::new(
                self.task_repo.clone(),
            )))
            .await;

        // TaskFailedRetryHandler (NORMAL) — retry after failure if retries remain
        reactor
            .register(Arc::new(TaskFailedRetryHandler::new(
                self.task_repo.clone(),
                self.config.max_task_retries,
            )))
            .await;

        // GoalCreatedHandler (NORMAL) — refresh active goals cache
        reactor
            .register(Arc::new(GoalCreatedHandler::new(
                self.goal_repo.clone(),
                self.active_goals_cache.clone(),
            )))
            .await;

        // GoalRetiredHandler (HIGH) — log goal retirement (no task coupling)
        reactor
            .register(Arc::new(GoalRetiredHandler::new()))
            .await;

        // StatsUpdateHandler (LOW) — periodic stats refresh
        reactor
            .register(Arc::new(StatsUpdateHandler::new(
                self.goal_repo.clone(),
                self.task_repo.clone(),
                self.worktree_repo.clone(),
                self.stats.clone(),
                self.agent_semaphore.clone(),
                self.config.max_agents,
                self.total_tokens.clone(),
            )))
            .await;

        // ReconciliationHandler (LOW) — periodic safety-net reconciliation
        reactor
            .register(Arc::new(ReconciliationHandler::new(
                self.task_repo.clone(),
            )))
            .await;

        // RetryProcessingHandler (NORMAL) — periodic retry sweep
        if self.config.auto_retry {
            reactor
                .register(Arc::new(RetryProcessingHandler::new(
                    self.task_repo.clone(),
                    self.config.max_task_retries,
                )))
                .await;
        }

        // EscalationTimeoutHandler (NORMAL) — check escalation deadlines
        reactor
            .register(Arc::new(EscalationTimeoutHandler::new(
                self.event_bus.clone(),
                self.escalation_store.clone(),
            )))
            .await;

        // MemoryMaintenanceHandler (NORMAL) — periodic memory maintenance
        if let Some(ref memory_repo) = self.memory_repo {
            let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
            reactor
                .register(Arc::new(MemoryMaintenanceHandler::new(memory_service)))
                .await;
        }

        // TaskReadySpawnHandler (NORMAL) — push ready tasks to spawn channel
        reactor
            .register(Arc::new(TaskReadySpawnHandler::new(
                self.task_repo.clone(),
                self.ready_task_tx.clone(),
            )))
            .await;

        // SpecialistCheckHandler (NORMAL) — scan for stuck/failed tasks needing specialists
        reactor
            .register(Arc::new(SpecialistCheckHandler::new(
                self.task_repo.clone(),
                self.specialist_tx.clone(),
                self.config.max_task_retries,
            )))
            .await;

        // EvolutionEvaluationHandler (NORMAL) — track and refine agent templates
        if self.config.track_evolution {
            reactor
                .register(Arc::new(EvolutionEvaluationHandler::new(
                    self.task_repo.clone(),
                    self.agent_repo.clone(),
                )))
                .await;
        }

        // A2APollHandler (NORMAL) — poll A2A gateway for delegations
        if let Some(ref a2a_url) = self.config.mcp_servers.a2a_gateway {
            reactor
                .register(Arc::new(A2APollHandler::new(
                    self.task_repo.clone(),
                    a2a_url.clone(),
                )))
                .await;
        }

        // GoalEvaluationHandler (NORMAL) — periodic goal progress observation
        if let Some(ref memory_repo) = self.memory_repo {
            reactor
                .register(Arc::new(GoalEvaluationHandler::new(
                    self.goal_repo.clone(),
                    self.task_repo.clone(),
                    memory_repo.clone(),
                    None, // event_store is optional for goal evaluation
                )))
                .await;
        }
    }

    /// Register built-in scheduled events with the scheduler.
    ///
    /// Called in `run()` after handler registration.
    pub(super) async fn register_builtin_schedules(&self) {
        let scheduler = &self.event_scheduler;

        let reconciliation_secs = self.config.reconciliation_interval_secs.unwrap_or(30);

        // Reconciliation — safety net for missed transitions
        scheduler
            .register(interval_schedule(
                "reconciliation",
                Duration::from_secs(reconciliation_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Stats update — refresh swarm statistics
        scheduler
            .register(interval_schedule(
                "stats-update",
                Duration::from_secs(10),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Escalation check — check escalation deadlines
        scheduler
            .register(interval_schedule(
                "escalation-check",
                Duration::from_secs(30),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Memory maintenance — prune, promote, resolve conflicts
        if self.memory_repo.is_some() {
            scheduler
                .register(interval_schedule(
                    "memory-maintenance",
                    Duration::from_secs(300), // 5 minutes
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Retry check — periodic retry sweep for failed tasks
        if self.config.auto_retry {
            scheduler
                .register(interval_schedule(
                    "retry-check",
                    Duration::from_secs(15),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Specialist check — scan for stuck/failed tasks needing specialists
        scheduler
            .register(interval_schedule(
                "specialist-check",
                Duration::from_secs(30),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Evolution evaluation — track and refine agent templates
        if self.config.track_evolution {
            scheduler
                .register(interval_schedule(
                    "evolution-evaluation",
                    Duration::from_secs(120), // 2 minutes
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // A2A delegation polling
        if self.config.mcp_servers.a2a_gateway.is_some() {
            scheduler
                .register(interval_schedule(
                    "a2a-poll",
                    Duration::from_secs(15),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Goal evaluation — periodic observation of goal progress
        if self.memory_repo.is_some() {
            scheduler
                .register(interval_schedule(
                    "goal-evaluation",
                    Duration::from_secs(60),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }
    }
}

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
    A2APollHandler, DeadLetterRetryHandler, EscalationTimeoutHandler,
    EventPruningHandler, EventStorePollerHandler, EvolutionEvaluationHandler,
    GoalCreatedHandler, GoalEvaluationHandler, GoalReconciliationHandler, GoalRetiredHandler,
    MemoryMaintenanceHandler, MemoryReconciliationHandler, ReconciliationHandler,
    RetryProcessingHandler, SpecialistCheckHandler, StatsUpdateHandler,
    TaskCompletedReadinessHandler, TaskFailedBlockHandler, TaskFailedRetryHandler,
    TaskReadySpawnHandler, TriggerCatchupHandler, WatermarkAuditHandler,
    WorktreeReconciliationHandler,
};
use crate::services::command_bus::CommandBus;
use crate::services::event_bus::EventCategory;
use crate::services::event_bus::EventSeverity;
use crate::services::event_scheduler::interval_schedule;
use crate::services::goal_service::GoalService;
use crate::services::memory_service::MemoryService;
use crate::services::task_service::TaskService;
use crate::services::trigger_rules::{TriggerRuleEngine, builtin_trigger_rules};

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

        // WorktreeReconciliationHandler (LOW) — detect orphaned worktrees
        reactor
            .register(Arc::new(WorktreeReconciliationHandler::new(
                self.task_repo.clone(),
                self.worktree_repo.clone(),
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
                .register(Arc::new(MemoryMaintenanceHandler::new(memory_service.clone())))
                .await;

            // MemoryReconciliationHandler (LOW) — periodic memory reconciliation
            reactor
                .register(Arc::new(MemoryReconciliationHandler::new(memory_service)))
                .await;
        }

        // GoalReconciliationHandler (LOW) — periodic goal reconciliation
        reactor
            .register(Arc::new(GoalReconciliationHandler::new(
                self.goal_repo.clone(),
            )))
            .await;

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

        // Build a shared CommandBus for handlers that need to dispatch commands
        let command_bus = {
            use crate::domain::ports::NullMemoryRepository;

            let task_service = Arc::new(TaskService::new(
                self.task_repo.clone(),
            ));
            let goal_service = Arc::new(GoalService::new(
                self.goal_repo.clone(),
            ));

            if let Some(ref memory_repo) = self.memory_repo {
                let memory_service = Arc::new(MemoryService::new(
                    memory_repo.clone(),
                ));
                Arc::new(CommandBus::new(task_service, goal_service, memory_service, self.event_bus.clone()))
            } else {
                let null_memory = Arc::new(MemoryService::new(
                    Arc::new(NullMemoryRepository::new()),
                ));
                Arc::new(CommandBus::new(task_service, goal_service, null_memory, self.event_bus.clone()))
            }
        };

        // A2APollHandler (NORMAL) — poll A2A gateway for delegations
        if let Some(ref a2a_url) = self.config.mcp_servers.a2a_gateway {
            reactor
                .register(Arc::new(A2APollHandler::new(
                    command_bus.clone(),
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

        // TriggerRuleEngine (NORMAL) — declarative event-driven automation
        let trigger_engine = {
            let mut engine_builder = TriggerRuleEngine::new(command_bus.clone())
                .with_event_bus(self.event_bus.clone());

            if let Some(ref repo) = self.trigger_rule_repo {
                engine_builder = engine_builder.with_rule_repo(repo.clone());
            }

            let engine = Arc::new(engine_builder);

            // Load rules: merge DB rules (if repo available) with built-in defaults
            if let Some(ref repo) = self.trigger_rule_repo {
                match repo.list().await {
                    Ok(db_rules) if !db_rules.is_empty() => {
                        // DB rules take precedence; seed built-ins that don't exist in DB
                        let mut merged = db_rules;
                        let builtins = builtin_trigger_rules();
                        for builtin in builtins {
                            if !merged.iter().any(|r| r.name == builtin.name) {
                                merged.push(builtin);
                            }
                        }
                        let count = merged.len();
                        engine.load_rules(merged).await;
                        tracing::info!("Loaded {} trigger rules (DB + built-in fallbacks)", count);
                    }
                    _ => {
                        // No DB rules or error: fall back to built-in defaults
                        let rules = builtin_trigger_rules();
                        let count = rules.len();
                        engine.load_rules(rules).await;
                        tracing::info!("Loaded {} built-in trigger rules (no DB rules found)", count);
                    }
                }
            } else {
                let rules = builtin_trigger_rules();
                let count = rules.len();
                engine.load_rules(rules).await;
                tracing::info!("Loaded {} built-in trigger rules", count);
            }

            reactor.register(engine.clone()).await;
            engine
        };

        // WatermarkAuditHandler + TriggerCatchupHandler + Poller/DLQ/Pruning (LOW) — require event store
        if let Some(event_store) = self.event_bus.store() {
            // Collect handler names for watermark auditing
            let handler_names: Vec<String> = reactor.handler_names().await;

            reactor
                .register(Arc::new(WatermarkAuditHandler::new(
                    event_store.clone(),
                    handler_names,
                )))
                .await;

            reactor
                .register(Arc::new(TriggerCatchupHandler::new(
                    trigger_engine,
                    event_store.clone(),
                )))
                .await;

            // EventStorePollerHandler (SYSTEM) — cross-process event propagation
            let poller = Arc::new(EventStorePollerHandler::new(
                event_store.clone(),
                self.event_bus.process_id(),
            ));
            poller.initialize_watermark().await;
            reactor.register(poller).await;

            // DeadLetterRetryHandler (LOW) — retry failed handler events
            reactor
                .register(Arc::new(DeadLetterRetryHandler::new(
                    event_store.clone(),
                )))
                .await;

            // EventPruningHandler (LOW) — prune old events
            reactor
                .register(Arc::new(EventPruningHandler::new(
                    event_store,
                    self.config.event_retention_days,
                )))
                .await;
        }
    }

    /// Register built-in scheduled events with the scheduler.
    ///
    /// Called in `run()` after handler registration.
    /// All intervals are configurable via `SwarmConfig.polling`.
    pub(super) async fn register_builtin_schedules(&self) {
        let scheduler = &self.event_scheduler;
        let p = &self.config.polling;

        // Use explicit override if set, otherwise fall back to PollingConfig
        let reconciliation_secs = self
            .config
            .reconciliation_interval_secs
            .unwrap_or(p.reconciliation_interval_secs);

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
                Duration::from_secs(p.stats_update_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Escalation check — check escalation deadlines
        scheduler
            .register(interval_schedule(
                "escalation-check",
                Duration::from_secs(p.escalation_check_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Memory maintenance — prune, promote, resolve conflicts
        if self.memory_repo.is_some() {
            scheduler
                .register(interval_schedule(
                    "memory-maintenance",
                    Duration::from_secs(p.memory_maintenance_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;

            // Memory reconciliation — periodic memory safety-net
            scheduler
                .register(interval_schedule(
                    "memory-reconciliation",
                    Duration::from_secs(p.memory_reconciliation_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Goal reconciliation — periodic goal staleness check
        scheduler
            .register(interval_schedule(
                "goal-reconciliation",
                Duration::from_secs(p.goal_reconciliation_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Retry check — periodic retry sweep for failed tasks
        if self.config.auto_retry {
            scheduler
                .register(interval_schedule(
                    "retry-check",
                    Duration::from_secs(p.retry_check_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Specialist check — scan for stuck/failed tasks needing specialists
        scheduler
            .register(interval_schedule(
                "specialist-check",
                Duration::from_secs(p.specialist_check_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Evolution evaluation — track and refine agent templates
        if self.config.track_evolution {
            scheduler
                .register(interval_schedule(
                    "evolution-evaluation",
                    Duration::from_secs(p.evolution_evaluation_interval_secs),
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
                    Duration::from_secs(p.a2a_poll_interval_secs),
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
                    Duration::from_secs(p.goal_evaluation_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Trigger rule catch-up — periodic sweep for missed trigger evaluations
        scheduler
            .register(interval_schedule(
                "trigger-rule-catchup",
                Duration::from_secs(p.trigger_catchup_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Watermark audit — verify handler watermark consistency
        scheduler
            .register(interval_schedule(
                "watermark-audit",
                Duration::from_secs(p.watermark_audit_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Event store polling — cross-process event propagation
        if self.event_bus.store().is_some() {
            scheduler
                .register(interval_schedule(
                    "event-store-poll",
                    Duration::from_secs(p.event_store_poll_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;

            // Dead letter retry — periodic DLQ retry sweep
            scheduler
                .register(interval_schedule(
                    "dead-letter-retry",
                    Duration::from_secs(p.dead_letter_retry_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;

            // Event pruning — remove old events based on retention policy
            scheduler
                .register(interval_schedule(
                    "event-pruning",
                    Duration::from_secs(p.event_pruning_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }
    }
}

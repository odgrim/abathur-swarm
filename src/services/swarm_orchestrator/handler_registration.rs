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
    A2APollHandler, AdapterLifecycleSyncHandler, ConvergenceCancellationHandler,
    ConvergenceCoordinationHandler,
    ConvergenceEscalationFeedbackHandler,
    ConvergenceEvolutionHandler, ConvergenceMemoryHandler, ConvergenceSLAPressureHandler,
    DeadLetterRetryHandler, DirectModeExecutionMemoryHandler, EscalationTimeoutHandler,
    EgressRoutingHandler,
    EventPruningHandler, EventStorePollerHandler, EvolutionEvaluationHandler,
    EvolutionTriggeredTemplateUpdateHandler,
    GoalConvergenceCheckHandler,
    GoalCreatedHandler, GoalEvaluationHandler, GoalEvaluationTaskCreationHandler,
    GoalReconciliationHandler, GoalRetiredHandler,
    IngestionPollHandler,
    MemoryConflictEscalationHandler, MemoryInformedDecompositionHandler,
    MemoryMaintenanceHandler, MemoryReconciliationHandler,
    PriorityAgingHandler, ReconciliationHandler,
    ReviewFailureLoopHandler, RetryProcessingHandler, SpecialistCheckHandler,
    StartupCatchUpHandler, StatsUpdateHandler, SystemStallDetectorHandler,
    TaskCompletionLearningHandler,
    TaskCompletedReadinessHandler, TaskFailedBlockHandler, TaskFailedRetryHandler,
    TaskOutcomeMemoryHandler,
    TaskReadySpawnHandler, TaskScheduleHandler, TaskSLAEnforcementHandler,
    TriggerCatchupHandler, WatermarkAuditHandler,
    WorkflowPhaseCompletionHandler,
    WorktreeReconciliationHandler,
};
use crate::services::command_bus::CommandBus;
use crate::services::convergence_bridge::DynTrajectoryRepository;
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
        let p = &self.config.polling;

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

        // ConvergenceCoordinationHandler (HIGH) — cascade child completion/failure to convergent parent
        reactor
            .register(Arc::new(ConvergenceCoordinationHandler::new(
                self.task_repo.clone(),
            )))
            .await;

        // ConvergenceCancellationHandler (HIGH) — cascade cancellation from convergent parent to children
        reactor
            .register(Arc::new(ConvergenceCancellationHandler::new(
                self.task_repo.clone(),
            )))
            .await;

        // ConvergenceSLAPressureHandler (HIGH) — add SLA pressure hints to convergent task context
        reactor
            .register(Arc::new(ConvergenceSLAPressureHandler::new(
                self.task_repo.clone(),
            )))
            .await;

        // ConvergenceEscalationFeedbackHandler (NORMAL) — feed human escalation responses back into convergence loop
        if let Some(ref trajectory_repo) = self.trajectory_repo {
            reactor
                .register(Arc::new(ConvergenceEscalationFeedbackHandler::new(
                    self.task_repo.clone(),
                    Arc::new(DynTrajectoryRepository(trajectory_repo.clone())),
                )))
                .await;
        }

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

        // GoalRetiredHandler (HIGH) — refresh cache on goal retirement (no task coupling)
        reactor
            .register(Arc::new(GoalRetiredHandler::new(
                self.goal_repo.clone(),
                self.active_goals_cache.clone(),
            )))
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

        // SystemStallDetectorHandler (LOW) — detect system-wide idle stalls
        {
            let threshold = p.goal_convergence_check_interval_secs.saturating_mul(2);
            reactor
                .register(Arc::new(SystemStallDetectorHandler::new(
                    self.task_repo.clone(),
                    threshold,
                )))
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

        // ConvergenceMemoryHandler (NORMAL) — record convergence outcomes to memory
        if let Some(ref memory_repo) = self.memory_repo {
            reactor
                .register(Arc::new(ConvergenceMemoryHandler::new(
                    self.task_repo.clone(),
                    memory_repo.clone(),
                )))
                .await;
        }

        // DirectModeExecutionMemoryHandler (NORMAL) — record all task executions for classification heuristic
        if let Some(ref memory_repo) = self.memory_repo {
            reactor
                .register(Arc::new(DirectModeExecutionMemoryHandler::new(
                    memory_repo.clone(),
                )))
                .await;
        }

        // TaskOutcomeMemoryHandler (NORMAL) — episodic memory for ALL task completions
        if let Some(ref memory_repo) = self.memory_repo {
            reactor
                .register(Arc::new(TaskOutcomeMemoryHandler::new(
                    self.task_repo.clone(),
                    memory_repo.clone(),
                )))
                .await;
        }

        // ConvergenceEvolutionHandler (NORMAL) — emit evolution metrics for convergent tasks
        if self.config.track_evolution {
            reactor
                .register(Arc::new(ConvergenceEvolutionHandler::new(
                    self.task_repo.clone(),
                )))
                .await;
        }

        // Build a shared CommandBus for handlers that need to dispatch commands.
        // Also stored on the orchestrator for use by goal_processing, specialist_triggers, etc.
        let command_bus = {
            use crate::domain::ports::NullMemoryRepository;

            let task_service = Arc::new(TaskService::new(
                self.task_repo.clone(),
            ).with_default_execution_mode(self.config.default_execution_mode.clone()));
            let goal_service = Arc::new(GoalService::new(
                self.goal_repo.clone(),
            ));

            let bus = if let Some(ref memory_repo) = self.memory_repo {
                let memory_service = Arc::new(MemoryService::new(
                    memory_repo.clone(),
                ));
                Arc::new(CommandBus::new(task_service, goal_service, memory_service, self.event_bus.clone()))
            } else {
                let null_memory = Arc::new(MemoryService::new(
                    Arc::new(NullMemoryRepository::new()),
                ));
                Arc::new(CommandBus::new(task_service, goal_service, null_memory, self.event_bus.clone()))
            };

            // Store on the orchestrator so other subsystems can use it
            {
                let mut stored = self.command_bus.write().await;
                *stored = Some(bus.clone());
            }

            bus
        };

        // WorkflowPhaseCompletionHandler (HIGH) — forward phase events to phase orchestrator
        if self.phase_orchestrator.is_some() {
            let (wf_tx, mut wf_rx) = tokio::sync::mpsc::channel::<(uuid::Uuid, uuid::Uuid)>(64);
            reactor
                .register(Arc::new(WorkflowPhaseCompletionHandler::new(wf_tx)))
                .await;

            // Spawn a background task that drains the channel and calls into the phase orchestrator
            let phase_orch = self.phase_orchestrator.clone().unwrap();
            tokio::spawn(async move {
                while let Some((workflow_instance_id, phase_id)) = wf_rx.recv().await {
                    if let Err(e) = phase_orch
                        .on_phase_tasks_completed(workflow_instance_id, phase_id)
                        .await
                    {
                        tracing::error!(
                            workflow_id = %workflow_instance_id,
                            phase_id = %phase_id,
                            error = %e,
                            "Failed to handle phase completion"
                        );
                    }
                }
            });
        }

        // ReviewFailureLoopHandler (HIGH) — loop review failures back to plan+implement
        reactor
            .register(Arc::new(ReviewFailureLoopHandler::new(
                self.task_repo.clone(),
                command_bus.clone(),
                self.config.max_review_iterations,
            )))
            .await;

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
            if let Some(ref pool) = self.pool {
                engine_builder = engine_builder.with_pool(pool.clone());
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
                                // Persist built-in rule so FK constraints on absence timers work
                                if let Err(e) = repo.create(&builtin).await {
                                    tracing::debug!("Could not seed built-in rule '{}': {}", builtin.name, e);
                                }
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
                        // Seed built-in rules into DB so FK constraints on absence timers work
                        for rule in &rules {
                            if let Err(e) = repo.create(rule).await {
                                tracing::debug!("Could not seed built-in rule '{}': {}", rule.name, e);
                            }
                        }
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

            // Restore persisted absence timers from DB
            engine.load_pending_timers().await;

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
            poller.initialize_watermark_with_replay(p.startup_max_replay_events).await;
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
                    event_store.clone(),
                    self.config.event_retention_days,
                )))
                .await;

            // StartupCatchUpHandler (SYSTEM) — fix orphaned tasks and replay missed events
            if p.startup_catchup_enabled {
                reactor
                    .register(Arc::new(StartupCatchUpHandler::new(
                        self.task_repo.clone(),
                        self.goal_repo.clone(),
                        event_store,
                        p.startup_stale_task_threshold_secs,
                        p.startup_max_replay_events,
                    )))
                    .await;
            }
        }

        // TaskSLAEnforcementHandler (NORMAL) — periodic SLA deadline checks
        reactor
            .register(Arc::new(TaskSLAEnforcementHandler::new(
                self.task_repo.clone(),
                p.sla_warning_threshold_pct,
                p.sla_critical_threshold_pct,
                p.sla_auto_escalate_on_breach,
            )))
            .await;

        // PriorityAgingHandler (LOW) — age task priorities based on wait time
        if p.priority_aging_enabled {
            reactor
                .register(Arc::new(PriorityAgingHandler::new(
                    self.task_repo.clone(),
                    p.priority_aging_low_to_normal_secs,
                    p.priority_aging_normal_to_high_secs,
                    p.priority_aging_high_to_critical_secs,
                )))
                .await;
        }

        // MemoryInformedDecompositionHandler (NORMAL) — trigger goal re-evaluation on semantic patterns
        if p.memory_informed_decomposition_enabled {
            reactor
                .register(Arc::new(MemoryInformedDecompositionHandler::new(
                    self.goal_repo.clone(),
                    p.memory_informed_cooldown_per_goal_secs,
                )))
                .await;
        }

        // MemoryConflictEscalationHandler (NORMAL) — escalate low-similarity memory conflicts
        reactor
            .register(Arc::new(MemoryConflictEscalationHandler::new()))
            .await;

        // TaskCompletionLearningHandler (NORMAL) — store learning patterns for retried tasks
        if p.task_learning_enabled {
            reactor
                .register(Arc::new(TaskCompletionLearningHandler::new(
                    command_bus.clone(),
                    p.task_learning_min_retries,
                    p.task_learning_store_efficiency,
                )))
                .await;
        }

        // GoalEvaluationTaskCreationHandler (NORMAL) — create diagnostic/remediation tasks
        if p.auto_create_diagnostic_tasks || p.auto_create_remediation_tasks {
            reactor
                .register(Arc::new(GoalEvaluationTaskCreationHandler::new(
                    command_bus.clone(),
                    p.auto_create_diagnostic_tasks,
                    p.max_diagnostic_tasks_per_goal,
                    p.auto_create_remediation_tasks,
                )))
                .await;
        }

        // GoalConvergenceCheckHandler (NORMAL) — periodic deep goal convergence evaluation
        if p.goal_convergence_check_enabled {
            reactor
                .register(Arc::new(GoalConvergenceCheckHandler::new(
                    self.goal_repo.clone(),
                    self.task_repo.clone(),
                    command_bus.clone(),
                )))
                .await;
        }

        // EvolutionTriggeredTemplateUpdateHandler (LOW) — submit refinement tasks for underperforming templates
        if self.config.track_evolution {
            reactor
                .register(Arc::new(EvolutionTriggeredTemplateUpdateHandler::new(
                    command_bus.clone(),
                )))
                .await;
        }

        // TaskScheduleHandler (NORMAL) — create tasks when periodic schedules fire
        if let Some(ref pool) = self.pool {
            use crate::adapters::sqlite::SqliteTaskScheduleRepository;
            use crate::services::task_schedule_service::TaskScheduleService;

            let schedule_repo = Arc::new(SqliteTaskScheduleRepository::new(pool.clone()));
            reactor
                .register(Arc::new(TaskScheduleHandler::new(
                    schedule_repo.clone(),
                    self.task_repo.clone(),
                    command_bus.clone(),
                )))
                .await;

            // Register all active task schedules with the EventScheduler
            let schedule_service = TaskScheduleService::new(schedule_repo);
            match schedule_service.register_active_schedules(&self.event_scheduler).await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!("Registered {} active task schedule(s) with EventScheduler", count);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to register active task schedules: {}", e);
                }
            }
        }

        // Adapter handlers — register only when an adapter registry is present
        if let Some(ref adapter_registry) = self.adapter_registry {
            // IngestionPollHandler (NORMAL) — poll external systems for new work items
            if !adapter_registry.ingestion_names().is_empty() {
                reactor
                    .register(Arc::new(IngestionPollHandler::new(
                        self.task_repo.clone(),
                        adapter_registry.clone(),
                        command_bus.clone(),
                    )))
                    .await;
            }

            // EgressRoutingHandler (NORMAL) — route task results to external systems
            if !adapter_registry.egress_names().is_empty() {
                reactor
                    .register(Arc::new(EgressRoutingHandler::new(
                        adapter_registry.clone(),
                    )))
                    .await;
            }

            // AdapterLifecycleSyncHandler (NORMAL) — sync task lifecycle back to external systems
            if !adapter_registry.egress_names().is_empty() {
                reactor
                    .register(Arc::new(AdapterLifecycleSyncHandler::new(
                        self.task_repo.clone(),
                        adapter_registry.clone(),
                    )))
                    .await;
            }
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

        // System stall check — detect idle swarm (no task activity)
        scheduler
            .register(interval_schedule(
                "system-stall-check",
                Duration::from_secs(p.system_stall_check_interval_secs),
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

        // Goal convergence check — periodic deep strategic evaluation
        if p.goal_convergence_check_enabled {
            scheduler
                .register(interval_schedule(
                    "goal-convergence-check",
                    Duration::from_secs(p.goal_convergence_check_interval_secs),
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

        // SLA check — periodic SLA deadline enforcement
        scheduler
            .register(interval_schedule(
                "sla-check",
                Duration::from_secs(p.sla_check_interval_secs),
                EventCategory::Scheduler,
                EventSeverity::Debug,
            ))
            .await;

        // Priority aging — periodic priority promotion for waiting tasks
        if p.priority_aging_enabled {
            scheduler
                .register(interval_schedule(
                    "priority-aging",
                    Duration::from_secs(p.priority_aging_interval_secs),
                    EventCategory::Scheduler,
                    EventSeverity::Debug,
                ))
                .await;
        }

        // Adapter ingestion poll — periodic external system ingestion
        if let Some(ref adapter_registry) = self.adapter_registry {
            if !adapter_registry.ingestion_names().is_empty() {
                scheduler
                    .register(interval_schedule(
                        "adapter-ingestion-poll",
                        Duration::from_secs(300),
                        EventCategory::Scheduler,
                        EventSeverity::Debug,
                    ))
                    .await;
            }
        }

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

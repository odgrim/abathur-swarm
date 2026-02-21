//! Swarm Orchestrator - the central coordinator for the Abathur system.
//!
//! The orchestrator is a thin coordinator over well-defined subsystems:
//!
//! - **types**: Public configuration, event, and status types
//! - **event_handling**: Human escalation, A2A messaging, event bus integration
//! - **agent_lifecycle**: Agent evolution, registration, prompts
//! - **goal_processing**: Task spawning, dependency management
//! - **specialist_triggers**: Failure recovery, restructuring, diagnostics, merge conflicts
//! - **infrastructure**: Cold start, decay daemon, MCP servers, stats, verification
//! - **helpers**: Utility functions for spawned tasks (auto-commit, post-completion)

pub mod types;
mod event_handling;
mod agent_lifecycle;
mod goal_processing;
mod handler_registration;
mod specialist_triggers;
mod infrastructure;
pub(crate) mod helpers;
pub(crate) mod convergent_execution;

// Re-export public types
pub use types::{
    ConvergenceLoopConfig, McpServerConfig, OrchestratorStatus, PollingConfig, SwarmConfig,
    SwarmEvent, SwarmStats, VerificationLevel,
};

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, HumanEscalationEvent};
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, NullMemoryRepository,
    Substrate, TaskRepository, TrajectoryRepository, WorktreeRepository,
};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    AuditLogConfig, AuditLogService,
    CircuitBreakerConfig, CircuitBreakerService,
    DaemonHandle, EvolutionLoop,
    IntentVerifierConfig, IntentVerifierService,
    command_bus::CommandBus,
    dag_restructure::DagRestructureService,
    event_reactor::EventReactor,
    event_scheduler::EventScheduler,
    guardrails::{Guardrails, GuardrailsConfig},
};

/// The main swarm orchestrator.
pub struct SwarmOrchestrator<G, T, W, A, M = NullMemoryRepository>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    // Repository layer
    pub(super) goal_repo: Arc<G>,
    pub(super) task_repo: Arc<T>,
    pub(super) worktree_repo: Arc<W>,
    pub(super) agent_repo: Arc<A>,
    pub(super) memory_repo: Option<Arc<M>>,
    pub(super) substrate: Arc<dyn Substrate>,

    // Configuration
    pub(super) config: SwarmConfig,

    // Runtime state
    pub(super) status: Arc<RwLock<OrchestratorStatus>>,
    pub(super) stats: Arc<RwLock<SwarmStats>>,
    pub(super) agent_semaphore: Arc<Semaphore>,
    pub(super) total_tokens: Arc<AtomicU64>,

    // Integrated services
    pub(super) audit_log: Arc<AuditLogService>,
    pub(super) circuit_breaker: Arc<CircuitBreakerService>,
    pub(super) decay_daemon_handle: Arc<RwLock<Option<DaemonHandle>>>,
    pub(super) hourly_reset_handle: Arc<RwLock<Option<crate::services::guardrails::HourlyResetHandle>>>,
    pub(super) evolution_loop: Arc<EvolutionLoop>,
    pub(super) active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    pub(super) restructure_service: Arc<tokio::sync::Mutex<DagRestructureService>>,
    pub(super) guardrails: Arc<Guardrails>,
    pub(super) mcp_shutdown_tx: Arc<RwLock<Option<tokio::sync::broadcast::Sender<()>>>>,
    pub(super) intent_verifier: Option<Arc<IntentVerifierService<G, T>>>,
    pub(super) overmind: Option<Arc<crate::services::OvermindService>>,
    pub(super) event_bus: Arc<crate::services::event_bus::EventBus>,
    pub(super) event_reactor: Arc<EventReactor>,
    pub(super) event_scheduler: Arc<EventScheduler>,
    pub(super) ready_task_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<uuid::Uuid>>>,
    pub(super) ready_task_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
    pub(super) specialist_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<uuid::Uuid>>>,
    pub(super) specialist_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
    pub(super) escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>,
    pub(super) federation_client: Option<Arc<crate::adapters::mcp::FederationClient>>,
    pub(super) trigger_rule_repo: Option<Arc<dyn crate::domain::ports::TriggerRuleRepository>>,
    pub(super) command_bus: Arc<RwLock<Option<Arc<CommandBus>>>>,
    /// Optional DB pool for services that need persistence (absence timers, command dedup).
    pub(super) pool: Option<sqlx::SqlitePool>,

    // -- Convergence infrastructure --

    /// Overseer cluster for convergent execution quality measurement.
    pub(super) overseer_cluster: Option<Arc<crate::services::overseers::OverseerClusterService>>,
    /// Trajectory repository for convergence state persistence.
    pub(super) trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    /// Pre-built convergence engine config derived from SwarmConfig.
    /// Stored here to avoid recomputing on every convergent task spawn.
    pub(super) convergence_engine_config: Option<crate::domain::models::convergence::ConvergenceEngineConfig>,

    // -- Phase orchestrator --

    /// Optional phase orchestrator for workflow-based execution.
    pub(super) phase_orchestrator: Option<Arc<crate::services::phase_orchestrator::PhaseOrchestrator<T, A, G>>>,

    /// Optional adapter registry for external system integration.
    pub(super) adapter_registry: Option<Arc<crate::services::adapter_registry::AdapterRegistry>>,

    /// Optional budget tracker for budget-aware scheduling.
    pub(super) budget_tracker: Option<Arc<crate::services::budget_tracker::BudgetTracker>>,
}

// ============================================================================
// Constructor & Builder Pattern
// ============================================================================

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        worktree_repo: Arc<W>,
        agent_repo: Arc<A>,
        substrate: Arc<dyn Substrate>,
        config: SwarmConfig,
        event_bus: Arc<crate::services::event_bus::EventBus>,
        event_reactor: Arc<EventReactor>,
        event_scheduler: Arc<EventScheduler>,
    ) -> Self {
        let max_agents = config.max_agents;
        let (ready_tx, ready_rx) = tokio::sync::mpsc::channel(256);
        let (specialist_tx, specialist_rx) = tokio::sync::mpsc::channel(64);
        Self {
            goal_repo,
            task_repo,
            worktree_repo,
            agent_repo,
            memory_repo: None,
            substrate,
            config,
            status: Arc::new(RwLock::new(OrchestratorStatus::Idle)),
            stats: Arc::new(RwLock::new(SwarmStats::default())),
            agent_semaphore: Arc::new(Semaphore::new(max_agents)),
            total_tokens: Arc::new(AtomicU64::new(0)),
            audit_log: Arc::new(AuditLogService::with_defaults()),
            circuit_breaker: Arc::new(CircuitBreakerService::with_defaults()),
            decay_daemon_handle: Arc::new(RwLock::new(None)),
            hourly_reset_handle: Arc::new(RwLock::new(None)),
            evolution_loop: Arc::new(EvolutionLoop::with_default_config()),
            active_goals_cache: Arc::new(RwLock::new(Vec::new())),
            restructure_service: Arc::new(tokio::sync::Mutex::new(DagRestructureService::with_defaults())),
            guardrails: Arc::new(Guardrails::with_defaults()),
            mcp_shutdown_tx: Arc::new(RwLock::new(None)),
            intent_verifier: None,
            overmind: None,
            event_bus,
            event_reactor,
            event_scheduler,
            escalation_store: Arc::new(RwLock::new(Vec::new())),
            federation_client: None,
            trigger_rule_repo: None,
            ready_task_rx: Arc::new(tokio::sync::Mutex::new(ready_rx)),
            ready_task_tx: ready_tx,
            specialist_rx: Arc::new(tokio::sync::Mutex::new(specialist_rx)),
            specialist_tx,
            command_bus: Arc::new(RwLock::new(None)),
            pool: None,
            overseer_cluster: None,
            trajectory_repo: None,
            convergence_engine_config: None,
            phase_orchestrator: None,
            adapter_registry: None,
            budget_tracker: None,
        }
    }

    // -- Builder methods --

    /// Create orchestrator with a federation client for cross-swarm task delegation.
    pub fn with_federation(mut self, federation_client: Arc<crate::adapters::mcp::FederationClient>) -> Self {
        self.federation_client = Some(federation_client);
        self
    }

    /// Create orchestrator with intent verification enabled.
    pub fn with_intent_verifier(mut self, substrate: Arc<dyn Substrate>) -> Self {
        let config = IntentVerifierConfig {
            max_turns: self.config.default_max_turns,
            convergence: crate::domain::models::ConvergenceConfig {
                max_iterations: self.config.convergence.max_iterations,
                min_confidence_threshold: self.config.convergence.min_confidence_threshold,
                require_full_satisfaction: self.config.convergence.require_full_satisfaction,
                auto_retry_partial: self.config.convergence.auto_retry_partial,
                convergence_timeout_secs: self.config.convergence.convergence_timeout_secs,
            },
            include_artifacts: true,
            include_task_output: true,
            verifier_agent_type: "intent-verifier".to_string(),
        };
        self.intent_verifier = Some(Arc::new(IntentVerifierService::new(
            self.goal_repo.clone(),
            self.task_repo.clone(),
            substrate,
            config,
        )));
        self
    }

    /// Create orchestrator with custom guardrails configuration.
    pub fn with_guardrails(mut self, config: GuardrailsConfig) -> Self {
        self.guardrails = Arc::new(Guardrails::new(config));
        self
    }

    /// Create orchestrator with a trigger rule repository for persisting fire state.
    pub fn with_trigger_rule_repo(mut self, repo: Arc<dyn crate::domain::ports::TriggerRuleRepository>) -> Self {
        self.trigger_rule_repo = Some(repo);
        self
    }

    /// Create orchestrator with memory repository for cold start and decay daemon.
    pub fn with_memory_repo(mut self, memory_repo: Arc<M>) -> Self {
        self.memory_repo = Some(memory_repo);
        self
    }

    /// Create orchestrator with custom audit log configuration.
    pub fn with_audit_log(mut self, config: AuditLogConfig) -> Self {
        self.audit_log = Arc::new(AuditLogService::new(config));
        self
    }

    /// Create orchestrator with custom circuit breaker configuration.
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = Arc::new(CircuitBreakerService::new(config));
        self
    }

    /// Provide a DB pool for services that need persistence (absence timers, command dedup,
    /// and evolution loop refinement requests).
    pub fn with_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        use crate::adapters::sqlite::SqliteRefinementRepository;
        use crate::services::evolution_loop::EvolutionConfig;

        let refinement_repo = Arc::new(SqliteRefinementRepository::new(pool.clone()));
        self.evolution_loop = Arc::new(
            EvolutionLoop::new(EvolutionConfig::default()).with_repo(refinement_repo),
        );
        self.pool = Some(pool);
        self
    }

    /// Create orchestrator with an overseer cluster for convergent execution.
    ///
    /// The overseer cluster provides quality measurement (compilation, type checking,
    /// linting, testing, etc.) used by the convergence engine during iterative
    /// convergent execution.
    pub fn with_overseer_cluster(
        mut self,
        cluster: Arc<crate::services::overseers::OverseerClusterService>,
    ) -> Self {
        self.overseer_cluster = Some(cluster);
        self
    }

    /// Create orchestrator with a trajectory repository for convergence state persistence.
    ///
    /// Required for convergent execution. Without this, convergent tasks will
    /// fall back to direct execution with a warning.
    pub fn with_trajectory_repo(mut self, repo: Arc<dyn TrajectoryRepository>) -> Self {
        self.trajectory_repo = Some(repo);
        self
    }

    /// Create orchestrator with convergence engine configuration.
    ///
    /// If not set explicitly, the config is derived from `SwarmConfig` when the
    /// first convergent task is spawned.
    pub fn with_convergence_engine_config(
        mut self,
        config: crate::domain::models::convergence::ConvergenceEngineConfig,
    ) -> Self {
        self.convergence_engine_config = Some(config);
        self
    }

    /// Create orchestrator with Overmind for strategic decision-making.
    ///
    /// The Overmind is an Architect-tier agent that provides intelligent decisions
    /// for goal decomposition, conflict resolution, stuck state recovery, and
    /// escalation evaluation.
    pub fn with_overmind(mut self, overmind: Arc<crate::services::OvermindService>) -> Self {
        // Propagate Overmind to restructure service for LLM-powered recovery
        let overmind_clone = overmind.clone();
        if let Ok(mut svc) = self.restructure_service.try_lock() {
            svc.set_overmind(overmind_clone);
        }
        self.overmind = Some(overmind);
        self
    }

    /// Create orchestrator with a phase orchestrator for workflow-based execution.
    pub fn with_phase_orchestrator(
        mut self,
        phase_orchestrator: Arc<crate::services::phase_orchestrator::PhaseOrchestrator<T, A, G>>,
    ) -> Self {
        self.phase_orchestrator = Some(phase_orchestrator);
        self
    }

    /// Create orchestrator with an adapter registry for external system integration.
    ///
    /// The adapter registry provides ingestion (pull work in) and egress (push results
    /// out) capabilities via external system connectors.
    pub fn with_adapter_registry(
        mut self,
        registry: Arc<crate::services::adapter_registry::AdapterRegistry>,
    ) -> Self {
        self.adapter_registry = Some(registry);
        self
    }

    /// Attach a budget tracker for budget-aware scheduling.
    ///
    /// When present, the orchestrator will gate task dispatching based on
    /// the current budget pressure level and emit budget events through the
    /// event bus.
    pub fn with_budget_tracker(
        mut self,
        tracker: Arc<crate::services::budget_tracker::BudgetTracker>,
    ) -> Self {
        self.budget_tracker = Some(tracker);
        self
    }

    // -- Service Accessors --

    /// Get the Overmind service if configured.
    pub fn overmind(&self) -> Option<&Arc<crate::services::OvermindService>> {
        self.overmind.as_ref()
    }

    /// Get the guardrails service for external use.
    pub fn guardrails(&self) -> &Arc<Guardrails> {
        &self.guardrails
    }

    /// Get the audit log service for external use.
    pub fn audit_log(&self) -> &Arc<AuditLogService> {
        &self.audit_log
    }

    /// Get the circuit breaker service for external use.
    pub fn circuit_breaker(&self) -> &Arc<CircuitBreakerService> {
        &self.circuit_breaker
    }

    /// Get the evolution loop service for external use.
    pub fn evolution_loop(&self) -> &Arc<EvolutionLoop> {
        &self.evolution_loop
    }

    /// Get the phase orchestrator if configured.
    pub fn phase_orchestrator(&self) -> Option<&Arc<crate::services::phase_orchestrator::PhaseOrchestrator<T, A, G>>> {
        self.phase_orchestrator.as_ref()
    }

    /// Get the command bus (available after `register_builtin_handlers()`).
    #[allow(dead_code)]
    pub(crate) async fn command_bus(&self) -> Arc<CommandBus> {
        self.command_bus.read().await
            .as_ref()
            .expect("command_bus not initialized — call register_builtin_handlers() first")
            .clone()
    }

    // ========================================================================
    // Main Orchestration Loop
    // ========================================================================

    /// Start the orchestrator and run the main loop.
    pub async fn run(&self, event_tx: mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        {
            let mut status = self.status.write().await;
            *status = OrchestratorStatus::Running;
        }
        let _ = event_tx.send(SwarmEvent::Started).await;
        self.event_bus.publish(crate::services::event_factory::orchestrator_event(
            crate::services::event_bus::EventSeverity::Info,
            crate::services::event_bus::EventPayload::OrchestratorStarted,
        )).await;

        // Spawn bridge: forward EventBus events to legacy event_tx channel for TUI/logging
        {
            let mut bus_rx = self.event_bus.subscribe();
            let bridge_tx = event_tx.clone();
            tokio::spawn(async move {
                loop {
                    match bus_rx.recv().await {
                        Ok(unified_event) => {
                            if let Some(swarm_event) = SwarmEvent::from_event_payload(&unified_event.payload) {
                                let _ = bridge_tx.send(swarm_event).await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("EventBus→SwarmEvent bridge lagged by {} events", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            });
        }

        // Log swarm startup
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            format!("Swarm orchestrator started with max {} agents", self.config.max_agents),
        ).await;

        // Run cold start if memory is empty (populates initial project context)
        if self.memory_repo.is_some() {
            match self.cold_start().await {
                Ok(Some(report)) => {
                    self.audit_log.info(
                        AuditCategory::Memory,
                        AuditAction::MemoryStored,
                        format!(
                            "Cold start completed: {} memories created, project type: {}",
                            report.memories_created, report.project_type
                        ),
                    ).await;
                }
                Ok(None) => {
                    // Memory already populated, skip cold start
                }
                Err(e) => {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::System,
                            AuditAction::SwarmStarted,
                            AuditActor::System,
                            format!("Cold start failed (non-fatal): {}", e),
                        ),
                    ).await;
                }
            }
        }

        // Start memory decay daemon if memory repo is available
        if self.memory_repo.is_some() {
            if let Err(e) = self.start_decay_daemon().await {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        AuditActor::System,
                        format!("Failed to start decay daemon (non-fatal): {}", e),
                    ),
                ).await;
            }
        }

        // Start hourly token reset daemon (always runs — no memory repo dependency)
        self.start_hourly_reset_daemon().await;

        // Refresh active goals cache for agent context
        if let Err(e) = self.refresh_active_goals_cache().await {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    AuditActor::System,
                    format!("Failed to cache active goals: {}", e),
                ),
            ).await;
        }

        // Seed baseline agent templates (DB is sole source, hardcoded as bootstrap)
        {
            use crate::services::AgentService;
            let agent_service = AgentService::new(self.agent_repo.clone(), self.event_bus.clone());
            // Use routing-aware seeding when all_workflows is populated; otherwise fall
            // back to single-workflow seeding (legacy / empty config path).
            let seed_result = if !self.config.all_workflows.is_empty() {
                agent_service.seed_baseline_agents_with_workflows(&self.config.all_workflows).await
            } else {
                agent_service.seed_baseline_agents_with_workflow(self.config.workflow_template.as_ref()).await
            };
            match seed_result {
                Ok(seeded) if !seeded.is_empty() => {
                    self.audit_log.info(
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        format!("Seeded {} baseline agent templates: {}", seeded.len(), seeded.join(", ")),
                    ).await;
                }
                Ok(_) => {
                    // All agents already exist
                }
                Err(e) => {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!("Failed to seed agent templates (non-fatal): {}", e),
                        ),
                    ).await;
                }
            }
        }

        // Register existing agent templates with A2A gateway for discovery
        if self.config.mcp_servers.a2a_gateway.is_some() {
            if let Err(e) = self.register_all_agent_templates().await {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        AuditActor::System,
                        format!("Failed to register agent templates with A2A gateway: {}", e),
                    ),
                ).await;
            }
        }

        // Wait for MCP servers to become healthy before entering the main loop.
        // If servers never come up, abort startup rather than spawning agents
        // into an environment where they can't reach the orchestration APIs.
        if let Err(e) = self.await_mcp_readiness().await {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Error,
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    AuditActor::System,
                    format!("Aborting orchestrator: {}", e),
                ),
            ).await;
            let _ = event_tx.send(SwarmEvent::Stopped).await;
            self.event_bus.publish(crate::services::event_factory::orchestrator_event(
                crate::services::event_bus::EventSeverity::Info,
                crate::services::event_bus::EventPayload::OrchestratorStopped,
            )).await;
            return Err(e);
        }

        // Initialize EventBus sequence from store to prevent overlap after restart
        self.event_bus.initialize_sequence_from_store().await;

        // Register built-in event handlers and schedules BEFORE starting
        // the reactor, so handlers are ready when it begins subscribing.
        self.register_builtin_handlers().await;
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            "Registered built-in event handlers",
        ).await;

        // Load persisted circuit breaker states (after handler registration)
        self.event_reactor.load_circuit_breaker_states().await;

        self.register_builtin_schedules().await;
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            "Registered built-in scheduled events",
        ).await;

        // Start EventReactor (handlers are already registered)
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            "Starting EventReactor for reactive event handling",
        ).await;
        let reactor_handle = self.event_reactor.start();

        // Load persistent scheduler state from DB before starting
        self.event_scheduler.initialize_from_store().await;

        // Start EventScheduler
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            "Starting EventScheduler for time-based events",
        ).await;
        let scheduler_handle = self.event_scheduler.start();

        // Replay missed events from the event store
        match self.event_reactor.replay_missed_events().await {
            Ok(count) if count > 0 => {
                self.audit_log.info(
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    format!("Replayed {} missed events from event store", count),
                ).await;
            }
            Ok(_) => {}
            Err(e) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        AuditActor::System,
                        format!("Failed to replay missed events (non-fatal): {}", e),
                    ),
                ).await;
            }
        }

        // Run startup reconciliation to fix inconsistent state
        match self.run_startup_reconciliation().await {
            Ok(count) if count > 0 => {
                self.audit_log.info(
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    format!("Startup reconciliation: {} corrections applied", count),
                ).await;
            }
            Ok(_) => {}
            Err(e) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        AuditActor::System,
                        format!("Startup reconciliation failed (non-fatal): {}", e),
                    ),
                ).await;
            }
        }

        // The main loop runs at reconciliation cadence as a safety net.
        // Handlers do the fast-path work (task cascades, retries, stats).
        // The loop handles draining the ready-task channel and spawning agents.
        let reconciliation_secs = self.config.reconciliation_interval_secs.unwrap_or(30);
        let loop_interval = tokio::time::Duration::from_secs(reconciliation_secs);

        // Main orchestration loop
        loop {
            let current_status = self.status.read().await.clone();

            match current_status {
                OrchestratorStatus::ShuttingDown | OrchestratorStatus::Stopped => {
                    break;
                }
                OrchestratorStatus::Paused => {
                    tokio::time::sleep(loop_interval).await;
                    continue;
                }
                _ => {}
            }

            // Event-driven mode: handlers manage task cascades, retries,
            // stats, escalation checks, and reconciliation via scheduled
            // events. The loop drains the ready-task channel and handles
            // operations that require full orchestrator context.

            // Drain ready-task channel and spawn agents
            self.drain_ready_tasks(&event_tx).await?;

            // Drain specialist channel and process specialist triggers
            self.drain_specialist_tasks(&event_tx).await?;

            if self.config.track_evolution {
                self.process_evolution_refinements(&event_tx).await?;
            }

            // Wait before next iteration
            tokio::time::sleep(loop_interval).await;
        }

        // Flush pending watermarks before stopping the reactor
        self.event_reactor.flush_watermarks().await;

        // Stop EventReactor
        self.event_reactor.stop();
        reactor_handle.abort();

        // Stop EventScheduler
        self.event_scheduler.stop();
        scheduler_handle.abort();

        // Log swarm shutdown
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStopped,
            "Swarm orchestrator stopped",
        ).await;

        // Stop decay daemon if running
        self.stop_decay_daemon().await;

        // Stop hourly token reset daemon if running
        self.stop_hourly_reset_daemon().await;

        // Stop embedded MCP servers if running
        self.stop_embedded_mcp_servers().await;

        let _ = event_tx.send(SwarmEvent::Stopped).await;
        self.event_bus.publish(crate::services::event_factory::orchestrator_event(
            crate::services::event_bus::EventSeverity::Info,
            crate::services::event_bus::EventPayload::OrchestratorStopped,
        )).await;
        Ok(())
    }

    /// Run a single iteration of the orchestration loop.
    pub async fn tick(&self) -> DomainResult<SwarmStats> {
        let (tx, _rx) = mpsc::channel(100);

        // Drain ready-task channel and spawn agents
        self.drain_ready_tasks(&tx).await?;

        // Update stats
        self.update_stats(&tx).await?;

        Ok(self.stats().await)
    }

    /// Drain the ready-task channel and spawn agents for each ready task.
    async fn drain_ready_tasks(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let mut rx = self.ready_task_rx.lock().await;
        let mut spawned_ids = std::collections::HashSet::new();

        while let Ok(task_id) = rx.try_recv() {
            // Fetch and validate task is still Ready
            if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                if task.status == crate::domain::models::TaskStatus::Ready {
                    self.spawn_task_agent(&task, event_tx).await?;
                    spawned_ids.insert(task_id);
                }
            }
        }

        // Also pick up any ready tasks not yet signaled via the channel
        // (e.g., tasks that became ready before the handler was registered)
        if spawned_ids.is_empty() {
            self.process_goals(event_tx).await?;
        } else {
            // Run process_goals but skip tasks already attempted in this drain cycle
            self.process_goals_excluding(event_tx, &spawned_ids).await?;
        }

        Ok(())
    }

    /// Drain the specialist channel and trigger specialist processing.
    async fn drain_specialist_tasks(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let mut rx = self.specialist_rx.lock().await;

        while let Ok(task_id) = rx.try_recv() {
            // Validate task is still in a state that warrants specialist attention
            if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                if task.status == crate::domain::models::TaskStatus::Failed {
                    // Delegate to existing specialist processing
                    self.process_specialist_triggers(event_tx).await?;
                    break; // process_specialist_triggers handles all pending specialists
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository,
        SqliteTaskRepository, SqliteWorktreeRepository,
    };
    use crate::adapters::substrates::MockSubstrate;

    async fn setup_orchestrator() -> SwarmOrchestrator<
        SqliteGoalRepository,
        SqliteTaskRepository,
        SqliteWorktreeRepository,
        SqliteAgentRepository,
        SqliteMemoryRepository,
    > {
        use crate::services::event_bus::{EventBus, EventBusConfig};
        use crate::services::event_reactor::{EventReactor, ReactorConfig};
        use crate::services::event_scheduler::{EventScheduler, SchedulerConfig};

        let pool = create_migrated_test_pool().await.unwrap();

        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
        let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool));
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let mut config = SwarmConfig::default();
        config.use_worktrees = false; // Disable worktrees for tests

        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let event_reactor = Arc::new(EventReactor::new(event_bus.clone(), ReactorConfig::default()));
        let event_scheduler = Arc::new(EventScheduler::new(event_bus.clone(), SchedulerConfig::default()));

        SwarmOrchestrator::new(goal_repo, task_repo, worktree_repo, agent_repo, substrate, config, event_bus, event_reactor, event_scheduler)
            .with_memory_repo(memory_repo)
    }

    #[tokio::test]
    async fn test_orchestrator_creation() {
        let orchestrator = setup_orchestrator().await;
        assert_eq!(orchestrator.status().await, OrchestratorStatus::Idle);
    }

    #[tokio::test]
    async fn test_orchestrator_pause_resume() {
        let orchestrator = setup_orchestrator().await;

        // Can't pause from idle
        orchestrator.pause().await;
        assert_eq!(orchestrator.status().await, OrchestratorStatus::Idle);
    }

    #[tokio::test]
    async fn test_tick_empty() {
        let orchestrator = setup_orchestrator().await;
        let stats = orchestrator.tick().await.unwrap();
        assert_eq!(stats.active_goals, 0);
        assert_eq!(stats.pending_tasks, 0);
    }

    #[tokio::test]
    async fn test_token_tracking() {
        let orchestrator = setup_orchestrator().await;
        assert_eq!(orchestrator.total_tokens(), 0);
    }
}

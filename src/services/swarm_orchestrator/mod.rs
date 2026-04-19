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

mod agent_lifecycle;
pub(crate) mod convergent_execution;
mod event_handling;
mod goal_processing;
mod handler_registration;
pub(crate) mod helpers;
mod infrastructure;
pub mod middleware;
mod specialist_triggers;
pub mod types;

// Re-export public types
pub use types::{
    ConvergenceLoopConfig, McpServerConfig, OrchestratorStatus, PollingConfig, SwarmConfig,
    SwarmEvent, SwarmStats, VerificationLevel,
};

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::{RwLock, Semaphore, mpsc};

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, HumanEscalationEvent};
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, NullMemoryRepository, Substrate,
    TaskRepository, TrajectoryRepository, WorktreeRepository,
};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, AuditLogConfig,
    AuditLogService, CircuitBreakerConfig, CircuitBreakerService, DaemonHandle, EvolutionLoop,
    IntentVerifierConfig, IntentVerifierService,
    command_bus::CommandBus,
    dag_restructure::DagRestructureService,
    event_reactor::EventReactor,
    event_scheduler::EventScheduler,
    guardrails::{Guardrails, GuardrailsConfig},
    supervise,
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
    // ---------------- Core dependencies (required) ----------------
    pub(super) goal_repo: Arc<G>,
    pub(super) task_repo: Arc<T>,
    pub(super) worktree_repo: Arc<W>,
    pub(super) agent_repo: Arc<A>,
    pub(super) substrate: Arc<dyn Substrate>,
    pub(super) config: SwarmConfig,

    // ---------------- Runtime state ----------------
    pub(super) status: Arc<RwLock<OrchestratorStatus>>,
    pub(super) stats: Arc<RwLock<SwarmStats>>,
    pub(super) agent_semaphore: Arc<Semaphore>,
    pub(super) total_tokens: Arc<AtomicU64>,
    pub(super) active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    pub(super) escalation_store: Arc<RwLock<std::collections::HashMap<uuid::Uuid, HumanEscalationEvent>>>,

    // ---------------- Core services (always present) ----------------
    pub(super) audit_log: Arc<AuditLogService>,
    pub(super) circuit_breaker: Arc<CircuitBreakerService>,
    pub(super) evolution_loop: Arc<EvolutionLoop>,
    pub(super) restructure_service: Arc<tokio::sync::Mutex<DagRestructureService>>,
    pub(super) guardrails: Arc<Guardrails>,

    // ---------------- Event plumbing ----------------
    pub(super) event_bus: Arc<crate::services::event_bus::EventBus>,
    pub(super) event_reactor: Arc<EventReactor>,
    pub(super) event_scheduler: Arc<EventScheduler>,
    pub(super) ready_task_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<uuid::Uuid>>>,
    pub(super) ready_task_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,
    pub(super) specialist_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<uuid::Uuid>>>,
    pub(super) specialist_tx: tokio::sync::mpsc::Sender<uuid::Uuid>,

    // ---------------- Daemon handles ----------------
    pub(super) decay_daemon_handle: Arc<RwLock<Option<DaemonHandle>>>,
    /// Cancellation token for the hourly token-counter reset daemon.
    pub(super) hourly_reset_cancel: Arc<RwLock<Option<tokio_util::sync::CancellationToken>>>,
    pub(super) mcp_shutdown_tx: Arc<RwLock<Option<tokio::sync::broadcast::Sender<()>>>>,
    /// Optional handle to the outbox poller daemon.
    pub(super) outbox_poller_handle:
        Arc<RwLock<Option<crate::services::outbox_poller::OutboxPollerHandle>>>,
    /// Optional handle to the convergence poller daemon (federation).
    pub(super) convergence_poller_handle:
        Arc<RwLock<Option<crate::services::federation::ConvergencePollerHandle>>>,
    /// Optional handle to the convergence publisher daemon (federation).
    pub(super) convergence_publisher_handle: Arc<
        RwLock<
            Option<crate::services::federation::convergence_publisher::ConvergencePublisherHandle>,
        >,
    >,

    // ---------------- Optional services (progressive enhancement) ----------------
    pub(super) memory_repo: Option<Arc<M>>,
    pub(super) intent_verifier: Option<Arc<IntentVerifierService<G, T>>>,
    pub(super) overmind: Option<Arc<crate::services::OvermindService>>,
    pub(super) command_bus: Arc<RwLock<Option<Arc<CommandBus>>>>,
    /// Optional DB pool for services that need persistence (absence timers, command dedup).
    pub(super) pool: Option<sqlx::SqlitePool>,
    /// Optional outbox repository for transactional event delivery.
    pub(super) outbox_repo: Option<Arc<dyn crate::domain::ports::OutboxRepository>>,
    pub(super) trigger_rule_repo: Option<Arc<dyn crate::domain::ports::TriggerRuleRepository>>,
    pub(super) merge_request_repo: Option<Arc<dyn crate::domain::ports::MergeRequestRepository>>,
    /// Optional adapter registry for external system integration.
    pub(super) adapter_registry: Option<Arc<crate::services::adapter_registry::AdapterRegistry>>,
    /// Optional budget tracker for budget-aware scheduling.
    pub(super) budget_tracker: Option<Arc<crate::services::budget_tracker::BudgetTracker>>,
    /// Optional cost-window service for quiet-hours scheduling.
    pub(super) cost_window_service:
        Option<Arc<crate::services::cost_window_service::CostWindowService>>,

    // ---------------- Federation ----------------
    pub(super) federation_client: Option<Arc<crate::adapters::mcp::FederationClient>>,
    pub(super) federation_service: Option<Arc<crate::services::federation::FederationService>>,

    // ---------------- Convergence infrastructure ----------------
    /// Overseer cluster for convergent execution quality measurement.
    pub(super) overseer_cluster: Option<Arc<crate::services::overseers::OverseerClusterService>>,
    /// Trajectory repository for convergence state persistence.
    pub(super) trajectory_repo: Option<Arc<dyn TrajectoryRepository>>,
    /// Pre-built convergence engine config derived from SwarmConfig.
    /// Stored here to avoid recomputing on every convergent task spawn.
    pub(super) convergence_engine_config:
        Option<crate::domain::models::convergence::ConvergenceEngineConfig>,

    // ---------------- Middleware ----------------
    /// Pre-spawn middleware chain. Runs before substrate invocation for each
    /// ready task; can short-circuit or enrich the spawn context. Populated
    /// with the built-in middleware at construction; external callers may
    /// register additional middleware via
    /// [`SwarmOrchestrator::with_pre_spawn_middleware`].
    pub(super) pre_spawn_chain: Arc<RwLock<middleware::PreSpawnChain>>,

    /// Post-completion middleware chain. Runs the side-effects (verification,
    /// feature-branch handling, PR creation, merge-queue) that previously
    /// lived inline in `run_post_completion_workflow`. Registrable via
    /// [`SwarmOrchestrator::with_post_completion_middleware`].
    pub(super) post_completion_chain: Arc<RwLock<middleware::PostCompletionChain>>,
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
    #[allow(clippy::too_many_arguments)]
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
            // ---------------- Core dependencies (required) ----------------
            goal_repo,
            task_repo,
            worktree_repo,
            agent_repo,
            substrate,
            config,

            // ---------------- Runtime state ----------------
            status: Arc::new(RwLock::new(OrchestratorStatus::Idle)),
            stats: Arc::new(RwLock::new(SwarmStats::default())),
            agent_semaphore: Arc::new(Semaphore::new(max_agents)),
            total_tokens: Arc::new(AtomicU64::new(0)),
            active_goals_cache: Arc::new(RwLock::new(Vec::new())),
            escalation_store: Arc::new(RwLock::new(std::collections::HashMap::new())),

            // ---------------- Core services (always present) ----------------
            audit_log: Arc::new(AuditLogService::with_defaults()),
            circuit_breaker: Arc::new(CircuitBreakerService::with_defaults()),
            evolution_loop: Arc::new(EvolutionLoop::with_default_config()),
            restructure_service: Arc::new(tokio::sync::Mutex::new(
                DagRestructureService::with_defaults(),
            )),
            guardrails: Arc::new(Guardrails::with_defaults()),

            // ---------------- Event plumbing ----------------
            event_bus,
            event_reactor,
            event_scheduler,
            ready_task_rx: Arc::new(tokio::sync::Mutex::new(ready_rx)),
            ready_task_tx: ready_tx,
            specialist_rx: Arc::new(tokio::sync::Mutex::new(specialist_rx)),
            specialist_tx,

            // ---------------- Daemon handles ----------------
            decay_daemon_handle: Arc::new(RwLock::new(None)),
            hourly_reset_cancel: Arc::new(RwLock::new(None)),
            mcp_shutdown_tx: Arc::new(RwLock::new(None)),
            outbox_poller_handle: Arc::new(RwLock::new(None)),
            convergence_poller_handle: Arc::new(RwLock::new(None)),
            convergence_publisher_handle: Arc::new(RwLock::new(None)),

            // ---------------- Optional services (progressive enhancement) ----------------
            memory_repo: None,
            intent_verifier: None,
            overmind: None,
            command_bus: Arc::new(RwLock::new(None)),
            pool: None,
            outbox_repo: None,
            trigger_rule_repo: None,
            merge_request_repo: None,
            adapter_registry: None,
            budget_tracker: None,
            cost_window_service: None,

            // ---------------- Federation ----------------
            federation_client: None,
            federation_service: None,

            // ---------------- Convergence infrastructure ----------------
            overseer_cluster: None,
            trajectory_repo: None,
            convergence_engine_config: None,

            // ---------------- Middleware ----------------
            pre_spawn_chain: Arc::new(RwLock::new(middleware::PreSpawnChain::new())),
            post_completion_chain: Arc::new(RwLock::new(middleware::PostCompletionChain::new())),
        }
    }

    /// Register an additional pre-spawn middleware. Registration order is
    /// preserved; middleware runs in the order it was registered.
    pub async fn with_pre_spawn_middleware(
        self,
        mw: Arc<dyn middleware::PreSpawnMiddleware>,
    ) -> Self {
        self.pre_spawn_chain.write().await.register(mw);
        self
    }

    /// Register an additional post-completion middleware. Registration order
    /// is preserved.
    pub async fn with_post_completion_middleware(
        self,
        mw: Arc<dyn middleware::PostCompletionMiddleware>,
    ) -> Self {
        self.post_completion_chain.write().await.register(mw);
        self
    }

    // -- Builder methods --

    /// Create orchestrator with a federation client for cross-swarm task delegation.
    pub fn with_federation(
        mut self,
        federation_client: Arc<crate::adapters::mcp::FederationClient>,
    ) -> Self {
        self.federation_client = Some(federation_client);
        self
    }

    /// Create orchestrator with a federation service for hierarchical swarm delegation.
    pub fn with_federation_service(
        mut self,
        federation_service: Arc<crate::services::federation::FederationService>,
    ) -> Self {
        self.federation_service = Some(federation_service);
        self
    }

    /// Get a reference to the federation service, if configured.
    pub fn federation_service(
        &self,
    ) -> Option<&Arc<crate::services::federation::FederationService>> {
        self.federation_service.as_ref()
    }

    /// Set the federation delegation strategy (pass-through to FederationService).
    ///
    /// Only takes effect if a federation service is configured. If not, the strategy
    /// is stored and applied when `with_federation_service` is called later.
    pub fn with_delegation_strategy(
        self,
        strategy: Arc<dyn crate::services::federation::traits::FederationDelegationStrategy>,
    ) -> Self {
        if let Some(ref svc) = self.federation_service {
            // Can't mutate Arc contents directly; log a warning.
            // Strategies should be set on FederationService before passing to orchestrator.
            tracing::warn!(
                "Delegation strategy should be set on FederationService before calling with_federation_service. \
                 Current service has {} cerebrates.",
                svc.config().cerebrates.len()
            );
            let _ = strategy; // consumed
        }
        self
    }

    /// Set the federation result processor (pass-through to FederationService).
    pub fn with_result_processor(
        self,
        processor: Arc<dyn crate::services::federation::traits::FederationResultProcessor>,
    ) -> Self {
        if let Some(ref svc) = self.federation_service {
            tracing::warn!(
                "Result processor should be set on FederationService before calling with_federation_service. \
                 Current service has {} cerebrates.",
                svc.config().cerebrates.len()
            );
            let _ = processor;
        }
        self
    }

    /// Set the federation task transformer (pass-through to FederationService).
    pub fn with_task_transformer(
        self,
        transformer: Arc<dyn crate::services::federation::traits::FederationTaskTransformer>,
    ) -> Self {
        if let Some(ref svc) = self.federation_service {
            tracing::warn!(
                "Task transformer should be set on FederationService before calling with_federation_service. \
                 Current service has {} cerebrates.",
                svc.config().cerebrates.len()
            );
            let _ = transformer;
        }
        self
    }

    /// Register a result schema with the federation service.
    pub async fn register_result_schema(
        &self,
        schema: Arc<dyn crate::services::federation::traits::ResultSchema>,
    ) {
        if let Some(ref svc) = self.federation_service {
            svc.register_result_schema(schema).await;
        } else {
            tracing::warn!("Cannot register result schema: no federation service configured");
        }
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
    pub fn with_trigger_rule_repo(
        mut self,
        repo: Arc<dyn crate::domain::ports::TriggerRuleRepository>,
    ) -> Self {
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
    /// evolution loop refinement requests, and event outbox).
    pub fn with_pool(mut self, pool: sqlx::SqlitePool) -> Self {
        use crate::adapters::sqlite::{
            SqliteMergeRequestRepository, SqliteOutboxRepository, SqliteRefinementRepository,
        };
        use crate::services::evolution_loop::EvolutionConfig;

        let refinement_repo = Arc::new(SqliteRefinementRepository::new(pool.clone()));
        self.evolution_loop = Arc::new(
            EvolutionLoop::new(EvolutionConfig::default())
                .with_repo(refinement_repo)
                .with_agent_repo(self.agent_repo.clone()),
        );
        self.outbox_repo = Some(Arc::new(SqliteOutboxRepository::new(pool.clone())));
        self.merge_request_repo = Some(Arc::new(SqliteMergeRequestRepository::new(pool.clone())));
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
    pub fn with_cost_window_service(
        mut self,
        service: Arc<crate::services::cost_window_service::CostWindowService>,
    ) -> Self {
        self.cost_window_service = Some(service);
        self
    }

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

    // ========================================================================
    // Dependency Validation
    // ========================================================================

    /// Validate that every feature gated by `SwarmConfig` has its runtime
    /// dependencies wired up.
    ///
    /// The orchestrator has many optional services that are toggled on/off via
    /// builder methods (`with_trajectory_repo`, `with_overseer_cluster`, etc).
    /// Several of those are required for features that are enabled by default
    /// in `SwarmConfig`. Without this check, missing a `with_xxx(...)` call
    /// silently degrades the feature to a no-op at runtime: convergent tasks
    /// fall back to direct execution, merge-queue posts go nowhere, etc.
    ///
    /// This method is called at the top of [`run`] so misconfigurations fail
    /// loudly at startup instead of silently at the first task that hits the
    /// missing dependency.
    ///
    /// Only dependencies that have a clear config→dependency relationship are
    /// checked here. "Progressive enhancement" services that are simply used
    /// when present (budget tracker, adapter registry, overmind, pool, outbox,
    /// command bus, trigger rule repo) are intentionally not validated.
    pub fn validate_dependencies(&self) -> DomainResult<()> {
        use crate::domain::errors::DomainError;

        // Convergent execution: if convergence is enabled globally, the
        // convergence engine needs trajectory storage, an overseer cluster
        // for quality measurement, an intent verifier, and a memory repo.
        // Without all four, convergent tasks silently fall back to direct
        // execution (see goal_processing::spawn_task_agent).
        if self.config.convergence_enabled {
            if self.trajectory_repo.is_none() {
                return Err(DomainError::ValidationFailed(
                    "convergence_enabled=true but no trajectory repository wired. \
                     Call .with_trajectory_repo(...) before run(), or set convergence_enabled=false."
                        .into(),
                ));
            }
            if self.overseer_cluster.is_none() {
                return Err(DomainError::ValidationFailed(
                    "convergence_enabled=true but no overseer cluster wired. \
                     Call .with_overseer_cluster(...) before run(), or set convergence_enabled=false."
                        .into(),
                ));
            }
            if self.intent_verifier.is_none() {
                return Err(DomainError::ValidationFailed(
                    "convergence_enabled=true but no intent verifier wired. \
                     Call .with_intent_verifier(...) before run(), or set convergence_enabled=false."
                        .into(),
                ));
            }
            if self.memory_repo.is_none() {
                return Err(DomainError::ValidationFailed(
                    "convergence_enabled=true but no memory repository wired. \
                     Call .with_memory_repo(...) before run(), or set convergence_enabled=false."
                        .into(),
                ));
            }
        }

        // Intent verification toggle: if this flag is set, we need the
        // verifier service that runs it.
        if self.config.enable_intent_verification && self.intent_verifier.is_none() {
            return Err(DomainError::ValidationFailed(
                "enable_intent_verification=true but no intent verifier wired. \
                 Call .with_intent_verifier(...) before run(), or set enable_intent_verification=false."
                    .into(),
            ));
        }

        // Merge queue: if the config says to route completions through the
        // two-stage merge queue, the merge-request repo must be present.
        // Without it, merge_queue middleware and conflict-specialist triggers
        // all silently no-op.
        if self.config.use_merge_queue && self.merge_request_repo.is_none() {
            return Err(DomainError::ValidationFailed(
                "use_merge_queue=true but no merge request repository wired. \
                 Call .with_pool(...) (which wires the SQLite backed repo) before run(), \
                 or set use_merge_queue=false."
                    .into(),
            ));
        }

        Ok(())
    }

    // ========================================================================
    // Main Orchestration Loop
    // ========================================================================

    /// Start the orchestrator and run the main loop.
    pub async fn run(&self, event_tx: mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Fail-fast on misconfiguration: if a feature toggle is on but its
        // runtime dependency is missing, refuse to start rather than letting
        // the feature silently no-op at runtime. Runs before any status
        // mutation or event publication so a failure leaves the orchestrator
        // in the Idle state.
        self.validate_dependencies()?;

        {
            let mut status = self.status.write().await;
            *status = OrchestratorStatus::Running;
        }
        let _ = event_tx.send(SwarmEvent::Started).await;
        self.event_bus
            .publish(crate::services::event_factory::orchestrator_event(
                crate::services::event_bus::EventSeverity::Info,
                crate::services::event_bus::EventPayload::OrchestratorStarted,
            ))
            .await;

        // Spawn bridge: forward EventBus events to legacy event_tx channel for TUI/logging
        {
            let mut bus_rx = self.event_bus.subscribe();
            let bridge_tx = event_tx.clone();
            supervise("eventbus_swarm_bridge", async move {
                loop {
                    match bus_rx.recv().await {
                        Ok(unified_event) => {
                            if let Some(swarm_event) =
                                SwarmEvent::from_event_payload(&unified_event.payload)
                            {
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
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                format!(
                    "Swarm orchestrator started with max {} agents",
                    self.config.max_agents
                ),
            )
            .await;

        // Check for origin remote — warn early if running without one
        self.check_remote_at_startup();

        // Run cold start if memory is empty (populates initial project context)
        if self.memory_repo.is_some() {
            match self.cold_start().await {
                Ok(Some(report)) => {
                    self.audit_log
                        .info(
                            AuditCategory::Memory,
                            AuditAction::MemoryStored,
                            format!(
                                "Cold start completed: {} memories created, project type: {}",
                                report.memories_created, report.project_type
                            ),
                        )
                        .await;
                }
                Ok(None) => {
                    // Memory already populated, skip cold start
                }
                Err(e) => {
                    self.audit_log
                        .log(AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::System,
                            AuditAction::SwarmStarted,
                            AuditActor::System,
                            format!("Cold start failed (non-fatal): {}", e),
                        ))
                        .await;
                }
            }
        }

        // Start memory decay daemon if memory repo is available
        if self.memory_repo.is_some()
            && let Err(e) = self.start_decay_daemon().await
        {
            self.audit_log
                .log(AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    AuditActor::System,
                    format!("Failed to start decay daemon (non-fatal): {}", e),
                ))
                .await;
        }

        // Start hourly token counter reset task
        {
            let cancel = tokio_util::sync::CancellationToken::new();
            let _hourly_reset_handle = self.guardrails.spawn_hourly_reset(cancel.clone());
            *self.hourly_reset_cancel.write().await = Some(cancel);
            tracing::info!("hourly token reset daemon started");
        }

        // Start outbox poller for reliable event delivery
        self.start_outbox_poller().await;

        // Refresh active goals cache for agent context
        if let Err(e) = self.refresh_active_goals_cache().await {
            self.audit_log
                .log(AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    AuditActor::System,
                    format!("Failed to cache active goals: {}", e),
                ))
                .await;
        }

        // Seed baseline agent templates (DB is sole source, hardcoded as bootstrap)
        {
            use crate::services::AgentService;
            let agent_service = AgentService::new(self.agent_repo.clone(), self.event_bus.clone());
            // Use routing-aware seeding when all_workflows is populated; otherwise fall
            // back to single-workflow seeding (legacy / empty config path).
            let seed_result = if !self.config.all_workflows.is_empty() {
                agent_service
                    .seed_baseline_agents_with_workflows(
                        &self.config.all_workflows,
                        self.config.overmind_max_turns,
                    )
                    .await
            } else {
                agent_service
                    .seed_baseline_agents_with_workflow(
                        self.config.workflow_template.as_ref(),
                        self.config.overmind_max_turns,
                    )
                    .await
            };
            match seed_result {
                Ok(seeded) if !seeded.is_empty() => {
                    self.audit_log
                        .info(
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            format!(
                                "Seeded {} baseline agent templates: {}",
                                seeded.len(),
                                seeded.join(", ")
                            ),
                        )
                        .await;
                }
                Ok(_) => {
                    // All agents already exist
                }
                Err(e) => {
                    self.audit_log
                        .log(AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!("Failed to seed agent templates (non-fatal): {}", e),
                        ))
                        .await;
                }
            }
        }

        // Run startup codebase triage if memory repo is available
        if self.memory_repo.is_some() {
            match self.run_startup_triage().await {
                Ok(true) => {
                    self.audit_log
                        .info(
                            AuditCategory::Memory,
                            AuditAction::MemoryStored,
                            "Startup codebase triage completed — profile stored in memory",
                        )
                        .await;
                }
                Ok(false) => {
                    // Codebase profile already exists, skip triage
                }
                Err(e) => {
                    self.audit_log
                        .log(AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::System,
                            AuditAction::SwarmStarted,
                            AuditActor::System,
                            format!("Startup triage failed (non-fatal): {}", e),
                        ))
                        .await;
                }
            }
        }

        // Register existing agent templates with A2A gateway for discovery
        if self.config.mcp_servers.a2a_gateway.is_some()
            && let Err(e) = self.register_all_agent_templates().await
        {
            self.audit_log
                .log(AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    AuditActor::System,
                    format!("Failed to register agent templates with A2A gateway: {}", e),
                ))
                .await;
        }

        // Wait for MCP servers to become healthy before entering the main loop.
        // If servers never come up, abort startup rather than spawning agents
        // into an environment where they can't reach the orchestration APIs.
        if let Err(e) = self.await_mcp_readiness().await {
            self.audit_log
                .log(AuditEntry::new(
                    AuditLevel::Error,
                    AuditCategory::System,
                    AuditAction::SwarmStarted,
                    AuditActor::System,
                    format!("Aborting orchestrator: {}", e),
                ))
                .await;
            let _ = event_tx.send(SwarmEvent::Stopped).await;
            self.event_bus
                .publish(crate::services::event_factory::orchestrator_event(
                    crate::services::event_bus::EventSeverity::Info,
                    crate::services::event_bus::EventPayload::OrchestratorStopped,
                ))
                .await;
            return Err(e);
        }

        // Initialize EventBus sequence from store to prevent overlap after restart
        self.event_bus.initialize_sequence_from_store().await;

        // Register built-in event handlers and schedules BEFORE starting
        // the reactor, so handlers are ready when it begins subscribing.
        self.register_builtin_handlers().await;
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Registered built-in event handlers",
            )
            .await;

        // Register default pre-spawn / post-completion middleware chains.
        // These preserve the logic previously hardcoded inline in
        // spawn_task_agent / run_post_completion_workflow. External callers
        // that registered extra middleware via `with_*_middleware` keep those
        // (they were registered earlier and retain their position).
        self.register_builtin_middleware().await;
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Registered built-in lifecycle middleware",
            )
            .await;

        // Load persisted circuit breaker states (after handler registration)
        self.event_reactor.load_circuit_breaker_states().await;

        self.register_builtin_schedules().await;
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Registered built-in scheduled events",
            )
            .await;

        // Start EventReactor (handlers are already registered)
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Starting EventReactor for reactive event handling",
            )
            .await;
        let reactor_handle = self.event_reactor.start();

        // Load persistent scheduler state from DB before starting
        self.event_scheduler.initialize_from_store().await;

        // Start EventScheduler
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                "Starting EventScheduler for time-based events",
            )
            .await;
        let scheduler_handle = self.event_scheduler.start();

        // Replay missed events from the event store
        match self.event_reactor.replay_missed_events().await {
            Ok(count) if count > 0 => {
                self.audit_log
                    .info(
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        format!("Replayed {} missed events from event store", count),
                    )
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                self.audit_log
                    .log(AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        AuditActor::System,
                        format!("Failed to replay missed events (non-fatal): {}", e),
                    ))
                    .await;
            }
        }

        // Run startup reconciliation to fix inconsistent state
        match self.run_startup_reconciliation().await {
            Ok(count) if count > 0 => {
                self.audit_log
                    .info(
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        format!("Startup reconciliation: {} corrections applied", count),
                    )
                    .await;
            }
            Ok(_) => {}
            Err(e) => {
                self.audit_log
                    .log(AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::System,
                        AuditAction::SwarmStarted,
                        AuditActor::System,
                        format!("Startup reconciliation failed (non-fatal): {}", e),
                    ))
                    .await;
            }
        }

        // The reconciliation interval is a safety net. Handlers do the
        // fast-path work (task cascades, retries, stats). The main loop wakes
        // on ready-task and specialist channel signals for low-latency
        // dispatch, and falls back to the interval for periodic reconciliation
        // (evolution refinements, idle auto-shutdown, table pruning) when the
        // channels are quiet.
        let reconciliation_secs = self.config.reconciliation_interval_secs.unwrap_or(30);
        let loop_interval = tokio::time::Duration::from_secs(reconciliation_secs);
        let mut reconciliation_interval = tokio::time::interval(loop_interval);
        reconciliation_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Periodic cleanup for processed_commands table (Issue #59).
        // Every ~24h worth of timer ticks, prune entries older than 7 days.
        let cleanup_every_n_ticks: u64 = if reconciliation_secs > 0 {
            (24 * 3600) / reconciliation_secs // ~2880 ticks at 30s
        } else {
            2880
        };
        let mut tick_counter: u64 = 0;
        let mut idle_terminal_ticks: u64 = 0; // consecutive timer ticks with all terminal
        let command_retention = std::time::Duration::from_secs(7 * 24 * 3600); // 7 days

        enum Wake {
            Timer,
            ReadyTask(uuid::Uuid),
            Specialist,
        }

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

            // Race the reconciliation interval against the spawn channels.
            // `biased` prefers event branches so a busy stream of task signals
            // gets dispatched with minimum latency; the timer branch only
            // wins during lulls, which is exactly when the safety-net
            // maintenance is cheapest to run. Both `recv()` and `tick()` are
            // cancel-safe, so losing a race doesn't drop a message.
            let wake = {
                let mut ready_rx = self.ready_task_rx.lock().await;
                let mut specialist_rx = self.specialist_rx.lock().await;
                tokio::select! {
                    biased;
                    Some(id) = ready_rx.recv() => Wake::ReadyTask(id),
                    Some(_id) = specialist_rx.recv() => Wake::Specialist,
                    _ = reconciliation_interval.tick() => Wake::Timer,
                }
            };

            // Primed spawn for a ready-task wake: handle this id before
            // draining so the first newly-ready task hits an agent without
            // waiting for the rest of the drain pass.
            if let Wake::ReadyTask(task_id) = wake
                && let Ok(Some(task)) = self.task_repo.get(task_id).await
                && task.status == crate::domain::models::TaskStatus::Ready
                && let Err(e) = self.spawn_task_agent(&task, &event_tx).await
            {
                tracing::error!(
                    error = %e,
                    task_id = %task_id,
                    "spawn_task_agent (primed) subsystem error (isolated)"
                );
                self.event_bus
                    .publish(crate::services::event_factory::orchestrator_event(
                        crate::services::event_bus::EventSeverity::Error,
                        crate::services::event_bus::EventPayload::SubsystemError {
                            subsystem: "spawn_task_agent".into(),
                            error: e.to_string(),
                        },
                    ))
                    .await;
            }

            // Specialist wake consumed the signalling id from the channel
            // but drain_specialist_tasks only fires when try_recv returns at
            // least one id. Call the processor directly so the primed signal
            // isn't lost; it scans the DB for all permanently-failed tasks,
            // so we don't need the specific id.
            if matches!(wake, Wake::Specialist)
                && let Err(e) = self.process_specialist_triggers(&event_tx).await
            {
                tracing::error!(error = %e, "process_specialist_triggers (primed) subsystem error (isolated)");
                self.event_bus
                    .publish(crate::services::event_factory::orchestrator_event(
                        crate::services::event_bus::EventSeverity::Error,
                        crate::services::event_bus::EventPayload::SubsystemError {
                            subsystem: "process_specialist_triggers".into(),
                            error: e.to_string(),
                        },
                    ))
                    .await;
            }

            // Drain any remaining queued tasks (and the DB safety-net scan
            // inside drain_ready_tasks). Cheap when the channels are empty.
            if let Err(e) = self.drain_ready_tasks(&event_tx).await {
                tracing::error!(error = %e, "drain_ready_tasks subsystem error (isolated)");
                self.event_bus
                    .publish(crate::services::event_factory::orchestrator_event(
                        crate::services::event_bus::EventSeverity::Error,
                        crate::services::event_bus::EventPayload::SubsystemError {
                            subsystem: "drain_ready_tasks".into(),
                            error: e.to_string(),
                        },
                    ))
                    .await;
            }

            if let Err(e) = self.drain_specialist_tasks(&event_tx).await {
                tracing::error!(error = %e, "drain_specialist_tasks subsystem error (isolated)");
                self.event_bus
                    .publish(crate::services::event_factory::orchestrator_event(
                        crate::services::event_bus::EventSeverity::Error,
                        crate::services::event_bus::EventPayload::SubsystemError {
                            subsystem: "drain_specialist_tasks".into(),
                            error: e.to_string(),
                        },
                    ))
                    .await;
            }

            // Timer-bound reconciliation. Gated to timer wakes so bursts of
            // task-ready signals don't accelerate evolution evaluation,
            // auto-shutdown detection, or the 24h cleanup cadence — all of
            // which assume per-interval semantics.
            if !matches!(wake, Wake::Timer) {
                continue;
            }

            if self.config.track_evolution
                && let Err(e) = self.process_evolution_refinements(&event_tx).await
            {
                tracing::error!(error = %e, "process_evolution_refinements subsystem error (isolated)");
                self.event_bus
                    .publish(crate::services::event_factory::orchestrator_event(
                        crate::services::event_bus::EventSeverity::Error,
                        crate::services::event_bus::EventPayload::SubsystemError {
                            subsystem: "process_evolution_refinements".into(),
                            error: e.to_string(),
                        },
                    ))
                    .await;
            }

            // Auto-shutdown: if all goals and tasks have reached terminal state
            // for 2 consecutive timer ticks, initiate graceful shutdown.
            {
                use crate::domain::ports::GoalFilter;
                let all_goals = self
                    .goal_repo
                    .list(GoalFilter::default())
                    .await
                    .unwrap_or_default();
                if !all_goals.is_empty() && all_goals.iter().all(|g| g.is_terminal()) {
                    use crate::domain::ports::TaskFilter;
                    let all_tasks = self
                        .task_repo
                        .list(TaskFilter::default())
                        .await
                        .unwrap_or_default();
                    let has_active = all_tasks.iter().any(|t| t.status.is_active());
                    if !has_active {
                        idle_terminal_ticks += 1;
                        if idle_terminal_ticks >= 2 {
                            tracing::info!(
                                goal_count = all_goals.len(),
                                task_count = all_tasks.len(),
                                "All goals and tasks are terminal — initiating auto-shutdown"
                            );
                            self.stop().await;
                            continue;
                        }
                    } else {
                        idle_terminal_ticks = 0;
                    }
                } else {
                    idle_terminal_ticks = 0;
                }
            }

            // Periodic maintenance: prune stale processed_commands entries
            tick_counter += 1;
            if tick_counter.is_multiple_of(cleanup_every_n_ticks)
                && let Some(bus) = self.command_bus.read().await.as_ref()
            {
                let pruned = bus.prune_old_commands(command_retention).await;
                if pruned > 0 {
                    tracing::info!(
                        pruned_count = pruned,
                        retention_days = 7,
                        "Pruned stale processed_commands entries"
                    );
                }
            }
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
        self.audit_log
            .info(
                AuditCategory::System,
                AuditAction::SwarmStopped,
                "Swarm orchestrator stopped",
            )
            .await;

        // Stop hourly token reset daemon if running
        if let Some(cancel) = self.hourly_reset_cancel.read().await.as_ref() {
            cancel.cancel();
            tracing::info!("hourly token reset daemon stopped");
        }

        // Stop decay daemon if running
        self.stop_decay_daemon().await;

        // Stop embedded MCP servers if running
        self.stop_embedded_mcp_servers().await;

        let _ = event_tx.send(SwarmEvent::Stopped).await;
        self.event_bus
            .publish(crate::services::event_factory::orchestrator_event(
                crate::services::event_bus::EventSeverity::Info,
                crate::services::event_bus::EventPayload::OrchestratorStopped,
            ))
            .await;
        Ok(())
    }

    /// Run a single iteration of the orchestration loop.
    pub async fn tick(&self) -> DomainResult<SwarmStats> {
        let (tx, _rx) = mpsc::channel(100);

        // Drain ready-task channel and spawn agents
        if let Err(e) = self.drain_ready_tasks(&tx).await {
            tracing::error!(error = %e, "tick: drain_ready_tasks subsystem error (isolated)");
            self.event_bus
                .publish(crate::services::event_factory::orchestrator_event(
                    crate::services::event_bus::EventSeverity::Error,
                    crate::services::event_bus::EventPayload::SubsystemError {
                        subsystem: "drain_ready_tasks".into(),
                        error: e.to_string(),
                    },
                ))
                .await;
        }

        // Update stats
        if let Err(e) = self.update_stats(&tx).await {
            tracing::error!(error = %e, "tick: update_stats subsystem error (isolated)");
            self.event_bus
                .publish(crate::services::event_factory::orchestrator_event(
                    crate::services::event_bus::EventSeverity::Error,
                    crate::services::event_bus::EventPayload::SubsystemError {
                        subsystem: "update_stats".into(),
                        error: e.to_string(),
                    },
                ))
                .await;
        }

        Ok(self.stats().await)
    }

    /// Drain the ready-task channel and spawn agents for each ready task.
    async fn drain_ready_tasks(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let mut rx = self.ready_task_rx.lock().await;
        let mut spawned_ids = std::collections::HashSet::new();

        while let Ok(task_id) = rx.try_recv() {
            // Fetch and validate task is still Ready
            if let Ok(Some(task)) = self.task_repo.get(task_id).await
                && task.status == crate::domain::models::TaskStatus::Ready
            {
                self.spawn_task_agent(&task, event_tx).await?;
                spawned_ids.insert(task_id);
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
    async fn drain_specialist_tasks(
        &self,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let mut rx = self.specialist_rx.lock().await;

        while let Ok(task_id) = rx.try_recv() {
            // Validate task is still in a state that warrants specialist attention
            if let Ok(Some(task)) = self.task_repo.get(task_id).await
                && task.status == crate::domain::models::TaskStatus::Failed
            {
                // Delegate to existing specialist processing
                self.process_specialist_triggers(event_tx).await?;
                break; // process_specialist_triggers handles all pending specialists
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
        SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository, SqliteTaskRepository,
        SqliteWorktreeRepository, create_migrated_test_pool,
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
        // Disable worktrees for tests.
        let config = SwarmConfig {
            use_worktrees: false,
            ..Default::default()
        };

        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let event_reactor = Arc::new(EventReactor::new(
            event_bus.clone(),
            ReactorConfig::default(),
        ));
        let event_scheduler = Arc::new(EventScheduler::new(
            event_bus.clone(),
            SchedulerConfig::default(),
        ));

        SwarmOrchestrator::new(
            goal_repo,
            task_repo,
            worktree_repo,
            agent_repo,
            substrate,
            config,
            event_bus,
            event_reactor,
            event_scheduler,
        )
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

    // ------------------------------------------------------------------------
    // validate_dependencies() — startup validation
    // ------------------------------------------------------------------------

    /// Build an orchestrator with the "low-fuss" config path — all
    /// convergence / verification / merge-queue features disabled — so we can
    /// validate individual feature toggles in isolation.
    async fn setup_orchestrator_bare(
        config: SwarmConfig,
    ) -> SwarmOrchestrator<
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
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());

        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let event_reactor = Arc::new(EventReactor::new(
            event_bus.clone(),
            ReactorConfig::default(),
        ));
        let event_scheduler = Arc::new(EventScheduler::new(
            event_bus.clone(),
            SchedulerConfig::default(),
        ));

        SwarmOrchestrator::new(
            goal_repo,
            task_repo,
            worktree_repo,
            agent_repo,
            substrate,
            config,
            event_bus,
            event_reactor,
            event_scheduler,
        )
    }

    /// Config with all optional-feature toggles off, so validate_dependencies
    /// passes without any `with_xxx` builder calls.
    fn disabled_feature_config() -> SwarmConfig {
        SwarmConfig {
            use_worktrees: false,
            convergence_enabled: false,
            enable_intent_verification: false,
            use_merge_queue: false,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_validate_dependencies_all_off_passes() {
        // With every feature toggle disabled, no dependencies are required,
        // so validation should pass with a bare orchestrator.
        let orchestrator = setup_orchestrator_bare(disabled_feature_config()).await;
        let result = orchestrator.validate_dependencies();
        assert!(
            result.is_ok(),
            "validate_dependencies should pass when all feature toggles are off, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_validate_dependencies_convergence_missing_trajectory_fails() {
        // convergence_enabled=true but no trajectory_repo → must fail with a
        // message that names the missing dependency and the builder method.
        let config = SwarmConfig {
            use_worktrees: false,
            convergence_enabled: true,
            // Keep other features off so we isolate the trajectory_repo check.
            enable_intent_verification: false,
            use_merge_queue: false,
            ..Default::default()
        };
        let orchestrator = setup_orchestrator_bare(config).await;

        let err = orchestrator
            .validate_dependencies()
            .expect_err("expected validation to fail when trajectory_repo is missing");
        let msg = err.to_string();
        assert!(
            msg.contains("trajectory repository"),
            "error should name the missing dependency; got: {}",
            msg
        );
        assert!(
            msg.contains("with_trajectory_repo"),
            "error should name the builder method to call; got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_validate_dependencies_merge_queue_missing_mr_repo_fails() {
        // use_merge_queue=true but no merge_request_repo → must fail.
        let config = SwarmConfig {
            use_worktrees: false,
            use_merge_queue: true,
            // Keep convergence off so the merge-queue path is exercised in
            // isolation.
            convergence_enabled: false,
            enable_intent_verification: false,
            ..Default::default()
        };
        let orchestrator = setup_orchestrator_bare(config).await;

        let err = orchestrator
            .validate_dependencies()
            .expect_err("expected validation to fail when merge_request_repo is missing");
        let msg = err.to_string();
        assert!(
            msg.contains("merge request repository"),
            "error should name the missing dependency; got: {}",
            msg
        );
        assert!(
            msg.contains("use_merge_queue=true"),
            "error should echo the config flag; got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_validate_dependencies_intent_verification_missing_verifier_fails() {
        // enable_intent_verification=true but no intent_verifier → must fail.
        let config = SwarmConfig {
            use_worktrees: false,
            enable_intent_verification: true,
            convergence_enabled: false,
            use_merge_queue: false,
            ..Default::default()
        };
        let orchestrator = setup_orchestrator_bare(config).await;

        let err = orchestrator
            .validate_dependencies()
            .expect_err("expected validation to fail when intent_verifier is missing");
        let msg = err.to_string();
        assert!(
            msg.contains("intent verifier"),
            "error should name the missing dependency; got: {}",
            msg
        );
    }
}

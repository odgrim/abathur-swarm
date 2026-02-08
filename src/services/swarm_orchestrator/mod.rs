//! Swarm Orchestrator - the central coordinator for the Abathur system.
//!
//! The orchestrator is a thin coordinator over well-defined subsystems:
//!
//! - **types**: Public configuration, event, and status types
//! - **event_handling**: Human escalation, A2A messaging, event bus integration
//! - **agent_lifecycle**: Agent evolution, registration, prompts, goal alignment
//! - **dag_execution**: DAG-based goal execution, convergence loops, merge queue
//! - **goal_processing**: Goal decomposition, task spawning, dependency management
//! - **specialist_triggers**: Failure recovery, restructuring, diagnostics, merge conflicts
//! - **infrastructure**: Cold start, decay daemon, MCP servers, stats, verification
//! - **helpers**: Utility functions for spawned tasks (auto-commit, post-completion)

pub mod types;
mod event_handling;
mod agent_lifecycle;
mod dag_execution;
mod goal_processing;
mod specialist_triggers;
mod infrastructure;
pub(crate) mod helpers;

// Re-export public types
pub use types::{
    ConvergenceLoopConfig, McpServerConfig, OrchestratorStatus, SwarmConfig,
    SwarmEvent, SwarmStats, VerificationLevel,
};

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, HumanEscalationEvent};
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, NullMemoryRepository,
    Substrate, TaskRepository, WorktreeRepository,
};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    AuditLogConfig, AuditLogService,
    CircuitBreakerConfig, CircuitBreakerService,
    DaemonHandle, EvolutionLoop,
    GoalAlignmentService,
    IntentVerifierConfig, IntentVerifierService,
    dag_restructure::DagRestructureService,
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
    pub(super) evolution_loop: Arc<EvolutionLoop>,
    pub(super) goal_alignment: Option<Arc<GoalAlignmentService<G>>>,
    pub(super) active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    pub(super) restructure_service: Arc<tokio::sync::Mutex<DagRestructureService>>,
    pub(super) guardrails: Arc<Guardrails>,
    pub(super) mcp_shutdown_tx: Arc<RwLock<Option<tokio::sync::broadcast::Sender<()>>>>,
    pub(super) intent_verifier: Option<Arc<IntentVerifierService<G, T>>>,
    pub(super) overmind: Option<Arc<crate::services::OvermindService>>,
    pub(super) event_bus: Option<Arc<crate::services::event_bus::EventBus>>,
    pub(super) escalation_store: Arc<RwLock<Vec<HumanEscalationEvent>>>,
    pub(super) federation_client: Option<Arc<crate::adapters::mcp::FederationClient>>,
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
    ) -> Self {
        let max_agents = config.max_agents;
        let goal_alignment = Some(Arc::new(GoalAlignmentService::with_defaults(goal_repo.clone())));
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
            evolution_loop: Arc::new(EvolutionLoop::with_default_config()),
            goal_alignment,
            active_goals_cache: Arc::new(RwLock::new(Vec::new())),
            restructure_service: Arc::new(tokio::sync::Mutex::new(DagRestructureService::with_defaults())),
            guardrails: Arc::new(Guardrails::with_defaults()),
            mcp_shutdown_tx: Arc::new(RwLock::new(None)),
            intent_verifier: None,
            overmind: None,
            event_bus: None,
            escalation_store: Arc::new(RwLock::new(Vec::new())),
            federation_client: None,
        }
    }

    // -- Builder methods --

    /// Create orchestrator with a federation client for cross-swarm task delegation.
    pub fn with_federation(mut self, federation_client: Arc<crate::adapters::mcp::FederationClient>) -> Self {
        self.federation_client = Some(federation_client);
        self
    }

    /// Create orchestrator with an EventBus for unified event streaming.
    pub fn with_event_bus(mut self, event_bus: Arc<crate::services::event_bus::EventBus>) -> Self {
        self.event_bus = Some(event_bus);
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
    // Main Orchestration Loop
    // ========================================================================

    /// Start the orchestrator and run the main loop.
    pub async fn run(&self, event_tx: mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        {
            let mut status = self.status.write().await;
            *status = OrchestratorStatus::Running;
        }
        let _ = event_tx.send(SwarmEvent::Started).await;
        self.emit_to_event_bus(SwarmEvent::Started).await;

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
            let agent_service = AgentService::new(self.agent_repo.clone());
            match agent_service.seed_baseline_agents().await {
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
            self.emit_to_event_bus(SwarmEvent::Stopped).await;
            return Err(e);
        }

        // Main orchestration loop
        loop {
            let current_status = self.status.read().await.clone();

            match current_status {
                OrchestratorStatus::ShuttingDown | OrchestratorStatus::Stopped => {
                    break;
                }
                OrchestratorStatus::Paused => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(self.config.poll_interval_ms)).await;
                    continue;
                }
                _ => {}
            }

            // Update task readiness based on dependencies
            self.update_task_readiness(&event_tx).await?;

            // Process active goals
            self.process_goals(&event_tx).await?;

            // Handle retries for failed tasks
            if self.config.auto_retry {
                self.process_retries(&event_tx).await?;
            }

            // Process pending evolution refinements
            if self.config.track_evolution {
                self.process_evolution_refinements(&event_tx).await?;
            }

            // Process specialist agent triggers (conflicts, persistent failures, etc.)
            self.process_specialist_triggers(&event_tx).await?;

            // Detect stalled goals (active goals with no in-flight work)
            self.detect_stalled_goals(&event_tx).await?;

            // Process A2A delegation requests from agents
            if self.config.mcp_servers.a2a_gateway.is_some() {
                self.process_a2a_delegations(&event_tx).await?;
            }

            // Check escalation deadlines
            self.check_escalation_deadlines(&event_tx).await?;

            // Update stats
            self.update_stats(&event_tx).await?;

            // Wait before next iteration
            tokio::time::sleep(tokio::time::Duration::from_millis(self.config.poll_interval_ms)).await;
        }

        // Log swarm shutdown
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStopped,
            "Swarm orchestrator stopped",
        ).await;

        // Stop decay daemon if running
        self.stop_decay_daemon().await;

        // Stop embedded MCP servers if running
        self.stop_embedded_mcp_servers().await;

        let _ = event_tx.send(SwarmEvent::Stopped).await;
        self.emit_to_event_bus(SwarmEvent::Stopped).await;
        Ok(())
    }

    /// Run a single iteration of the orchestration loop.
    pub async fn tick(&self) -> DomainResult<SwarmStats> {
        let (tx, _rx) = mpsc::channel(100);

        // Update task readiness
        self.update_task_readiness(&tx).await?;

        // Process goals
        self.process_goals(&tx).await?;

        // Handle retries
        if self.config.auto_retry {
            self.process_retries(&tx).await?;
        }

        // Detect stalled goals
        self.detect_stalled_goals(&tx).await?;

        // Update stats
        self.update_stats(&tx).await?;

        Ok(self.stats().await)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_test_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteMemoryRepository,
        SqliteTaskRepository, SqliteWorktreeRepository, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::MockSubstrate;

    async fn setup_orchestrator() -> SwarmOrchestrator<
        SqliteGoalRepository,
        SqliteTaskRepository,
        SqliteWorktreeRepository,
        SqliteAgentRepository,
        SqliteMemoryRepository,
    > {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool.clone()));
        let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
        let memory_repo = Arc::new(SqliteMemoryRepository::new(pool));
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let mut config = SwarmConfig::default();
        config.use_worktrees = false; // Disable worktrees for tests

        SwarmOrchestrator::new(goal_repo, task_repo, worktree_repo, agent_repo, substrate, config)
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

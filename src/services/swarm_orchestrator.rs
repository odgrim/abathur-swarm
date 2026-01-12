//! Swarm Orchestrator - the central coordinator for the Abathur system.
//!
//! The orchestrator manages the execution loop, coordinating between:
//! - Goals and their task decomposition
//! - Task scheduling and DAG execution
//! - Agent spawning and management
//! - Worktree management for isolation
//! - Memory system for context sharing

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    Goal, GoalStatus, SessionStatus, SubstrateConfig, SubstrateRequest, Task, TaskDag, TaskStatus,
};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, NullMemoryRepository, Substrate, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, AuditLogConfig, AuditLogService,
    CircuitBreakerConfig, CircuitBreakerService, CircuitScope,
    ColdStartConfig, ColdStartService, ColdStartReport,
    DagExecutor, DecayDaemonConfig, DaemonHandle, ExecutionEvent, ExecutionResults, ExecutionStatus, ExecutorConfig,
    EvolutionLoop, TaskExecution, TaskOutcome,
    GoalAlignmentService, HolisticEvaluation,
    IntegrationVerifierService, VerificationResult, VerifierConfig,
    MemoryDecayDaemon, MemoryService,
    MergeQueue, MergeQueueConfig,
    MetaPlanner, MetaPlannerConfig,
    WorktreeConfig, WorktreeService,
    dag_restructure::{DagRestructureService, RestructureContext, RestructureDecision, RestructureTrigger, TaskPriorityModifier},
    guardrails::{Guardrails, GuardrailsConfig},
};

/// Configuration for the swarm orchestrator.
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Maximum concurrent agents.
    pub max_agents: usize,
    /// Default max turns per agent invocation.
    pub default_max_turns: u32,
    /// Whether to use worktrees for task isolation.
    pub use_worktrees: bool,
    /// Poll interval for goal progress (ms).
    pub poll_interval_ms: u64,
    /// Maximum execution time per goal (seconds).
    pub goal_timeout_secs: u64,
    /// Whether to auto-retry failed tasks.
    pub auto_retry: bool,
    /// Maximum retries per task.
    pub max_task_retries: u32,
    /// Base path for worktrees.
    pub worktree_base_path: PathBuf,
    /// Repository path.
    pub repo_path: PathBuf,
    /// Default base ref for worktrees.
    pub default_base_ref: String,
    /// Whether to use LLM for task decomposition.
    pub use_llm_decomposition: bool,
    /// Whether to run integration verification on task completion.
    pub verify_on_completion: bool,
    /// Whether to use merge queue for controlled merging.
    pub use_merge_queue: bool,
    /// Whether to track agent evolution metrics.
    pub track_evolution: bool,
    /// MCP server addresses for agent access to system services.
    /// These get passed to substrate requests so agents can access memory, tasks, etc.
    pub mcp_servers: McpServerConfig,
}

/// MCP server configuration for agent access to system services.
#[derive(Debug, Clone, Default)]
pub struct McpServerConfig {
    /// Memory MCP server address (e.g., "http://localhost:8081/mcp")
    pub memory_server: Option<String>,
    /// Tasks MCP server address (e.g., "http://localhost:8082/mcp")
    pub tasks_server: Option<String>,
    /// A2A gateway address (e.g., "http://localhost:8083/a2a")
    pub a2a_gateway: Option<String>,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_agents: 4,
            default_max_turns: 25,
            use_worktrees: true,
            poll_interval_ms: 1000,
            goal_timeout_secs: 3600,
            auto_retry: true,
            max_task_retries: 3,
            worktree_base_path: PathBuf::from(".abathur/worktrees"),
            repo_path: PathBuf::from("."),
            default_base_ref: "main".to_string(),
            use_llm_decomposition: false,
            verify_on_completion: true,
            use_merge_queue: true,
            track_evolution: true,
            mcp_servers: McpServerConfig::default(),
        }
    }
}

/// Orchestrator status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorStatus {
    /// Not running.
    Idle,
    /// Running and processing goals.
    Running,
    /// Paused but can resume.
    Paused,
    /// Shutting down gracefully.
    ShuttingDown,
    /// Stopped.
    Stopped,
}

/// Event emitted by the orchestrator.
#[derive(Debug, Clone)]
pub enum SwarmEvent {
    /// Orchestrator started.
    Started,
    /// Goal processing started.
    GoalStarted { goal_id: Uuid, goal_name: String },
    /// Goal decomposed into tasks.
    GoalDecomposed { goal_id: Uuid, task_count: usize },
    /// Task submitted (created and added to the system).
    TaskSubmitted { task_id: Uuid, task_title: String, goal_id: Uuid },
    /// Task readiness updated.
    TaskReady { task_id: Uuid, task_title: String },
    /// Task spawned.
    TaskSpawned { task_id: Uuid, task_title: String, agent_type: Option<String> },
    /// Worktree created for task.
    WorktreeCreated { task_id: Uuid, path: String },
    /// Task completed.
    TaskCompleted { task_id: Uuid, tokens_used: u64 },
    /// Task failed.
    TaskFailed { task_id: Uuid, error: String, retry_count: u32 },
    /// Task retrying.
    TaskRetrying { task_id: Uuid, attempt: u32, max_attempts: u32 },
    /// Task verified.
    TaskVerified { task_id: Uuid, passed: bool, checks_passed: usize, checks_total: usize },
    /// Task queued for merge.
    TaskQueuedForMerge { task_id: Uuid, stage: String },
    /// Task merged successfully.
    TaskMerged { task_id: Uuid, commit_sha: String },
    /// Evolution event triggered.
    EvolutionTriggered { template_name: String, trigger: String },
    /// Specialist agent spawned for special handling.
    SpecialistSpawned { specialist_type: String, trigger: String, task_id: Option<Uuid> },
    /// Agent dynamically created through capability-driven genesis.
    AgentCreated { agent_type: String, tier: String },
    /// Goal alignment evaluated.
    GoalAlignmentEvaluated { task_id: Uuid, overall_score: f64, passes: bool },
    /// DAG restructure triggered for a permanently failed task.
    RestructureTriggered { task_id: Uuid, decision: String },
    /// Goal completed.
    GoalCompleted { goal_id: Uuid },
    /// Goal failed.
    GoalFailed { goal_id: Uuid, error: String },
    /// Orchestrator paused.
    Paused,
    /// Orchestrator resumed.
    Resumed,
    /// Orchestrator stopped.
    Stopped,
    /// Status update.
    StatusUpdate(SwarmStats),
}

/// Statistics about the swarm.
#[derive(Debug, Clone, Default)]
pub struct SwarmStats {
    pub active_goals: usize,
    pub pending_tasks: usize,
    pub ready_tasks: usize,
    pub running_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub active_agents: usize,
    pub active_worktrees: usize,
    pub total_tokens_used: u64,
}

/// The main swarm orchestrator.
pub struct SwarmOrchestrator<G, T, W, A, M = NullMemoryRepository>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
    agent_repo: Arc<A>,
    memory_repo: Option<Arc<M>>,
    substrate: Arc<dyn Substrate>,
    config: SwarmConfig,
    status: Arc<RwLock<OrchestratorStatus>>,
    stats: Arc<RwLock<SwarmStats>>,
    agent_semaphore: Arc<Semaphore>,
    total_tokens: Arc<AtomicU64>,
    // Integrated services
    audit_log: Arc<AuditLogService>,
    circuit_breaker: Arc<CircuitBreakerService>,
    decay_daemon_handle: Arc<RwLock<Option<DaemonHandle>>>,
    // Evolution tracking for agent template improvement
    evolution_loop: Arc<EvolutionLoop>,
    // Goal alignment service for holistic evaluation
    goal_alignment: Option<Arc<GoalAlignmentService<G>>>,
    // Active goals cache for agent context
    active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    // DAG restructure service for failure recovery
    restructure_service: Arc<tokio::sync::Mutex<DagRestructureService>>,
    // Guardrails for safety limits (tokens, cost, concurrent tasks)
    guardrails: Arc<Guardrails>,
}

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
        }
    }

    /// Create orchestrator with custom guardrails configuration.
    pub fn with_guardrails(mut self, config: GuardrailsConfig) -> Self {
        self.guardrails = Arc::new(Guardrails::new(config));
        self
    }

    /// Get the guardrails service for external use.
    pub fn guardrails(&self) -> &Arc<Guardrails> {
        &self.guardrails
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

    /// Verify a completed task using the IntegrationVerifier.
    ///
    /// Returns the verification result if verification is enabled and passes.
    pub async fn verify_task(&self, task_id: Uuid) -> DomainResult<Option<VerificationResult>> {
        if !self.config.verify_on_completion {
            return Ok(None);
        }

        let verifier = IntegrationVerifierService::new(
            self.task_repo.clone(),
            self.goal_repo.clone(),
            self.worktree_repo.clone(),
            VerifierConfig::default(),
        );

        let result = verifier.verify_task(task_id).await?;

        // Compute check statistics
        let checks_total = result.checks.len();
        let checks_passed = result.checks.iter().filter(|c| c.passed).count();

        // Log verification result
        if result.passed {
            self.audit_log.info(
                AuditCategory::Task,
                AuditAction::TaskCompleted,
                format!(
                    "Task {} passed verification: {}/{} checks",
                    task_id, checks_passed, checks_total
                ),
            ).await;
        } else {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Task,
                    AuditAction::TaskFailed,
                    AuditActor::System,
                    format!(
                        "Task {} failed verification: {}",
                        task_id, result.failures_summary.clone().unwrap_or_default()
                    ),
                )
                .with_entity(task_id, "task"),
            ).await;
        }

        Ok(Some(result))
    }

    /// Send a message to another agent via A2A protocol.
    ///
    /// This allows agents to communicate and coordinate work.
    pub async fn send_a2a_message(
        &self,
        from_agent: &str,
        to_agent: &str,
        message_type: crate::domain::models::a2a::MessageType,
        subject: &str,
        content: &str,
    ) -> DomainResult<()> {
        use crate::domain::models::a2a::A2AMessage;

        let message = A2AMessage::new(message_type, from_agent, to_agent, subject, content);

        // Log the A2A message
        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned, // Could add A2AMessageSent action
            format!(
                "A2A message from '{}' to '{}': {} ({})",
                from_agent, to_agent, message_type.as_str(), message.id
            ),
        ).await;

        // If A2A gateway is configured, route the message via HTTP
        if let Some(ref gateway_url) = self.config.mcp_servers.a2a_gateway {
            // Build JSON-RPC request for tasks/send
            let request_id = Uuid::new_v4().to_string();
            let json_rpc_request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tasks/send",
                "params": {
                    "id": message.id.to_string(),
                    "message": {
                        "role": "user",
                        "parts": [{
                            "type": "text",
                            "text": format!(
                                "[A2A Message]\nFrom: {}\nTo: {}\nType: {}\nSubject: {}\n\n{}",
                                from_agent, to_agent, message_type.as_str(), subject, content
                            )
                        }]
                    },
                    "metadata": {
                        "from_agent": from_agent,
                        "to_agent": to_agent,
                        "message_type": message_type.as_str(),
                        "subject": subject,
                        "message_id": message.id.to_string(),
                        "task_id": message.task_id.as_ref().map(|t| t.to_string()),
                        "goal_id": message.goal_id.as_ref().map(|g| g.to_string()),
                    }
                }
            });

            // Send HTTP POST to A2A gateway
            let client = reqwest::Client::new();
            let rpc_url = format!("{}/rpc", gateway_url.trim_end_matches('/'));

            match client.post(&rpc_url)
                .json(&json_rpc_request)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            "A2A message {} routed successfully via gateway: {} -> {}",
                            message.id, from_agent, to_agent
                        );
                    } else {
                        tracing::warn!(
                            "A2A gateway returned error status {} for message {}",
                            response.status(), message.id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to route A2A message {} via gateway: {}",
                        message.id, e
                    );
                    // Don't fail the operation - message routing is best-effort
                }
            }
        } else {
            tracing::debug!(
                "A2A message {} not routed (no gateway configured): {} -> {}",
                message.id, from_agent, to_agent
            );
        }

        Ok(())
    }

    /// Register an agent's capabilities with the A2A registry.
    ///
    /// This allows other agents to discover and communicate with this agent.
    pub async fn register_agent_capabilities(
        &self,
        agent_name: &str,
        capabilities: Vec<String>,
    ) -> DomainResult<()> {
        use crate::domain::models::a2a::A2AAgentCard;

        let mut card = A2AAgentCard::new(agent_name);
        for cap in capabilities {
            card = card.with_capability(cap);
        }

        // Log the registration
        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Agent '{}' registered with {} capabilities",
                agent_name, card.capabilities.len()
            ),
        ).await;

        // If A2A gateway is configured, register the agent card
        if let Some(ref gateway_url) = self.config.mcp_servers.a2a_gateway {
            let register_url = format!("{}/agents", gateway_url.trim_end_matches('/'));

            let client = reqwest::Client::new();
            match client.post(&register_url)
                .json(&card)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            "Agent '{}' card registered with A2A gateway",
                            agent_name
                        );
                    } else {
                        tracing::warn!(
                            "A2A gateway returned error status {} when registering agent '{}'",
                            response.status(), agent_name
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to register agent '{}' with A2A gateway: {}",
                        agent_name, e
                    );
                    // Don't fail the operation - registration is best-effort
                }
            }
        }

        Ok(())
    }

    /// Register all existing agent templates with the A2A gateway at startup.
    ///
    /// This enables agent discovery for all known agent types.
    async fn register_all_agent_templates(&self) -> DomainResult<()> {
        // Get all agent templates from the repository
        use crate::domain::ports::AgentFilter;
        let templates = self.agent_repo.list_templates(AgentFilter::default()).await?;

        if templates.is_empty() {
            self.audit_log.info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned,
                "No agent templates to register with A2A gateway".to_string(),
            ).await;
            return Ok(());
        }

        let mut registered_count = 0;
        for template in templates {
            // Extract capabilities from template tools
            let capabilities: Vec<String> = template.tools
                .iter()
                .map(|t| t.name.clone())
                .collect();

            // Add default capability if no tools defined
            let capabilities = if capabilities.is_empty() {
                vec!["task-execution".to_string()]
            } else {
                capabilities
            };

            if self.register_agent_capabilities(&template.name, capabilities).await.is_ok() {
                registered_count += 1;
            }
        }

        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Registered {} agent templates with A2A gateway at startup",
                registered_count
            ),
        ).await;

        Ok(())
    }

    /// Execute a goal's tasks using the DagExecutor for wave-based parallel execution.
    ///
    /// This provides structured execution with waves, guardrails, and circuit breakers.
    pub async fn execute_goal_with_dag(
        &self,
        goal_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<ExecutionResults> {
        // Get tasks for this goal
        let tasks = self.task_repo.list_by_goal(goal_id).await?;
        if tasks.is_empty() {
            return Ok(ExecutionResults::default());
        }

        // Build DAG from tasks
        let dag = TaskDag::from_tasks(tasks);

        // Fetch project context from semantic memory if available
        let project_context = if let Some(ref memory_repo) = self.memory_repo {
            let memory_service = MemoryService::new(memory_repo.clone());
            // Get goal-specific context and general project knowledge
            let mut context_parts = Vec::new();

            // Fetch goal-related memories
            if let Ok(goal_memories) = memory_service.get_goal_context(goal_id).await {
                for mem in goal_memories.iter().take(5) {
                    context_parts.push(format!("- {}: {}", mem.key, mem.content));
                }
            }

            // Fetch semantic memories (long-term project knowledge)
            if let Ok(semantic_memories) = memory_service.search("architecture project", Some("semantic"), 5).await {
                for mem in semantic_memories {
                    context_parts.push(format!("- {}: {}", mem.key, mem.content));
                }
            }

            if context_parts.is_empty() {
                None
            } else {
                Some(format!("Relevant project knowledge:\n{}", context_parts.join("\n")))
            }
        } else {
            None
        };

        // Create DAG executor with MCP services and goal context
        let executor_config = ExecutorConfig {
            max_concurrency: self.config.max_agents,
            task_timeout_secs: self.config.goal_timeout_secs,
            max_retries: self.config.max_task_retries,
            default_max_turns: self.config.default_max_turns,
            fail_fast: false,
            memory_server_url: self.config.mcp_servers.memory_server.clone(),
            a2a_gateway_url: self.config.mcp_servers.a2a_gateway.clone(),
            tasks_server_url: self.config.mcp_servers.tasks_server.clone(),
            project_context,
        };

        // Create restructure service for failure recovery
        let restructure_service = Arc::new(crate::services::dag_restructure::DagRestructureService::with_defaults());

        let executor = DagExecutor::new(
            self.task_repo.clone(),
            self.agent_repo.clone(),
            self.substrate.clone(),
            executor_config,
        )
        .with_goal_repo(self.goal_repo.clone())
        .with_circuit_breaker(self.circuit_breaker.clone())
        .with_restructure_service(restructure_service.clone())
        .with_guardrails(self.guardrails.clone());

        // Create execution event channel
        let (exec_event_tx, mut exec_event_rx) = mpsc::channel::<ExecutionEvent>(100);

        // Forward execution events to swarm events
        let audit_log = self.audit_log.clone();
        let evolution_loop = self.evolution_loop.clone();
        let track_evolution = self.config.track_evolution;
        let swarm_event_tx = event_tx.clone();

        let event_forwarder = tokio::spawn(async move {
            while let Some(event) = exec_event_rx.recv().await {
                match event {
                    ExecutionEvent::Started { total_tasks, wave_count } => {
                        audit_log.info(
                            AuditCategory::Execution,
                            AuditAction::WaveStarted,
                            format!("DAG execution started: {} tasks in {} waves", total_tasks, wave_count),
                        ).await;
                    }
                    ExecutionEvent::WaveStarted { wave_number, task_count } => {
                        audit_log.info(
                            AuditCategory::Execution,
                            AuditAction::WaveStarted,
                            format!("Wave {} started: {} tasks", wave_number, task_count),
                        ).await;
                    }
                    ExecutionEvent::TaskStarted { task_id, task_title } => {
                        let _ = swarm_event_tx.send(SwarmEvent::TaskSpawned {
                            task_id,
                            task_title,
                            agent_type: None,
                        }).await;
                    }
                    ExecutionEvent::TaskCompleted { task_id, result } => {
                        let tokens = result.session.as_ref().map(|s| s.total_tokens()).unwrap_or(0);
                        let _ = swarm_event_tx.send(SwarmEvent::TaskCompleted {
                            task_id,
                            tokens_used: tokens,
                        }).await;

                        // Record in evolution loop
                        if track_evolution {
                            let turns = result.session.as_ref().map(|s| s.turns_completed).unwrap_or(0);
                            let execution = TaskExecution {
                                task_id,
                                template_name: "dag_executor".to_string(),
                                template_version: 1,
                                outcome: TaskOutcome::Success,
                                executed_at: chrono::Utc::now(),
                                turns_used: turns,
                                tokens_used: tokens,
                                downstream_tasks: vec![],
                            };
                            evolution_loop.record_execution(execution).await;
                        }
                    }
                    ExecutionEvent::TaskFailed { task_id, error, retry_count } => {
                        let _ = swarm_event_tx.send(SwarmEvent::TaskFailed {
                            task_id,
                            error,
                            retry_count,
                        }).await;
                    }
                    ExecutionEvent::TaskRetrying { task_id, attempt, max_attempts } => {
                        let _ = swarm_event_tx.send(SwarmEvent::TaskRetrying {
                            task_id,
                            attempt,
                            max_attempts,
                        }).await;
                    }
                    ExecutionEvent::WaveCompleted { wave_number, succeeded, failed } => {
                        audit_log.info(
                            AuditCategory::Execution,
                            AuditAction::WaveCompleted,
                            format!("Wave {} completed: {} succeeded, {} failed", wave_number, succeeded, failed),
                        ).await;
                    }
                    ExecutionEvent::Completed { status, results } => {
                        audit_log.info(
                            AuditCategory::Execution,
                            AuditAction::WaveCompleted,
                            format!(
                                "DAG execution completed: {:?}, {}/{} tasks succeeded",
                                status, results.completed_tasks, results.total_tasks
                            ),
                        ).await;
                    }
                    ExecutionEvent::RestructureDecision { task_id, decision } => {
                        audit_log.log(
                            AuditEntry::new(
                                AuditLevel::Info,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!("DAG restructure triggered for task {}: {}", task_id, decision),
                            )
                            .with_entity(task_id, "task"),
                        ).await;

                        // Emit swarm event for restructure
                        let _ = swarm_event_tx.send(SwarmEvent::RestructureTriggered {
                            task_id,
                            decision,
                        }).await;
                    }
                }
            }
        });

        // Execute the DAG
        let mut results = executor.execute_with_events(&dag, exec_event_tx).await?;

        // Wait for event forwarder to finish
        let _ = event_forwarder.await;

        // Post-execution verification: verify completed tasks and update state on failures
        if self.config.verify_on_completion {
            let mut verification_failures = 0;

            for task_result in &results.task_results {
                if task_result.status == TaskStatus::Complete {
                    // Verify the task
                    match self.verify_task(task_result.task_id).await {
                        Ok(Some(verification)) if !verification.passed => {
                            // Verification failed - update task status
                            if let Ok(Some(mut task)) = self.task_repo.get(task_result.task_id).await {
                                if task.transition_to(TaskStatus::Failed).is_ok() {
                                    let _ = self.task_repo.update(&task).await;
                                }
                            }
                            verification_failures += 1;

                            // Record as goal violation in evolution loop
                            if self.config.track_evolution {
                                let execution = TaskExecution {
                                    task_id: task_result.task_id,
                                    template_name: task_result.session.as_ref()
                                        .map(|s| s.agent_template.clone())
                                        .unwrap_or_else(|| "unknown".to_string()),
                                    template_version: 1,
                                    outcome: TaskOutcome::GoalViolation,
                                    executed_at: chrono::Utc::now(),
                                    turns_used: task_result.session.as_ref()
                                        .map(|s| s.turns_completed)
                                        .unwrap_or(0),
                                    tokens_used: task_result.session.as_ref()
                                        .map(|s| s.total_tokens())
                                        .unwrap_or(0),
                                    downstream_tasks: vec![],
                                };
                                self.evolution_loop.record_execution(execution).await;
                            }

                            // Emit verification event
                            let checks_total = verification.checks.len();
                            let checks_passed = verification.checks.iter().filter(|c| c.passed).count();
                            let _ = event_tx.send(SwarmEvent::TaskVerified {
                                task_id: task_result.task_id,
                                passed: false,
                                checks_passed,
                                checks_total,
                            }).await;

                            self.audit_log.log(
                                AuditEntry::new(
                                    AuditLevel::Warning,
                                    AuditCategory::Task,
                                    AuditAction::TaskFailed,
                                    AuditActor::System,
                                    format!(
                                        "Task {} failed verification: {}/{} checks passed. {}",
                                        task_result.task_id, checks_passed, checks_total,
                                        verification.failures_summary.unwrap_or_default()
                                    ),
                                )
                                .with_entity(task_result.task_id, "task"),
                            ).await;
                        }
                        Ok(Some(verification)) => {
                            // Verification passed
                            let checks_total = verification.checks.len();
                            let checks_passed = verification.checks.iter().filter(|c| c.passed).count();
                            let _ = event_tx.send(SwarmEvent::TaskVerified {
                                task_id: task_result.task_id,
                                passed: true,
                                checks_passed,
                                checks_total,
                            }).await;
                        }
                        _ => {}
                    }

                    // Also check goal alignment
                    if let Some(ref alignment_svc) = self.goal_alignment {
                        if let Ok(Some(task)) = self.task_repo.get(task_result.task_id).await {
                            if let Ok(eval) = alignment_svc.evaluate_task(&task).await {
                                let _ = event_tx.send(SwarmEvent::GoalAlignmentEvaluated {
                                    task_id: task_result.task_id,
                                    overall_score: eval.overall_score,
                                    passes: eval.passes,
                                }).await;

                                if !eval.passes {
                                    self.audit_log.log(
                                        AuditEntry::new(
                                            AuditLevel::Warning,
                                            AuditCategory::Goal,
                                            AuditAction::GoalFailed,
                                            AuditActor::System,
                                            format!(
                                                "Task {} has low goal alignment: {:.0}% ({}/{}). {}",
                                                task_result.task_id,
                                                eval.overall_score * 100.0,
                                                eval.goals_satisfied,
                                                eval.goal_alignments.len(),
                                                eval.summary
                                            ),
                                        )
                                        .with_entity(task_result.task_id, "task"),
                                    ).await;
                                }
                            }
                        }
                    }
                }
            }

            // Update results to reflect verification failures
            if verification_failures > 0 {
                results.completed_tasks = results.completed_tasks.saturating_sub(verification_failures);
                results.failed_tasks += verification_failures;
            }
        }

        // Persist successful task outputs to memory for future agent reference
        if let Some(ref memory_repo) = self.memory_repo {
            use crate::domain::models::Memory;

            for task_result in &results.task_results {
                if task_result.status == TaskStatus::Complete {
                    if let Some(ref session) = task_result.session {
                        if let Some(ref result_text) = session.result {
                            // Store task output as episodic memory for future agent reference
                            let task = self.task_repo.get(task_result.task_id).await
                                .ok()
                                .flatten();

                            let key = format!("task/{}/output", task_result.task_id);
                            let namespace = task.as_ref()
                                .and_then(|t| t.goal_id.map(|g| format!("goal/{}", g)))
                                .unwrap_or_else(|| "tasks".to_string());

                            let content = format!(
                                "Task: {}\nAgent: {}\nOutput:\n{}",
                                task.as_ref().map(|t| t.title.as_str()).unwrap_or("Unknown"),
                                session.agent_template,
                                result_text
                            );

                            let memory = Memory::episodic(key, content)
                                .with_namespace(namespace)
                                .with_type(crate::domain::models::MemoryType::Context);

                            if let Err(e) = memory_repo.store(&memory).await {
                                tracing::warn!(
                                    "Failed to persist task {} output to memory: {}",
                                    task_result.task_id, e
                                );
                            }
                        }
                    }
                }
            }
        }

        // Update goal status based on results
        if let Ok(Some(mut goal)) = self.goal_repo.get(goal_id).await {
            if results.failed_tasks == 0 {
                goal.complete();
                let _ = self.goal_repo.update(&goal).await;
                let _ = event_tx.send(SwarmEvent::GoalCompleted { goal_id }).await;
            } else if results.completed_tasks == 0 {
                let error_msg = format!("{} tasks failed", results.failed_tasks);
                goal.fail(&error_msg);
                let _ = self.goal_repo.update(&goal).await;
                let _ = event_tx.send(SwarmEvent::GoalFailed {
                    goal_id,
                    error: error_msg,
                }).await;
            }
        }

        Ok(results)
    }

    /// Queue a completed task for merge via the two-stage merge queue.
    ///
    /// Stage 1: Agent worktree -> task integration branch
    /// Stage 2: Task integration branch -> main (with verification)
    pub async fn queue_task_for_merge(
        &self,
        task_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        if !self.config.use_merge_queue {
            return Ok(());
        }

        // Get the worktree for this task
        let worktree = self.worktree_repo.get_by_task(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        // Create the verifier needed by MergeQueue
        let verifier = IntegrationVerifierService::new(
            self.task_repo.clone(),
            self.goal_repo.clone(),
            self.worktree_repo.clone(),
            VerifierConfig::default(),
        );

        // Create merge queue with config
        let merge_config = MergeQueueConfig {
            repo_path: self.config.repo_path.to_str().unwrap_or(".").to_string(),
            main_branch: self.config.default_base_ref.clone(),
            require_verification: self.config.verify_on_completion,
            ..Default::default()
        };

        let merge_queue = MergeQueue::new(
            self.task_repo.clone(),
            self.worktree_repo.clone(),
            Arc::new(verifier),
            merge_config,
        );

        // Queue Stage 1: Agent worktree -> task branch
        let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
            task_id,
            stage: "AgentToTask".to_string(),
        }).await;

        match merge_queue.queue_stage1(
            task_id,
            &worktree.branch,
            &format!("task/{}", task_id),
        ).await {
            Ok(_) => {
                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!("Task {} queued for stage 1 merge", task_id),
                ).await;
            }
            Err(e) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Error,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} failed to queue for stage 1: {}", task_id, e),
                    )
                    .with_entity(task_id, "task"),
                ).await;
                return Err(e);
            }
        }

        // Process the queued merge
        match merge_queue.process_next().await {
            Ok(Some(result)) if result.success => {
                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!(
                        "Task {} stage 1 merge completed: {}",
                        task_id, result.commit_sha.clone().unwrap_or_default()
                    ),
                ).await;

                // Queue stage 2
                let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
                    task_id,
                    stage: "TaskToMain".to_string(),
                }).await;

                if let Ok(_) = merge_queue.queue_stage2(task_id).await {
                    // Process stage 2
                    if let Ok(Some(result2)) = merge_queue.process_next().await {
                        if result2.success {
                            let _ = event_tx.send(SwarmEvent::TaskMerged {
                                task_id,
                                commit_sha: result2.commit_sha.clone().unwrap_or_default(),
                            }).await;

                            self.audit_log.info(
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                format!(
                                    "Task {} stage 2 merge completed: {}",
                                    task_id, result2.commit_sha.unwrap_or_default()
                                ),
                            ).await;
                        }
                    }
                }
            }
            Ok(Some(result)) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!(
                            "Task {} stage 1 merge failed: {}",
                            task_id, result.error.unwrap_or_default()
                        ),
                    )
                    .with_entity(task_id, "task"),
                ).await;
            }
            Ok(None) => {
                // No queued merge to process
            }
            Err(e) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Error,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} merge error: {}", task_id, e),
                    )
                    .with_entity(task_id, "task"),
                ).await;
            }
        }

        Ok(())
    }

    /// Evaluate goal alignment for a completed task.
    ///
    /// Returns the evaluation result if goal alignment service is configured.
    pub async fn evaluate_goal_alignment(&self, task_id: Uuid) -> DomainResult<Option<HolisticEvaluation>> {
        let Some(ref alignment_service) = self.goal_alignment else {
            return Ok(None);
        };

        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let evaluation = alignment_service.evaluate_task(&task).await?;

        // Log alignment result
        if evaluation.passes {
            self.audit_log.info(
                AuditCategory::Goal,
                AuditAction::GoalEvaluated,
                format!(
                    "Task {} aligned with goals: {:.0}% ({}/{} goals satisfied)",
                    task_id, evaluation.overall_score * 100.0,
                    evaluation.goals_satisfied, evaluation.goal_alignments.len()
                ),
            ).await;
        } else {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Goal,
                    AuditAction::GoalEvaluated,
                    AuditActor::System,
                    format!(
                        "Task {} misaligned with goals: {:.0}% - {}",
                        task_id, evaluation.overall_score * 100.0, evaluation.summary
                    ),
                )
                .with_entity(task_id, "task"),
            ).await;
        }

        Ok(Some(evaluation))
    }

    /// Refresh the cache of active goals for context injection.
    async fn refresh_active_goals_cache(&self) -> DomainResult<()> {
        use crate::domain::ports::GoalFilter;
        let goals = self.goal_repo.list(GoalFilter {
            status: Some(GoalStatus::Active),
            ..Default::default()
        }).await?;
        let mut cache = self.active_goals_cache.write().await;
        *cache = goals;
        Ok(())
    }

    /// Build goal context string for agent prompts.
    async fn build_goal_context(&self) -> String {
        let goals = self.active_goals_cache.read().await;
        if goals.is_empty() {
            return String::new();
        }

        let mut context = String::from("\n## Active Goals Context\n\n");
        context.push_str("Your work must align with these active goals:\n\n");

        for goal in goals.iter() {
            context.push_str(&format!("### {} (Priority: {:?})\n", goal.name, goal.priority));
            context.push_str(&format!("{}\n", goal.description));

            if !goal.constraints.is_empty() {
                context.push_str("\n**Constraints:**\n");
                for constraint in &goal.constraints {
                    context.push_str(&format!("- {}: {}\n", constraint.name, constraint.description));
                }
            }
            context.push('\n');
        }

        context.push_str("Ensure your implementation satisfies all constraints and contributes to these goals.\n");
        context
    }

    /// Run cold start analysis if memory is empty.
    pub async fn cold_start(&self) -> DomainResult<Option<ColdStartReport>>
    where
        M: MemoryRepository + Send + Sync + 'static,
    {
        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(None);
        };

        // Create memory service
        let memory_service = MemoryService::new(memory_repo.clone());

        // Check if we have any existing memories
        let stats = memory_service.get_stats().await?;
        let total_memories = stats.total();
        if total_memories > 0 {
            self.audit_log.info(
                AuditCategory::System,
                AuditAction::SwarmStarted,
                format!("Skipping cold start - {} existing memories found", total_memories),
            ).await;
            return Ok(None);
        }

        // Run cold start
        self.audit_log.info(
            AuditCategory::System,
            AuditAction::SwarmStarted,
            "Running cold start analysis...",
        ).await;

        let cold_start_service = ColdStartService::new(
            memory_service,
            ColdStartConfig {
                project_root: self.config.repo_path.clone(),
                ..Default::default()
            },
        );

        let report = cold_start_service.gather_context().await?;

        self.audit_log.info(
            AuditCategory::Memory,
            AuditAction::MemoryStored,
            format!(
                "Cold start complete: {} memories created, project type: {}",
                report.memories_created, report.project_type
            ),
        ).await;

        Ok(Some(report))
    }

    /// Start the memory decay daemon.
    pub async fn start_decay_daemon(&self) -> DomainResult<()>
    where
        M: MemoryRepository + Send + Sync + 'static,
    {
        let Some(ref memory_repo) = self.memory_repo else {
            return Ok(());
        };

        let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
        let daemon = MemoryDecayDaemon::new(memory_service, DecayDaemonConfig::default());

        // Get the handle before running
        let handle = daemon.handle();

        // Store the handle
        {
            let mut daemon_handle = self.decay_daemon_handle.write().await;
            *daemon_handle = Some(handle);
        }

        // Run daemon and log events in background
        let audit_log = self.audit_log.clone();
        tokio::spawn(async move {
            let mut event_rx = daemon.run().await;
            while let Some(event) = event_rx.recv().await {
                match event {
                    crate::services::DecayDaemonEvent::Started => {
                        audit_log.info(
                            AuditCategory::System,
                            AuditAction::SwarmStarted,
                            "Memory decay daemon started",
                        ).await;
                    }
                    crate::services::DecayDaemonEvent::MaintenanceCompleted { run_number, report, .. } => {
                        audit_log.info(
                            AuditCategory::Memory,
                            AuditAction::MemoryPruned,
                            format!(
                                "Memory maintenance #{}: {} expired, {} decayed, {} promoted",
                                run_number, report.expired_pruned, report.decayed_pruned, report.promoted
                            ),
                        ).await;
                    }
                    crate::services::DecayDaemonEvent::Stopped { reason } => {
                        audit_log.info(
                            AuditCategory::System,
                            AuditAction::SwarmStopped,
                            format!("Memory decay daemon stopped: {:?}", reason),
                        ).await;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    /// Stop the memory decay daemon.
    pub async fn stop_decay_daemon(&self) {
        let daemon_handle = self.decay_daemon_handle.read().await;
        if let Some(ref handle) = *daemon_handle {
            handle.stop();
        }
    }

    /// Start the orchestrator and run the main loop.
    pub async fn run(&self, event_tx: mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        {
            let mut status = self.status.write().await;
            *status = OrchestratorStatus::Running;
        }
        let _ = event_tx.send(SwarmEvent::Started).await;

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

        // Seed baseline specialist templates if they don't exist
        {
            use crate::services::AgentService;
            let agent_service = AgentService::new(self.agent_repo.clone());
            match agent_service.seed_baseline_specialists().await {
                Ok(seeded) if !seeded.is_empty() => {
                    self.audit_log.info(
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        format!("Seeded {} baseline specialist templates: {}", seeded.len(), seeded.join(", ")),
                    ).await;
                }
                Ok(_) => {
                    // All specialists already exist
                }
                Err(e) => {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!("Failed to seed specialist templates (non-fatal): {}", e),
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

            // Process A2A delegation requests from agents
            if self.config.mcp_servers.a2a_gateway.is_some() {
                self.process_a2a_delegations(&event_tx).await?;
            }

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

        let _ = event_tx.send(SwarmEvent::Stopped).await;
        Ok(())
    }

    /// Update task readiness based on dependency completion.
    async fn update_task_readiness(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Get all pending tasks
        let pending_tasks = self.task_repo.list_by_status(TaskStatus::Pending).await?;

        for task in pending_tasks {
            // Check if any dependencies have permanently failed
            if self.has_failed_dependencies(&task).await? {
                // Transition to Blocked since upstream failed
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Blocked).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskFailed {
                        task_id: task.id,
                        error: "Upstream dependency failed".to_string(),
                        retry_count: 0,
                    }).await;
                }
            } else if self.are_dependencies_met(&task).await? {
                // Transition to Ready
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskReady {
                        task_id: task.id,
                        task_title: task.title.clone(),
                    }).await;
                }
            }
        }

        // Check for blocked tasks that can become ready (after upstream completion)
        let blocked_tasks = self.task_repo.list_by_status(TaskStatus::Blocked).await?;

        for task in blocked_tasks {
            // Skip if dependencies still failing
            if self.has_failed_dependencies(&task).await? {
                continue;
            }

            if self.are_dependencies_met(&task).await? {
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskReady {
                        task_id: task.id,
                        task_title: task.title.clone(),
                    }).await;
                }
            }
        }

        // Also check Ready tasks - they may need to be blocked if a dependency failed
        let ready_tasks = self.task_repo.list_by_status(TaskStatus::Ready).await?;

        for task in ready_tasks {
            if self.has_failed_dependencies(&task).await? {
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Blocked).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskFailed {
                        task_id: task.id,
                        error: "Upstream dependency failed".to_string(),
                        retry_count: 0,
                    }).await;
                }
            }
        }

        Ok(())
    }

    /// Check if all dependencies for a task are complete.
    async fn are_dependencies_met(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(true);
        }

        let dependencies = self.task_repo.get_dependencies(task.id).await?;

        // All dependencies must be complete
        Ok(dependencies.iter().all(|dep| dep.status == TaskStatus::Complete))
    }

    /// Check if any dependencies failed (would block this task).
    async fn has_failed_dependencies(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(false);
        }

        let dependencies = self.task_repo.get_dependencies(task.id).await?;
        Ok(dependencies.iter().any(|dep| dep.status == TaskStatus::Failed))
    }

    /// Process all active goals.
    async fn process_goals(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Get active goals
        let goals = self.goal_repo.list(crate::domain::ports::GoalFilter {
            status: Some(GoalStatus::Active),
            ..Default::default()
        }).await?;

        for goal in goals {
            self.process_goal(&goal, event_tx).await?;
        }

        Ok(())
    }

    /// Process a single goal.
    async fn process_goal(&self, goal: &Goal, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Get tasks for this goal
        let tasks = self.task_repo.list_by_goal(goal.id).await?;

        // If no tasks exist, this goal needs decomposition
        if tasks.is_empty() {
            let _ = event_tx.send(SwarmEvent::GoalStarted {
                goal_id: goal.id,
                goal_name: goal.name.clone(),
            }).await;

            // Use MetaPlanner to decompose goal into tasks
            let task_count = self.decompose_goal_with_meta_planner(goal, event_tx).await?;

            let _ = event_tx.send(SwarmEvent::GoalDecomposed {
                goal_id: goal.id,
                task_count,
            }).await;
            return Ok(());
        }

        // Get ready tasks
        let ready_tasks: Vec<_> = tasks.iter()
            .filter(|t| t.status == TaskStatus::Ready)
            .collect();

        // Spawn agents for ready tasks
        for task in ready_tasks {
            // Check circuit breaker for this goal's task chain
            let scope = CircuitScope::task_chain(goal.id);
            let check_result = self.circuit_breaker.check(scope.clone()).await;

            if check_result.is_blocked() {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Execution,
                        AuditAction::CircuitBreakerTriggered,
                        AuditActor::System,
                        format!("Task {} blocked by circuit breaker for goal {}", task.id, goal.id),
                    )
                    .with_entity(task.id, "task"),
                ).await;
                continue;
            }

            // Pre-execution constraint validation
            if let Some(ref alignment_service) = self.goal_alignment {
                match alignment_service.evaluate_task(task).await {
                    Ok(evaluation) => {
                        // Check for constraint violations before execution
                        for alignment in &evaluation.goal_alignments {
                            if !alignment.constraints_satisfied {
                                for violation in &alignment.violations {
                                    self.audit_log.log(
                                        AuditEntry::new(
                                            AuditLevel::Warning,
                                            AuditCategory::Goal,
                                            AuditAction::GoalEvaluated,
                                            AuditActor::System,
                                            format!(
                                                "Task {} may violate constraint '{}': {} (severity: {:.0}%)",
                                                task.id,
                                                violation.constraint_name,
                                                violation.description,
                                                violation.severity * 100.0
                                            ),
                                        )
                                        .with_entity(task.id, "task"),
                                    ).await;
                                }
                            }
                        }

                        // Emit alignment evaluation event
                        let _ = event_tx.send(SwarmEvent::GoalAlignmentEvaluated {
                            task_id: task.id,
                            overall_score: evaluation.overall_score,
                            passes: evaluation.passes,
                        }).await;
                    }
                    Err(e) => {
                        // Log but don't block execution on evaluation failure
                        self.audit_log.log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Goal,
                                AuditAction::GoalEvaluated,
                                AuditActor::System,
                                format!("Failed to evaluate task {} alignment: {}", task.id, e),
                            )
                            .with_entity(task.id, "task"),
                        ).await;
                    }
                }
            }

            // Try to acquire agent permit
            if let Ok(permit) = self.agent_semaphore.clone().try_acquire_owned() {
                // Get agent template for system prompt
                let agent_type = task.agent_type.clone().unwrap_or_else(|| "default".to_string());
                let system_prompt = self.get_agent_system_prompt(&agent_type).await;

                // Register agent capabilities with A2A gateway if configured
                if self.config.mcp_servers.a2a_gateway.is_some() {
                    // Get capabilities from agent template
                    let capabilities = match self.agent_repo.get_template_by_name(&agent_type).await {
                        Ok(Some(template)) => {
                            template.tools.iter()
                                .map(|t| t.name.clone())
                                .collect()
                        }
                        _ => vec!["task-execution".to_string()],
                    };

                    if let Err(e) = self.register_agent_capabilities(&agent_type, capabilities).await {
                        tracing::warn!("Failed to register agent '{}' capabilities: {}", agent_type, e);
                    }
                }

                let _ = event_tx.send(SwarmEvent::TaskSpawned {
                    task_id: task.id,
                    task_title: task.title.clone(),
                    agent_type: task.agent_type.clone(),
                }).await;

                // Create worktree if configured
                let worktree_path = if self.config.use_worktrees {
                    match self.create_worktree_for_task(task.id, event_tx).await {
                        Ok(path) => Some(path),
                        Err(e) => {
                            tracing::warn!("Failed to create worktree for task {}: {}", task.id, e);
                            None
                        }
                    }
                } else {
                    None
                };

                // Spawn task execution
                let task_id = task.id;
                let goal_id = goal.id;
                let task_description = task.description.clone();
                let substrate = self.substrate.clone();
                let task_repo = self.task_repo.clone();
                let goal_repo = self.goal_repo.clone();
                let worktree_repo = self.worktree_repo.clone();
                let event_tx = event_tx.clone();
                let max_turns = self.config.default_max_turns;
                let total_tokens = self.total_tokens.clone();
                let use_worktrees = self.config.use_worktrees;
                let circuit_breaker = self.circuit_breaker.clone();
                let audit_log = self.audit_log.clone();
                let evolution_loop = self.evolution_loop.clone();
                let track_evolution = self.config.track_evolution;
                let agent_type_for_evolution = agent_type.clone();
                let mcp_servers = self.config.mcp_servers.clone();
                // Configuration for post-completion workflow
                let verify_on_completion = self.config.verify_on_completion;
                let use_merge_queue = self.config.use_merge_queue;
                let repo_path = self.config.repo_path.clone();
                let default_base_ref = self.config.default_base_ref.clone();

                tokio::spawn(async move {
                    let _permit = permit;

                    // Update task to running
                    if let Ok(Some(mut running_task)) = task_repo.get(task_id).await {
                        let _ = running_task.transition_to(TaskStatus::Running);
                        let _ = task_repo.update(&running_task).await;
                    }

                    // Build substrate request with MCP servers for agent access to system services
                    let mut config = SubstrateConfig::default().with_max_turns(max_turns);
                    if let Some(ref wt_path) = worktree_path {
                        config = config.with_working_dir(wt_path);
                    }

                    // Add MCP servers so agents can access memory, tasks, and A2A
                    if let Some(ref memory_server) = mcp_servers.memory_server {
                        config = config.with_mcp_server(memory_server);
                    }
                    if let Some(ref tasks_server) = mcp_servers.tasks_server {
                        config = config.with_mcp_server(tasks_server);
                    }
                    if let Some(ref a2a_gateway) = mcp_servers.a2a_gateway {
                        config = config.with_mcp_server(a2a_gateway);
                    }

                    let request = SubstrateRequest::new(
                        task_id,
                        &agent_type,
                        &system_prompt,
                        &task_description,
                    ).with_config(config);

                    let result = substrate.execute(request).await;

                    // Update task based on result
                    if let Ok(Some(mut completed_task)) = task_repo.get(task_id).await {
                        match result {
                            Ok(session) if session.status == SessionStatus::Completed => {
                                let tokens = session.total_tokens();
                                let turns = session.turns_completed;
                                total_tokens.fetch_add(tokens, Ordering::Relaxed);

                                let _ = completed_task.transition_to(TaskStatus::Complete);
                                let _ = task_repo.update(&completed_task).await;

                                // Record success with circuit breaker
                                circuit_breaker.record_success(CircuitScope::task_chain(goal_id)).await;

                                // Record success in evolution loop for template improvement
                                if track_evolution {
                                    let execution = TaskExecution {
                                        task_id,
                                        template_name: agent_type_for_evolution.clone(),
                                        template_version: 1, // Would come from agent repo
                                        outcome: TaskOutcome::Success,
                                        executed_at: chrono::Utc::now(),
                                        turns_used: turns,
                                        tokens_used: tokens,
                                        downstream_tasks: vec![],
                                    };
                                    evolution_loop.record_execution(execution).await;
                                }

                                // Log task completion
                                audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Info,
                                        AuditCategory::Task,
                                        AuditAction::TaskCompleted,
                                        AuditActor::System,
                                        format!("Task completed: {} tokens used, {} turns", tokens, turns),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Mark worktree as completed and create artifact reference
                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.complete();
                                        let _ = worktree_repo.update(&wt).await;

                                        // Create artifact reference for downstream tasks
                                        // This enables lineage tracking and worktree-based handoffs
                                        if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                                            let artifact = crate::domain::models::ArtifactRef {
                                                uri: format!("worktree://{}/{}", task_id, wt.branch),
                                                artifact_type: crate::domain::models::ArtifactType::Code,
                                                checksum: wt.merge_commit.clone(),
                                            };
                                            task.artifacts.push(artifact);
                                            task.worktree_path = Some(wt.path.clone());
                                            let _ = task_repo.update(&task).await;
                                        }
                                    }
                                }

                                let _ = event_tx.send(SwarmEvent::TaskCompleted {
                                    task_id,
                                    tokens_used: tokens,
                                }).await;

                                // Run post-completion workflow: verify and merge
                                if verify_on_completion || use_merge_queue {
                                    let workflow_result = run_post_completion_workflow(
                                        task_id,
                                        task_repo.clone(),
                                        goal_repo.clone(),
                                        worktree_repo.clone(),
                                        &event_tx,
                                        &audit_log,
                                        verify_on_completion,
                                        use_merge_queue,
                                        &repo_path,
                                        &default_base_ref,
                                    ).await;

                                    if let Err(e) = workflow_result {
                                        audit_log.log(
                                            AuditEntry::new(
                                                AuditLevel::Warning,
                                                AuditCategory::Task,
                                                AuditAction::TaskFailed,
                                                AuditActor::System,
                                                format!("Post-completion workflow error for task {}: {}", task_id, e),
                                            )
                                            .with_entity(task_id, "task"),
                                        ).await;
                                    }
                                }

                                // Evaluate evolution loop for potential refinements
                                if track_evolution {
                                    let events = evolution_loop.evaluate().await;
                                    for event in events {
                                        // Check if this event is for our agent type
                                        if event.template_name == agent_type_for_evolution {
                                            audit_log.log(
                                                AuditEntry::new(
                                                    AuditLevel::Info,
                                                    AuditCategory::Agent,
                                                    AuditAction::AgentSpawned,
                                                    AuditActor::System,
                                                    format!(
                                                        "Evolution triggered for '{}': {:?} (success rate: {:.0}%)",
                                                        event.template_name,
                                                        event.trigger,
                                                        event.stats_at_trigger.success_rate * 100.0
                                                    ),
                                                ),
                                            ).await;

                                            let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                                                template_name: event.template_name.clone(),
                                                trigger: format!("{:?}", event.trigger),
                                            }).await;
                                        }
                                    }
                                }
                            }
                            Ok(session) => {
                                let tokens = session.total_tokens();
                                let turns = session.turns_completed;
                                total_tokens.fetch_add(tokens, Ordering::Relaxed);

                                let error_msg = session.error.clone().unwrap_or_else(|| "Unknown error".to_string());

                                completed_task.retry_count += 1;
                                let _ = completed_task.transition_to(TaskStatus::Failed);
                                let _ = task_repo.update(&completed_task).await;

                                // Record failure with circuit breaker
                                circuit_breaker.record_failure(
                                    CircuitScope::task_chain(goal_id),
                                    &error_msg,
                                ).await;

                                // Record failure in evolution loop for template improvement
                                if track_evolution {
                                    let execution = TaskExecution {
                                        task_id,
                                        template_name: agent_type_for_evolution.clone(),
                                        template_version: 1,
                                        outcome: TaskOutcome::Failure,
                                        executed_at: chrono::Utc::now(),
                                        turns_used: turns,
                                        tokens_used: tokens,
                                        downstream_tasks: vec![],
                                    };
                                    evolution_loop.record_execution(execution).await;
                                }

                                // Log task failure
                                audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        format!("Task failed: {}", error_msg),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Mark worktree as failed
                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(error_msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }
                                }

                                let _ = event_tx.send(SwarmEvent::TaskFailed {
                                    task_id,
                                    error: error_msg,
                                    retry_count: completed_task.retry_count,
                                }).await;
                            }
                            Err(e) => {
                                let error_msg = e.to_string();

                                completed_task.retry_count += 1;
                                let _ = completed_task.transition_to(TaskStatus::Failed);
                                let _ = task_repo.update(&completed_task).await;

                                // Record failure with circuit breaker
                                circuit_breaker.record_failure(
                                    CircuitScope::task_chain(goal_id),
                                    &error_msg,
                                ).await;

                                // Record failure in evolution loop for template improvement
                                if track_evolution {
                                    let execution = TaskExecution {
                                        task_id,
                                        template_name: agent_type_for_evolution.clone(),
                                        template_version: 1,
                                        outcome: TaskOutcome::Failure,
                                        executed_at: chrono::Utc::now(),
                                        turns_used: 0,
                                        tokens_used: 0,
                                        downstream_tasks: vec![],
                                    };
                                    evolution_loop.record_execution(execution).await;
                                }

                                // Log task failure
                                audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Error,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        format!("Task execution error: {}", error_msg),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Mark worktree as failed
                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(error_msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }
                                }

                                let _ = event_tx.send(SwarmEvent::TaskFailed {
                                    task_id,
                                    error: error_msg,
                                    retry_count: completed_task.retry_count,
                                }).await;
                            }
                        }
                    }
                });
            }
        }

        // Check if goal is complete
        let all_complete = tasks.iter().all(|t| t.status == TaskStatus::Complete);
        let permanently_failed = tasks.iter().any(|t| {
            t.status == TaskStatus::Failed && t.retry_count >= self.config.max_task_retries
        });

        if all_complete {
            let mut updated_goal = goal.clone();
            updated_goal.complete();
            self.goal_repo.update(&updated_goal).await?;
            let _ = event_tx.send(SwarmEvent::GoalCompleted { goal_id: goal.id }).await;
        } else if permanently_failed {
            let mut updated_goal = goal.clone();
            updated_goal.fail("One or more tasks permanently failed");
            self.goal_repo.update(&updated_goal).await?;
            let _ = event_tx.send(SwarmEvent::GoalFailed {
                goal_id: goal.id,
                error: "Task failures exceeded retry limit".to_string(),
            }).await;
        }

        Ok(())
    }

    /// Process retry logic for failed tasks.
    async fn process_retries(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let failed_tasks = self.task_repo.list_by_status(TaskStatus::Failed).await?;

        for task in failed_tasks {
            // Check if we should retry
            if task.retry_count < self.config.max_task_retries {
                // Check if dependencies are still met (they might have changed)
                let deps_met = self.are_dependencies_met(&task).await?;
                let deps_failed = self.has_failed_dependencies(&task).await?;

                if deps_failed {
                    // Mark as blocked - can't retry until upstream is fixed
                    let mut blocked_task = task.clone();
                    let _ = blocked_task.transition_to(TaskStatus::Blocked);
                    self.task_repo.update(&blocked_task).await?;
                } else if deps_met {
                    // Transition back to Ready for retry
                    let mut retry_task = task.clone();
                    if retry_task.transition_to(TaskStatus::Ready).is_ok() {
                        self.task_repo.update(&retry_task).await?;
                        let _ = event_tx.send(SwarmEvent::TaskRetrying {
                            task_id: task.id,
                            attempt: task.retry_count + 1,
                            max_attempts: self.config.max_task_retries,
                        }).await;
                    }
                }
            }
        }

        Ok(())
    }

    /// Process pending evolution refinement requests.
    ///
    /// Checks for agent templates that need refinement and uses MetaPlanner
    /// to create improved versions based on failure patterns.
    async fn process_evolution_refinements(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // First, evaluate all templates to detect any that need refinement
        // This checks success rates, goal violations, and regression patterns
        let evolution_events = self.evolution_loop.evaluate().await;

        // Emit events for any evolution triggers detected
        for event in evolution_events {
            let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                template_name: event.template_name.clone(),
                trigger: format!("{:?}", event.trigger),
            }).await;

            self.audit_log.info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned,
                format!(
                    "Evolution triggered for '{}': {:?} (success rate: {:.0}%)",
                    event.template_name,
                    event.trigger,
                    event.stats_at_trigger.success_rate * 100.0
                ),
            ).await;
        }

        // Get pending refinement requests from evolution loop
        let pending_refinements = self.evolution_loop.get_pending_refinements().await;

        for request in pending_refinements {
            // Mark as in progress
            if !self.evolution_loop.start_refinement(request.id).await {
                continue; // Already being processed
            }

            // Log the refinement attempt
            self.audit_log.info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned,
                format!(
                    "Processing evolution refinement for '{}': {:?}",
                    request.template_name, request.severity
                ),
            ).await;

            // Get the current agent template
            let template = match self.agent_repo.get_template_by_name(&request.template_name).await {
                Ok(Some(t)) => t,
                _ => {
                    self.evolution_loop.complete_refinement(request.id, false).await;
                    continue;
                }
            };

            // Create a refined version based on the failure patterns
            let refined_prompt = format!(
                "{}\n\n## Refinement Notes (v{})\n\n\
                Based on {} recent executions with {:.0}% success rate.\n\
                Trigger: {:?}\n\
                {} failed tasks tracked.\n\n\
                Please pay special attention to:\n\
                - Careful validation of inputs and outputs\n\
                - Handling edge cases gracefully\n\
                - Clear error reporting for debugging",
                template.system_prompt,
                template.version + 1,
                request.stats.total_tasks,
                request.stats.success_rate * 100.0,
                request.trigger,
                request.failed_task_ids.len()
            );

            // Create new version of the template
            let mut new_template = template.clone();
            new_template.version += 1;
            new_template.system_prompt = refined_prompt;

            match self.agent_repo.update_template(&new_template).await {
                Ok(_) => {
                    // Record version change for regression detection
                    self.evolution_loop.record_version_change(
                        &request.template_name,
                        new_template.version,
                    ).await;

                    // Complete the refinement
                    self.evolution_loop.complete_refinement(request.id, true).await;

                    let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                        template_name: request.template_name.clone(),
                        trigger: format!("Refined to v{}", new_template.version),
                    }).await;

                    self.audit_log.info(
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        format!(
                            "Agent '{}' refined to version {}",
                            request.template_name, new_template.version
                        ),
                    ).await;
                }
                Err(e) => {
                    self.evolution_loop.complete_refinement(request.id, false).await;
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!(
                                "Failed to refine agent '{}': {}",
                                request.template_name, e
                            ),
                        ),
                    ).await;
                }
            }
        }

        Ok(())
    }

    /// Process specialist agent triggers.
    ///
    /// Checks for conditions that should spawn specialist agents:
    /// - DAG restructuring for recoverable failures → New decomposition/alternative path
    /// - Merge conflicts → Merge Conflict Specialist
    /// - Persistent failures (max retries exceeded, restructuring exhausted) → Diagnostic Analyst
    async fn process_specialist_triggers(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Check for persistent failures that need restructuring or diagnostic analysis
        let failed_tasks = self.task_repo.list_by_status(TaskStatus::Failed).await?;
        let permanently_failed: Vec<_> = failed_tasks
            .iter()
            .filter(|t| t.retry_count >= self.config.max_task_retries)
            .collect();

        for task in permanently_failed {
            // Skip tasks without a goal_id - can't restructure or diagnose without goal context
            let Some(goal_id) = task.goal_id else {
                continue;
            };

            // Get the goal for context
            let goal = match self.goal_repo.get(goal_id).await? {
                Some(g) => g,
                None => continue,
            };

            // First, try DAG restructuring before falling back to diagnostic analyst
            let restructure_result = self.try_restructure_for_failure(task, &goal, event_tx).await;

            match restructure_result {
                Ok(true) => {
                    // Restructuring was applied, skip diagnostic analyst
                    continue;
                }
                Ok(false) => {
                    // Restructuring not possible (exhausted or not eligible), fall through to diagnostic
                }
                Err(e) => {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Restructure attempt failed for task {}: {}", task.id, e),
                        )
                        .with_entity(task.id, "task"),
                    ).await;
                    // Fall through to diagnostic analyst
                }
            }

            // Check if we haven't already created a diagnostic task for this failure
            let diagnostic_exists = self.task_repo
                .list_by_goal(goal_id)
                .await?
                .iter()
                .any(|t| t.title.contains("Diagnostic:") && t.title.contains(&task.id.to_string()[..8]));

            if !diagnostic_exists {
                // Spawn Diagnostic Analyst specialist
                if let Err(e) = self.spawn_specialist_for_failure(task, goal_id, event_tx).await {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!("Failed to spawn diagnostic specialist for task {}: {}", task.id, e),
                        )
                        .with_entity(task.id, "task"),
                    ).await;
                }
            }
        }

        // Check for merge conflicts needing specialist resolution
        // This is done via the merge queue's conflict detection
        if self.config.use_merge_queue {
            if let Err(e) = self.process_merge_conflict_specialists(event_tx).await {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        AuditActor::System,
                        format!("Failed to process merge conflict specialists: {}", e),
                    ),
                ).await;
            }
        }

        Ok(())
    }

    /// Try to restructure the DAG for a permanently failed task.
    /// Returns Ok(true) if restructuring was applied, Ok(false) if not possible/exhausted.
    async fn try_restructure_for_failure(
        &self,
        failed_task: &Task,
        goal: &Goal,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<bool> {
        let trigger = RestructureTrigger::PermanentFailure {
            task_id: failed_task.id,
            retries_exhausted: failed_task.retry_count,
        };

        // Check if restructuring should be attempted
        let mut restructure_svc = self.restructure_service.lock().await;

        if !restructure_svc.should_restructure(&trigger) {
            return Ok(false);
        }

        // Get related failures in the same goal
        let goal_tasks = self.task_repo.list_by_goal(goal.id).await?;
        let related_failures: Vec<Task> = goal_tasks
            .into_iter()
            .filter(|t| t.status == TaskStatus::Failed && t.id != failed_task.id)
            .collect();

        // Build restructure context
        let context = RestructureContext {
            goal: goal.clone(),
            failed_task: failed_task.clone(),
            failure_reason: format!("Task failed after {} retries", failed_task.retry_count),
            previous_attempts: vec![], // Would need to track this in task metadata
            related_failures,
            available_approaches: vec![], // Could be populated from agent templates
            attempt_number: restructure_svc.attempt_count(failed_task.id) + 1,
            time_since_last: None,
        };

        // Get restructure decision
        let decision = restructure_svc.analyze_and_decide(&context)?;

        // Log the decision
        self.audit_log.info(
            AuditCategory::Task,
            AuditAction::TaskCreated,
            format!(
                "DAG restructure decision for task {}: {:?}",
                failed_task.id, decision
            ),
        ).await;

        // Emit event
        let _ = event_tx.send(SwarmEvent::RestructureTriggered {
            task_id: failed_task.id,
            decision: format!("{:?}", decision),
        }).await;

        // Apply the decision
        match decision {
            RestructureDecision::RetryDifferentApproach { new_approach, new_agent_type } => {
                // Update the failed task to retry with different approach
                let mut updated_task = failed_task.clone();
                updated_task.description = format!(
                    "{}\n\n## Restructure Note\nPrevious approach failed. Try: {}",
                    updated_task.description, new_approach
                );
                if let Some(agent_type) = new_agent_type {
                    updated_task.agent_type = Some(agent_type);
                }
                // Reset retry count and transition to Ready
                updated_task.retry_count = 0;
                let _ = updated_task.transition_to(TaskStatus::Ready);
                self.task_repo.update(&updated_task).await?;
                Ok(true)
            }
            RestructureDecision::DecomposeDifferently { new_subtasks, remove_original } => {
                // Create new subtasks based on the restructure plan
                for spec in &new_subtasks {
                    let priority = match spec.priority {
                        TaskPriorityModifier::Same => failed_task.priority.clone(),
                        TaskPriorityModifier::Higher => crate::domain::models::TaskPriority::High,
                        TaskPriorityModifier::Lower => crate::domain::models::TaskPriority::Low,
                    };

                    let new_task = Task::new(&spec.title, &spec.description)
                        .with_goal(goal.id)
                        .with_priority(priority);

                    if let Some(ref agent_type) = spec.agent_type {
                        let new_task = new_task.with_agent(agent_type);
                        if new_task.validate().is_ok() {
                            self.task_repo.create(&new_task).await?;
                            let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                                task_id: new_task.id,
                                task_title: new_task.title.clone(),
                                goal_id: goal.id,
                            }).await;
                        }
                    } else if new_task.validate().is_ok() {
                        self.task_repo.create(&new_task).await?;
                        let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                            task_id: new_task.id,
                            task_title: new_task.title.clone(),
                            goal_id: goal.id,
                        }).await;
                    }
                }

                // Cancel the original task if specified
                if remove_original {
                    let mut canceled_task = failed_task.clone();
                    let _ = canceled_task.transition_to(TaskStatus::Canceled);
                    self.task_repo.update(&canceled_task).await?;
                }

                Ok(true)
            }
            RestructureDecision::AlternativePath { description, new_tasks } => {
                // Create alternative path tasks
                for spec in &new_tasks {
                    let priority = match spec.priority {
                        TaskPriorityModifier::Same => failed_task.priority.clone(),
                        TaskPriorityModifier::Higher => crate::domain::models::TaskPriority::High,
                        TaskPriorityModifier::Lower => crate::domain::models::TaskPriority::Low,
                    };

                    let new_task = Task::new(&spec.title, &spec.description)
                        .with_goal(goal.id)
                        .with_priority(priority);

                    if let Some(ref agent_type) = spec.agent_type {
                        let new_task = new_task.with_agent(agent_type);
                        if new_task.validate().is_ok() {
                            self.task_repo.create(&new_task).await?;
                            let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                                task_id: new_task.id,
                                task_title: new_task.title.clone(),
                                goal_id: goal.id,
                            }).await;
                        }
                    } else if new_task.validate().is_ok() {
                        self.task_repo.create(&new_task).await?;
                        let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                            task_id: new_task.id,
                            task_title: new_task.title.clone(),
                            goal_id: goal.id,
                        }).await;
                    }
                }

                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCreated,
                    format!("Created alternative path: {}", description),
                ).await;

                Ok(true)
            }
            RestructureDecision::WaitAndRetry { delay, reason } => {
                // For now, just log and return false - we don't have a scheduler
                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskFailed,
                    format!("Restructure suggests waiting {} seconds: {}", delay.as_secs(), reason),
                ).await;
                Ok(false)
            }
            RestructureDecision::Escalate { reason, context } => {
                // Escalation means we should fall through to diagnostic analyst
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} escalated: {} - {}", failed_task.id, reason, context),
                    )
                    .with_entity(failed_task.id, "task"),
                ).await;
                Ok(false)
            }
            RestructureDecision::AcceptFailure { reason } => {
                // No recovery possible
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Error,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} failure accepted: {}", failed_task.id, reason),
                    )
                    .with_entity(failed_task.id, "task"),
                ).await;
                Ok(false)
            }
        }
    }

    /// Spawn a diagnostic analyst for a permanently failed task.
    async fn spawn_specialist_for_failure(
        &self,
        failed_task: &Task,
        goal_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        // Create a diagnostic task that will be handled by the Diagnostic Analyst
        let diagnostic_task = Task::new(
            &format!("Diagnostic: Investigate failure of task {}", &failed_task.id.to_string()[..8]),
            &format!(
                "The following task has permanently failed after {} retries:\n\n\
                Title: {}\n\
                Description: {}\n\n\
                Please investigate the root cause of failure and suggest remediation.\n\
                Consider:\n\
                - Are the task requirements achievable?\n\
                - Are there missing dependencies or prerequisites?\n\
                - Is the agent type appropriate for this task?\n\
                - Are there external blockers (permissions, resources, etc.)?",
                failed_task.retry_count,
                failed_task.title,
                failed_task.description
            ),
        )
        .with_goal(goal_id)
        .with_agent("diagnostic-analyst");

        diagnostic_task.validate().map_err(DomainError::ValidationFailed)?;
        self.task_repo.create(&diagnostic_task).await?;

        let _ = event_tx.send(SwarmEvent::SpecialistSpawned {
            specialist_type: "diagnostic-analyst".to_string(),
            trigger: format!("Task {} permanently failed", failed_task.id),
            task_id: Some(diagnostic_task.id),
        }).await;

        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Spawned Diagnostic Analyst for permanently failed task {}",
                failed_task.id
            ),
        ).await;

        Ok(())
    }

    /// Process merge conflicts and spawn conflict resolution specialists.
    async fn process_merge_conflict_specialists(
        &self,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        // Create a temporary verifier for the merge queue
        let verifier = IntegrationVerifierService::new(
            self.task_repo.clone(),
            self.goal_repo.clone(),
            self.worktree_repo.clone(),
            VerifierConfig::default(),
        );

        let merge_config = MergeQueueConfig {
            repo_path: self.config.repo_path.to_str().unwrap_or(".").to_string(),
            main_branch: self.config.default_base_ref.clone(),
            require_verification: self.config.verify_on_completion,
            route_conflicts_to_specialist: true,
            ..Default::default()
        };

        let merge_queue = MergeQueue::new(
            self.task_repo.clone(),
            self.worktree_repo.clone(),
            Arc::new(verifier),
            merge_config,
        );

        // Get conflicts needing resolution
        let conflicts = merge_queue.get_conflicts_needing_resolution().await;

        for conflict in conflicts {
            // Check if we haven't already created a resolution task for this conflict
            let resolution_exists = self.task_repo
                .list_by_goal(conflict.task_id)
                .await
                .map(|tasks| {
                    tasks.iter().any(|t| {
                        t.title.contains("Resolve merge conflict") &&
                        t.title.contains(&conflict.source_branch)
                    })
                })
                .unwrap_or(false);

            if !resolution_exists {
                // Get the goal_id for this task - skip if task has no goal
                if let Ok(Some(task)) = self.task_repo.get(conflict.task_id).await {
                    let Some(goal_id) = task.goal_id else {
                        continue;
                    };

                    let resolution_task = Task::new(
                        &format!("Resolve merge conflict: {} → {}", conflict.source_branch, conflict.target_branch),
                        &format!(
                            "A merge conflict was detected when trying to merge branch '{}' into '{}'.\n\n\
                            Conflicting files:\n{}\n\n\
                            Working directory: {}\n\n\
                            Please resolve the conflicts by:\n\
                            1. Analyzing the conflicting changes\n\
                            2. Understanding the intent of each change\n\
                            3. Merging the changes in a way that preserves both intents\n\
                            4. Testing the merged result\n\
                            5. Completing the merge commit",
                            conflict.source_branch,
                            conflict.target_branch,
                            conflict.conflict_files.iter()
                                .map(|f| format!("  - {}", f))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            conflict.workdir
                        ),
                    )
                    .with_goal(goal_id)
                    .with_agent("merge-conflict-specialist");

                    if resolution_task.validate().is_ok() {
                        if let Ok(()) = self.task_repo.create(&resolution_task).await {
                            let _ = event_tx.send(SwarmEvent::SpecialistSpawned {
                                specialist_type: "merge-conflict-specialist".to_string(),
                                trigger: format!("Merge conflict in {} files", conflict.conflict_files.len()),
                                task_id: Some(resolution_task.id),
                            }).await;

                            self.audit_log.info(
                                AuditCategory::Agent,
                                AuditAction::AgentSpawned,
                                format!(
                                    "Spawned Merge Conflict Specialist for {} → {}",
                                    conflict.source_branch, conflict.target_branch
                                ),
                            ).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Process A2A delegation requests from agents.
    ///
    /// Polls the A2A gateway for pending delegation messages and creates
    /// corresponding tasks for the target agents.
    async fn process_a2a_delegations(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let Some(ref a2a_url) = self.config.mcp_servers.a2a_gateway else {
            return Ok(());
        };

        // Poll A2A gateway for pending delegation messages
        // Using HTTP GET to fetch pending delegations
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/delegations/pending", a2a_url);

        let response = match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) => resp,
            Err(e) => {
                // Non-fatal: A2A gateway may not be running or reachable
                tracing::debug!("Failed to poll A2A gateway for delegations: {}", e);
                return Ok(());
            }
        };

        if !response.status().is_success() {
            return Ok(());
        }

        // Parse pending delegations
        #[derive(serde::Deserialize)]
        struct PendingDelegation {
            id: Uuid,
            sender_id: String,
            target_agent: String,
            task_description: String,
            parent_task_id: Option<Uuid>,
            goal_id: Option<Uuid>,
            priority: String,
        }

        let delegations: Vec<PendingDelegation> = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("Failed to parse A2A delegations: {}", e);
                return Ok(());
            }
        };

        for delegation in delegations {
            // Create a new task for the delegated work
            let priority = match delegation.priority.to_lowercase().as_str() {
                "critical" => crate::domain::models::TaskPriority::Critical,
                "high" => crate::domain::models::TaskPriority::High,
                "low" => crate::domain::models::TaskPriority::Low,
                _ => crate::domain::models::TaskPriority::Normal,
            };

            let mut task = Task::new(
                &format!("Delegated: {}", &delegation.task_description.chars().take(50).collect::<String>()),
                &format!(
                    "## A2A Delegation\n\n\
                    Delegated by: {}\n\n\
                    ## Task\n\n{}",
                    delegation.sender_id,
                    delegation.task_description
                ),
            )
            .with_priority(priority)
            .with_agent(&delegation.target_agent);

            if let Some(goal_id) = delegation.goal_id {
                task = task.with_goal(goal_id);
            }

            if let Some(parent_id) = delegation.parent_task_id {
                task.parent_id = Some(parent_id);
            }

            if task.validate().is_ok() {
                if let Ok(()) = self.task_repo.create(&task).await {
                    let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                        task_id: task.id,
                        task_title: task.title.clone(),
                        goal_id: delegation.goal_id.unwrap_or(Uuid::nil()),
                    }).await;

                    self.audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCreated,
                        format!(
                            "Created delegated task {} for agent '{}' (from: {})",
                            task.id, delegation.target_agent, delegation.sender_id
                        ),
                    ).await;

                    // Acknowledge the delegation in A2A gateway
                    let ack_url = format!("{}/api/v1/delegations/{}/ack", a2a_url, delegation.id);
                    let _ = client.post(&ack_url).send().await;
                }
            }
        }

        Ok(())
    }

    /// Decompose a goal into tasks using MetaPlanner.
    ///
    /// Uses LLM decomposition if configured, otherwise falls back to heuristic decomposition.
    async fn decompose_goal_with_meta_planner(&self, goal: &Goal, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<usize> {
        // Create MetaPlanner with current configuration
        let meta_planner_config = MetaPlannerConfig {
            use_llm_decomposition: self.config.use_llm_decomposition,
            max_tasks_per_decomposition: 10,
            auto_generate_agents: true,
            ..Default::default()
        };

        let mut meta_planner = MetaPlanner::new(
            self.goal_repo.clone(),
            self.task_repo.clone(),
            self.agent_repo.clone(),
            meta_planner_config,
        );

        // Wire memory repository for pattern queries during decomposition
        if let Some(ref memory_repo) = self.memory_repo {
            meta_planner = meta_planner.with_memory_repo(memory_repo.clone() as Arc<dyn MemoryRepository>);
        }

        // Decompose the goal into tasks
        let plan = meta_planner.decompose_goal(goal.id).await?;

        // Log the decomposition
        self.audit_log.info(
            AuditCategory::Task,
            AuditAction::TaskCreated,
            format!(
                "Goal '{}' decomposed into {} tasks (complexity: {:?})",
                goal.name, plan.tasks.len(), plan.estimated_complexity
            ),
        ).await;

        // Execute the plan - create the tasks
        let created_tasks = meta_planner.execute_plan(&plan).await?;
        let task_count = created_tasks.len();

        // Emit TaskSubmitted events for each created task
        for task in &created_tasks {
            let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                task_id: task.id,
                task_title: task.title.clone(),
                goal_id: goal.id,
            }).await;
        }

        // Ensure required agents exist (capability-driven agent genesis)
        for agent_type in &plan.required_agents {
            // Check if agent already exists
            let exists = meta_planner.agent_exists(agent_type).await.unwrap_or(false);

            if !exists {
                let purpose = format!("Execute tasks for goal: {}", goal.name);
                match meta_planner.ensure_agent(agent_type, &purpose).await {
                    Ok(agent) => {
                        // Emit event for dynamically created agent
                        let _ = event_tx.send(SwarmEvent::AgentCreated {
                            agent_type: agent_type.clone(),
                            tier: format!("{:?}", agent.tier),
                        }).await;

                        self.audit_log.info(
                            AuditCategory::Agent,
                            AuditAction::TemplateCreated,
                            format!(
                                "Dynamically created agent '{}' for goal '{}'",
                                agent_type, goal.name
                            ),
                        ).await;
                    }
                    Err(e) => {
                        self.audit_log.log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Agent,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!("Could not ensure agent '{}': {}", agent_type, e),
                            ),
                        ).await;
                    }
                }
            }
        }

        Ok(task_count)
    }

    /// Basic goal decomposition (creates a single task).
    /// Fallback when MetaPlanner is unavailable.
    #[allow(dead_code)]
    async fn decompose_goal_basic(&self, goal: &Goal) -> DomainResult<usize> {
        // Create a single task for the goal
        let task = Task::new(
            &format!("Implement: {}", goal.name),
            &goal.description,
        )
        .with_goal(goal.id)
        .with_priority(match goal.priority {
            crate::domain::models::GoalPriority::Low => crate::domain::models::TaskPriority::Low,
            crate::domain::models::GoalPriority::Normal => crate::domain::models::TaskPriority::Normal,
            crate::domain::models::GoalPriority::High => crate::domain::models::TaskPriority::High,
            crate::domain::models::GoalPriority::Critical => crate::domain::models::TaskPriority::Critical,
        });

        task.validate().map_err(DomainError::ValidationFailed)?;
        self.task_repo.create(&task).await?;

        Ok(1)
    }

    /// Get the system prompt for an agent type, including goal context.
    async fn get_agent_system_prompt(&self, agent_type: &str) -> String {
        let base_prompt = match self.agent_repo.get_template_by_name(agent_type).await {
            Ok(Some(template)) => template.system_prompt.clone(),
            _ => {
                // Default system prompt if agent template not found
                format!(
                    "You are a specialized agent for executing tasks.\n\
                    Follow the task description carefully and complete the work.\n\
                    Agent type: {}",
                    agent_type
                )
            }
        };

        // Append goal context to the system prompt
        let goal_context = self.build_goal_context().await;
        if goal_context.is_empty() {
            base_prompt
        } else {
            format!("{}\n\n{}", base_prompt, goal_context)
        }
    }

    /// Create a worktree for task execution.
    async fn create_worktree_for_task(
        &self,
        task_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<String> {
        let worktree_config = WorktreeConfig {
            base_path: self.config.worktree_base_path.clone(),
            repo_path: self.config.repo_path.clone(),
            default_base_ref: self.config.default_base_ref.clone(),
            auto_cleanup: true,
        };

        let worktree_service = WorktreeService::new(
            self.worktree_repo.clone(),
            worktree_config,
        );

        let worktree = worktree_service.create_worktree(task_id, None).await?;

        let _ = event_tx.send(SwarmEvent::WorktreeCreated {
            task_id,
            path: worktree.path.clone(),
        }).await;

        Ok(worktree.path)
    }

    /// Update statistics.
    async fn update_stats(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let task_counts = self.task_repo.count_by_status().await?;
        let active_worktrees = self.worktree_repo.list_active().await?.len();

        let stats = SwarmStats {
            active_goals: self.goal_repo.list(crate::domain::ports::GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            }).await?.len(),
            pending_tasks: *task_counts.get(&TaskStatus::Pending).unwrap_or(&0) as usize,
            ready_tasks: *task_counts.get(&TaskStatus::Ready).unwrap_or(&0) as usize,
            running_tasks: *task_counts.get(&TaskStatus::Running).unwrap_or(&0) as usize,
            completed_tasks: *task_counts.get(&TaskStatus::Complete).unwrap_or(&0) as usize,
            failed_tasks: *task_counts.get(&TaskStatus::Failed).unwrap_or(&0) as usize,
            active_agents: self.config.max_agents - self.agent_semaphore.available_permits(),
            active_worktrees,
            total_tokens_used: self.total_tokens.load(Ordering::Relaxed),
        };

        {
            let mut s = self.stats.write().await;
            *s = stats.clone();
        }

        let _ = event_tx.send(SwarmEvent::StatusUpdate(stats)).await;
        Ok(())
    }

    /// Get current status.
    pub async fn status(&self) -> OrchestratorStatus {
        self.status.read().await.clone()
    }

    /// Get current stats.
    pub async fn stats(&self) -> SwarmStats {
        self.stats.read().await.clone()
    }

    /// Pause the orchestrator.
    pub async fn pause(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Running {
            *status = OrchestratorStatus::Paused;
        }
    }

    /// Resume the orchestrator.
    pub async fn resume(&self) {
        let mut status = self.status.write().await;
        if *status == OrchestratorStatus::Paused {
            *status = OrchestratorStatus::Running;
        }
    }

    /// Stop the orchestrator gracefully.
    pub async fn stop(&self) {
        let mut status = self.status.write().await;
        *status = OrchestratorStatus::ShuttingDown;
    }

    /// Execute a specific goal with its task DAG.
    pub async fn execute_goal(&self, goal_id: Uuid) -> DomainResult<ExecutionResults> {
        let goal = self.goal_repo.get(goal_id).await?
            .ok_or(DomainError::GoalNotFound(goal_id))?;

        let tasks = self.task_repo.list_by_goal(goal_id).await?;
        let dag = TaskDag::from_tasks(tasks);

        // Fetch project context from memory if available
        let project_context = if let Some(ref memory_repo) = self.memory_repo {
            let memory_service = MemoryService::new(memory_repo.clone());
            if let Ok(memories) = memory_service.search("architecture project", Some("semantic"), 5).await {
                if memories.is_empty() {
                    None
                } else {
                    let context_parts: Vec<String> = memories.iter()
                        .map(|m| format!("- {}: {}", m.key, m.content))
                        .collect();
                    Some(format!("Relevant project knowledge:\n{}", context_parts.join("\n")))
                }
            } else {
                None
            }
        } else {
            None
        };

        let executor_config = ExecutorConfig {
            max_concurrency: self.config.max_agents,
            task_timeout_secs: self.config.goal_timeout_secs,
            max_retries: self.config.max_task_retries,
            default_max_turns: self.config.default_max_turns,
            fail_fast: false,
            memory_server_url: self.config.mcp_servers.memory_server.clone(),
            a2a_gateway_url: self.config.mcp_servers.a2a_gateway.clone(),
            tasks_server_url: self.config.mcp_servers.tasks_server.clone(),
            project_context,
        };

        let executor = DagExecutor::new(
            self.task_repo.clone(),
            self.agent_repo.clone(),
            self.substrate.clone(),
            executor_config,
        ).with_goal_repo(self.goal_repo.clone());

        let results = executor.execute(&dag).await?;

        // Track tokens
        for task_result in &results.task_results {
            if let Some(ref session) = task_result.session {
                self.total_tokens.fetch_add(session.total_tokens(), Ordering::Relaxed);
            }
        }

        // Update goal status based on results
        let mut updated_goal = goal;
        if results.status() == ExecutionStatus::Completed {
            updated_goal.complete();
        } else if results.failed_tasks > 0 {
            updated_goal.fail("Some tasks failed");
        }
        self.goal_repo.update(&updated_goal).await?;

        Ok(results)
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

        // Update stats
        self.update_stats(&tx).await?;

        Ok(self.stats().await)
    }

    /// Get total tokens used.
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }
}

/// Helper function to run post-completion workflow (verification and merging).
/// This is called from spawned tasks after successful task completion.
async fn run_post_completion_workflow<G, T, W>(
    task_id: Uuid,
    task_repo: Arc<T>,
    goal_repo: Arc<G>,
    worktree_repo: Arc<W>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    audit_log: &Arc<AuditLogService>,
    verify_on_completion: bool,
    use_merge_queue: bool,
    repo_path: &std::path::Path,
    default_base_ref: &str,
) -> DomainResult<()>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    // Step 1: Run integration verification if enabled
    let verification_passed = if verify_on_completion {
        let verifier = IntegrationVerifierService::new(
            task_repo.clone(),
            goal_repo.clone(),
            worktree_repo.clone(),
            VerifierConfig::default(),
        );

        match verifier.verify_task(task_id).await {
            Ok(result) => {
                let checks_total = result.checks.len();
                let checks_passed = result.checks.iter().filter(|c| c.passed).count();

                let _ = event_tx.send(SwarmEvent::TaskVerified {
                    task_id,
                    passed: result.passed,
                    checks_passed,
                    checks_total,
                }).await;

                if result.passed {
                    audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCompleted,
                        format!(
                            "Task {} passed verification: {}/{} checks",
                            task_id, checks_passed, checks_total
                        ),
                    ).await;
                } else {
                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!(
                                "Task {} failed verification: {}",
                                task_id, result.failures_summary.clone().unwrap_or_default()
                            ),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                }

                result.passed
            }
            Err(e) => {
                audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} verification error: {}", task_id, e),
                    )
                    .with_entity(task_id, "task"),
                ).await;
                false
            }
        }
    } else {
        true // Skip verification, assume passed
    };

    // Step 2: Queue for merge if verification passed and merge queue is enabled
    if verification_passed && use_merge_queue {
        // Get the worktree for this task
        if let Ok(Some(worktree)) = worktree_repo.get_by_task(task_id).await {
            let verifier = IntegrationVerifierService::new(
                task_repo.clone(),
                goal_repo.clone(),
                worktree_repo.clone(),
                VerifierConfig::default(),
            );

            let merge_config = MergeQueueConfig {
                repo_path: repo_path.to_str().unwrap_or(".").to_string(),
                main_branch: default_base_ref.to_string(),
                require_verification: verify_on_completion,
                ..Default::default()
            };

            let merge_queue = MergeQueue::new(
                task_repo.clone(),
                worktree_repo.clone(),
                Arc::new(verifier),
                merge_config,
            );

            // Queue Stage 1: Agent worktree -> task branch
            let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
                task_id,
                stage: "AgentToTask".to_string(),
            }).await;

            match merge_queue.queue_stage1(
                task_id,
                &worktree.branch,
                &format!("task/{}", task_id),
            ).await {
                Ok(_) => {
                    audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCompleted,
                        format!("Task {} queued for stage 1 merge", task_id),
                    ).await;

                    // Process the queued merge
                    if let Ok(Some(result)) = merge_queue.process_next().await {
                        if result.success {
                            // Queue stage 2
                            let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
                                task_id,
                                stage: "TaskToMain".to_string(),
                            }).await;

                            if let Ok(_) = merge_queue.queue_stage2(task_id).await {
                                if let Ok(Some(result2)) = merge_queue.process_next().await {
                                    if result2.success {
                                        let _ = event_tx.send(SwarmEvent::TaskMerged {
                                            task_id,
                                            commit_sha: result2.commit_sha.clone().unwrap_or_default(),
                                        }).await;

                                        audit_log.info(
                                            AuditCategory::Task,
                                            AuditAction::TaskCompleted,
                                            format!(
                                                "Task {} merged to main: {}",
                                                task_id, result2.commit_sha.unwrap_or_default()
                                            ),
                                        ).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Task {} failed to queue for merge: {}", task_id, e),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                }
            }
        }
    }

    // Step 3: Evaluate goal alignment
    // This is handled separately in the orchestrator's goal completion check

    Ok(())
}

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

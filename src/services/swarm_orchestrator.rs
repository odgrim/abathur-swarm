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
    DagExecutor, DecayDaemonConfig, DaemonHandle, ExecutionResults, ExecutionStatus, ExecutorConfig,
    MemoryDecayDaemon, MemoryService,
    WorktreeConfig, WorktreeService,
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
        }
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
            if self.are_dependencies_met(&task).await? {
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

            // Use meta-planner to decompose goal (basic implementation)
            // In a full implementation, this would use the MetaPlanner service with LLM
            self.decompose_goal_basic(goal, event_tx).await?;
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

            // Try to acquire agent permit
            if let Ok(permit) = self.agent_semaphore.clone().try_acquire_owned() {
                // Get agent template for system prompt
                let agent_type = task.agent_type.clone().unwrap_or_else(|| "default".to_string());
                let system_prompt = self.get_agent_system_prompt(&agent_type).await;

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
                let worktree_repo = self.worktree_repo.clone();
                let event_tx = event_tx.clone();
                let max_turns = self.config.default_max_turns;
                let total_tokens = self.total_tokens.clone();
                let use_worktrees = self.config.use_worktrees;
                let circuit_breaker = self.circuit_breaker.clone();
                let audit_log = self.audit_log.clone();

                tokio::spawn(async move {
                    let _permit = permit;

                    // Update task to running
                    if let Ok(Some(mut running_task)) = task_repo.get(task_id).await {
                        let _ = running_task.transition_to(TaskStatus::Running);
                        let _ = task_repo.update(&running_task).await;
                    }

                    // Build substrate request
                    let mut config = SubstrateConfig::default().with_max_turns(max_turns);
                    if let Some(ref wt_path) = worktree_path {
                        config = config.with_working_dir(wt_path);
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
                                total_tokens.fetch_add(tokens, Ordering::Relaxed);

                                let _ = completed_task.transition_to(TaskStatus::Complete);
                                let _ = task_repo.update(&completed_task).await;

                                // Record success with circuit breaker
                                circuit_breaker.record_success(CircuitScope::task_chain(goal_id)).await;

                                // Log task completion
                                audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Info,
                                        AuditCategory::Task,
                                        AuditAction::TaskCompleted,
                                        AuditActor::System,
                                        format!("Task completed: {} tokens used", tokens),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Mark worktree as completed
                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.complete();
                                        let _ = worktree_repo.update(&wt).await;
                                    }
                                }

                                let _ = event_tx.send(SwarmEvent::TaskCompleted {
                                    task_id,
                                    tokens_used: tokens,
                                }).await;
                            }
                            Ok(session) => {
                                let tokens = session.total_tokens();
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

    /// Basic goal decomposition (creates a single task).
    /// In a full implementation, this would use the MetaPlanner with LLM.
    async fn decompose_goal_basic(&self, goal: &Goal, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
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

        let _ = event_tx.send(SwarmEvent::GoalDecomposed {
            goal_id: goal.id,
            task_count: 1,
        }).await;

        Ok(())
    }

    /// Get the system prompt for an agent type.
    async fn get_agent_system_prompt(&self, agent_type: &str) -> String {
        match self.agent_repo.get_template_by_name(agent_type).await {
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

        let executor_config = ExecutorConfig {
            max_concurrency: self.config.max_agents,
            task_timeout_secs: self.config.goal_timeout_secs,
            max_retries: self.config.max_task_retries,
            default_max_turns: self.config.default_max_turns,
            fail_fast: false,
        };

        let executor = DagExecutor::new(
            self.task_repo.clone(),
            self.agent_repo.clone(),
            self.substrate.clone(),
            executor_config,
        );

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

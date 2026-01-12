//! DAG Executor service for wave-based parallel task execution.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    Goal, ConstraintType, SessionStatus, SubstrateConfig, SubstrateRequest, SubstrateSession, TaskDag, TaskStatus,
};
use crate::domain::ports::{AgentRepository, GoalRepository, Substrate, TaskRepository};
use crate::services::guardrails::{GuardrailResult, Guardrails};
use crate::services::circuit_breaker::{CircuitBreakerService, CircuitScope};
use crate::services::dag_restructure::DagRestructureService;

/// Configuration for the DAG executor.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Maximum concurrent tasks per wave.
    pub max_concurrency: usize,
    /// Timeout for individual tasks (seconds).
    pub task_timeout_secs: u64,
    /// Maximum retries per task.
    pub max_retries: u32,
    /// Default max turns for substrate invocations.
    pub default_max_turns: u32,
    /// Whether to stop on first failure.
    pub fail_fast: bool,
    /// Memory MCP server URL for agent access.
    pub memory_server_url: Option<String>,
    /// A2A gateway URL for agent-to-agent communication.
    pub a2a_gateway_url: Option<String>,
    /// Tasks MCP server URL for agents to query task state.
    pub tasks_server_url: Option<String>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            task_timeout_secs: 600,
            max_retries: 3,
            default_max_turns: 25,
            fail_fast: false,
            memory_server_url: None,
            a2a_gateway_url: None,
            tasks_server_url: None,
        }
    }
}

/// Status of a DAG execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// Not started.
    Pending,
    /// Currently running.
    Running,
    /// Completed successfully.
    Completed,
    /// Completed with some failures.
    PartialSuccess,
    /// Failed.
    Failed,
    /// Canceled.
    Canceled,
}

/// Result of a single task execution.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub session: Option<SubstrateSession>,
    pub error: Option<String>,
    pub duration_secs: u64,
    pub retry_count: u32,
}

/// Event emitted during execution.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ExecutionEvent {
    /// Execution started.
    Started { total_tasks: usize, wave_count: usize },
    /// Wave started.
    WaveStarted { wave_number: usize, task_count: usize },
    /// Task started.
    TaskStarted { task_id: Uuid, task_title: String },
    /// Task completed.
    TaskCompleted { task_id: Uuid, result: TaskResult },
    /// Task failed.
    TaskFailed { task_id: Uuid, error: String, retry_count: u32 },
    /// Task retrying.
    TaskRetrying { task_id: Uuid, attempt: u32, max_attempts: u32 },
    /// Wave completed.
    WaveCompleted { wave_number: usize, succeeded: usize, failed: usize },
    /// Execution completed.
    Completed { status: ExecutionStatus, results: ExecutionResults },
    /// DAG restructure decision made for a permanently failed task.
    RestructureDecision { task_id: Uuid, decision: String },
}

/// Results of a DAG execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionResults {
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub skipped_tasks: usize,
    pub total_duration_secs: u64,
    pub task_results: Vec<TaskResult>,
    pub total_tokens_used: u64,
}

impl ExecutionResults {
    pub fn success_rate(&self) -> f64 {
        if self.total_tasks == 0 {
            return 0.0;
        }
        self.completed_tasks as f64 / self.total_tasks as f64
    }

    pub fn status(&self) -> ExecutionStatus {
        if self.failed_tasks == 0 && self.skipped_tasks == 0 {
            ExecutionStatus::Completed
        } else if self.completed_tasks > 0 {
            ExecutionStatus::PartialSuccess
        } else {
            ExecutionStatus::Failed
        }
    }
}

/// DAG Executor for running task graphs.
pub struct DagExecutor<T, A, G>
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
    G: GoalRepository + 'static,
{
    task_repo: Arc<T>,
    agent_repo: Arc<A>,
    goal_repo: Option<Arc<G>>,
    substrate: Arc<dyn Substrate>,
    config: ExecutorConfig,
    guardrails: Option<Arc<Guardrails>>,
    circuit_breaker: Option<Arc<CircuitBreakerService>>,
    restructure_service: Option<Arc<DagRestructureService>>,
    /// Active goals cache for injecting constraints into agent context.
    active_goals_cache: Arc<RwLock<Vec<Goal>>>,
    status: Arc<RwLock<ExecutionStatus>>,
    results: Arc<RwLock<ExecutionResults>>,
}

impl<T, A, G> DagExecutor<T, A, G>
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
    G: GoalRepository + 'static,
{
    pub fn new(
        task_repo: Arc<T>,
        agent_repo: Arc<A>,
        substrate: Arc<dyn Substrate>,
        config: ExecutorConfig,
    ) -> Self {
        Self {
            task_repo,
            agent_repo,
            goal_repo: None,
            substrate,
            config,
            guardrails: None,
            circuit_breaker: None,
            restructure_service: None,
            active_goals_cache: Arc::new(RwLock::new(Vec::new())),
            status: Arc::new(RwLock::new(ExecutionStatus::Pending)),
            results: Arc::new(RwLock::new(ExecutionResults::default())),
        }
    }

    /// Add goal repository for constraint injection into agent context.
    pub fn with_goal_repo(mut self, goal_repo: Arc<G>) -> Self {
        self.goal_repo = Some(goal_repo);
        self
    }

    /// Add guardrails to the executor.
    pub fn with_guardrails(mut self, guardrails: Arc<Guardrails>) -> Self {
        self.guardrails = Some(guardrails);
        self
    }

    /// Add circuit breaker to the executor.
    pub fn with_circuit_breaker(mut self, circuit_breaker: Arc<CircuitBreakerService>) -> Self {
        self.circuit_breaker = Some(circuit_breaker);
        self
    }

    /// Add restructure service to the executor for failure recovery.
    pub fn with_restructure_service(mut self, restructure_service: Arc<DagRestructureService>) -> Self {
        self.restructure_service = Some(restructure_service);
        self
    }

    /// Refresh the active goals cache for constraint injection.
    async fn refresh_active_goals_cache(&self) -> DomainResult<()> {
        if let Some(ref goal_repo) = self.goal_repo {
            use crate::domain::ports::GoalFilter;
            use crate::domain::models::GoalStatus;

            let filter = GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            };
            let goals = goal_repo.list(filter).await?;
            let mut cache = self.active_goals_cache.write().await;
            *cache = goals;
        }
        Ok(())
    }

    /// Execute a DAG of tasks.
    pub async fn execute(&self, dag: &TaskDag) -> DomainResult<ExecutionResults> {
        let (tx, _rx) = mpsc::channel(100);
        self.execute_with_events(dag, tx).await
    }

    /// Execute a DAG with event streaming.
    pub async fn execute_with_events(
        &self,
        dag: &TaskDag,
        event_tx: mpsc::Sender<ExecutionEvent>,
    ) -> DomainResult<ExecutionResults> {
        // Refresh active goals cache for constraint injection
        self.refresh_active_goals_cache().await?;

        // Validate and get execution waves
        let waves = dag.execution_waves()
            .map_err(|e| DomainError::ValidationFailed(e.to_string()))?;

        let start_time = std::time::Instant::now();

        // Update status
        {
            let mut status = self.status.write().await;
            *status = ExecutionStatus::Running;
        }

        // Initialize results
        {
            let mut results = self.results.write().await;
            results.total_tasks = dag.nodes.len();
        }

        // Send started event
        let _ = event_tx.send(ExecutionEvent::Started {
            total_tasks: dag.nodes.len(),
            wave_count: waves.len(),
        }).await;

        // Track completed and failed tasks
        let completed: Arc<RwLock<HashSet<Uuid>>> = Arc::new(RwLock::new(HashSet::new()));
        let failed: Arc<RwLock<HashSet<Uuid>>> = Arc::new(RwLock::new(HashSet::new()));
        let total_tokens: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));

        // Execute waves sequentially
        for (wave_idx, wave) in waves.iter().enumerate() {
            let _ = event_tx.send(ExecutionEvent::WaveStarted {
                wave_number: wave_idx + 1,
                task_count: wave.len(),
            }).await;

            // Check for fail-fast abort
            if self.config.fail_fast {
                let failed_count = failed.read().await.len();
                if failed_count > 0 {
                    break;
                }
            }

            // Execute wave tasks in parallel with concurrency limit
            let wave_results = self.execute_wave(wave, dag, &event_tx, &total_tokens, &self.guardrails).await?;

            // Process wave results
            let mut wave_succeeded = 0;
            let mut wave_failed = 0;

            for result in wave_results {
                let mut results = self.results.write().await;
                results.task_results.push(result.clone());

                match result.status {
                    TaskStatus::Complete => {
                        completed.write().await.insert(result.task_id);
                        results.completed_tasks += 1;
                        wave_succeeded += 1;
                    }
                    TaskStatus::Failed => {
                        failed.write().await.insert(result.task_id);
                        results.failed_tasks += 1;
                        wave_failed += 1;
                    }
                    _ => {}
                }
            }

            let _ = event_tx.send(ExecutionEvent::WaveCompleted {
                wave_number: wave_idx + 1,
                succeeded: wave_succeeded,
                failed: wave_failed,
            }).await;

            // Check for permanent failures and signal for restructure if service is available
            if wave_failed > 0 {
                if let Some(ref restructure_svc) = self.restructure_service {
                    // Get permanently failed task IDs from this wave
                    let failed_tasks: Vec<(Uuid, u32)> = {
                        let results = self.results.read().await;
                        results.task_results.iter()
                            .filter(|r| r.status == TaskStatus::Failed && r.retry_count >= self.config.max_retries)
                            .map(|r| (r.task_id, r.retry_count))
                            .collect()
                    };

                    for (task_id, retries) in failed_tasks {
                        // Check if restructure should be attempted
                        let trigger = crate::services::dag_restructure::RestructureTrigger::PermanentFailure {
                            task_id,
                            retries_exhausted: retries,
                        };

                        if restructure_svc.should_restructure(&trigger) {
                            // Emit event to signal that restructure is needed
                            // The actual restructure decision will be made by the orchestrator
                            // which has access to goals and can build the full RestructureContext
                            let _ = event_tx.send(ExecutionEvent::RestructureDecision {
                                task_id,
                                decision: format!("Restructure triggered: {:?}", trigger),
                            }).await;

                            tracing::info!(
                                "DAG restructure triggered for task {}: {:?}",
                                task_id, trigger
                            );
                        }
                    }
                }
            }
        }

        // Finalize results
        let final_results = {
            let mut results = self.results.write().await;
            results.total_duration_secs = start_time.elapsed().as_secs();
            results.total_tokens_used = *total_tokens.read().await;

            // Count skipped (tasks blocked by failed dependencies)
            let completed_count = completed.read().await.len();
            let failed_count = failed.read().await.len();
            results.skipped_tasks = results.total_tasks - completed_count - failed_count;

            results.clone()
        };

        // Update final status
        let final_status = final_results.status();
        {
            let mut status = self.status.write().await;
            *status = final_status.clone();
        }

        let _ = event_tx.send(ExecutionEvent::Completed {
            status: final_status,
            results: final_results.clone(),
        }).await;

        Ok(final_results)
    }

    /// Execute a single wave of tasks in parallel.
    async fn execute_wave(
        &self,
        wave: &[Uuid],
        dag: &TaskDag,
        event_tx: &mpsc::Sender<ExecutionEvent>,
        total_tokens: &Arc<RwLock<u64>>,
        guardrails: &Option<Arc<Guardrails>>,
    ) -> DomainResult<Vec<TaskResult>> {
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let mut handles = vec![];

        // Snapshot goals for this wave
        let active_goals: Vec<Goal> = self.active_goals_cache.read().await.clone();

        for &task_id in wave {
            let node = match dag.nodes.get(&task_id) {
                Some(n) => n.clone(),
                None => continue,
            };

            // Check circuit breaker for this task's goal chain
            if let Some(ref cb) = self.circuit_breaker {
                if let Some(goal_id) = node.goal_id {
                    let check_result = cb.check(CircuitScope::task_chain(goal_id)).await;
                    if check_result.is_blocked() {
                        // Skip this task - circuit is open
                        let _ = event_tx.send(ExecutionEvent::TaskFailed {
                            task_id,
                            error: "Circuit breaker open for goal chain".to_string(),
                            retry_count: 0,
                        }).await;
                        continue;
                    }
                }
            }

            let permit = semaphore.clone().acquire_owned().await
                .map_err(|_| DomainError::ValidationFailed("Semaphore error".to_string()))?;

            let task_repo = self.task_repo.clone();
            let agent_repo = self.agent_repo.clone();
            let substrate = self.substrate.clone();
            let config = self.config.clone();
            let event_tx = event_tx.clone();
            let total_tokens = total_tokens.clone();
            let guardrails = guardrails.clone();
            let circuit_breaker = self.circuit_breaker.clone();
            let goal_id = node.goal_id;
            let goals_for_task = active_goals.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                execute_single_task(
                    task_id,
                    goal_id,
                    task_repo,
                    agent_repo,
                    substrate,
                    config,
                    event_tx,
                    total_tokens,
                    guardrails,
                    circuit_breaker,
                    goals_for_task,
                ).await
            });

            handles.push(handle);
        }

        // Collect results
        let mut results = vec![];
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }

        Ok(results)
    }

    /// Get current execution status.
    pub async fn status(&self) -> ExecutionStatus {
        self.status.read().await.clone()
    }

    /// Get current results (may be partial if still running).
    pub async fn current_results(&self) -> ExecutionResults {
        self.results.read().await.clone()
    }

    /// Cancel execution.
    pub async fn cancel(&self) {
        let mut status = self.status.write().await;
        if *status == ExecutionStatus::Running {
            *status = ExecutionStatus::Canceled;
        }
    }
}

/// Build goal context string for agent system prompt.
fn build_goal_context(goals: &[Goal], task_goal_id: Option<Uuid>) -> String {
    if goals.is_empty() {
        return String::new();
    }

    let mut context = String::from("\n\n## Active Project Goals and Constraints\n\n");
    context.push_str("Your work must align with these goals and respect their constraints:\n\n");

    for goal in goals {
        let is_primary = task_goal_id == Some(goal.id);
        let marker = if is_primary { " [PRIMARY - This task's goal]" } else { "" };

        context.push_str(&format!("### {}{}\n", goal.name, marker));
        context.push_str(&format!("{}\n", goal.description));

        if !goal.constraints.is_empty() {
            context.push_str("\n**Constraints:**\n");
            for constraint in &goal.constraints {
                let severity = match constraint.constraint_type {
                    ConstraintType::Invariant => "MUST",
                    ConstraintType::Preference => "SHOULD",
                    ConstraintType::Boundary => "WITHIN",
                };
                context.push_str(&format!("- {} [{}]: {}\n", constraint.name, severity, constraint.description));
            }
        }
        context.push('\n');
    }

    context.push_str("---\n\n");
    context
}

/// Build MCP context for agent system prompt.
/// Documents available HTTP REST APIs that agents can call via WebFetch.
fn build_mcp_context(config: &ExecutorConfig) -> String {
    let mut context = String::new();

    if config.memory_server_url.is_some() || config.a2a_gateway_url.is_some() || config.tasks_server_url.is_some() {
        context.push_str("\n\n## Available System Services (HTTP REST APIs)\n\n");
        context.push_str("Use the WebFetch tool to interact with these services.\n\n");

        if let Some(ref url) = config.memory_server_url {
            context.push_str(&format!("### Memory Service ({})\n", url));
            context.push_str("Query and store project knowledge, patterns, and decisions.\n\n");
            context.push_str("**Endpoints:**\n");
            context.push_str(&format!("- `GET {}/api/v1/memory?search=<query>` - Search memories\n", url));
            context.push_str(&format!("- `GET {}/api/v1/memory?namespace=<ns>` - List memories in namespace\n", url));
            context.push_str(&format!("- `POST {}/api/v1/memory` - Store new memory (JSON body: {{\"key\": \"...\", \"content\": \"...\", \"namespace\": \"...\"}})\n", url));
            context.push_str(&format!("- `GET {}/api/v1/memory/key/<namespace>/<key>` - Get specific memory\n\n", url));
        }
        if let Some(ref url) = config.tasks_server_url {
            context.push_str(&format!("### Tasks Service ({})\n", url));
            context.push_str("Query task dependencies, status, and spawn subtasks.\n\n");
            context.push_str("**Endpoints:**\n");
            context.push_str(&format!("- `GET {}/api/v1/tasks/<id>` - Get task details\n", url));
            context.push_str(&format!("- `GET {}/api/v1/tasks?status=<status>` - List tasks by status\n", url));
            context.push_str(&format!("- `GET {}/api/v1/tasks/<id>/dependencies` - Get task dependencies\n", url));
            context.push_str(&format!("- `POST {}/api/v1/tasks` - Create subtask (JSON body: {{\"title\": \"...\", \"description\": \"...\", \"parent_id\": \"...\"}})\n\n", url));
        }
        if let Some(ref url) = config.a2a_gateway_url {
            context.push_str(&format!("### A2A Gateway ({})\n", url));
            context.push_str("Delegate work to specialized agents via JSON-RPC 2.0.\n\n");
            context.push_str("**Endpoints:**\n");
            context.push_str(&format!("- `GET {}/api/v1/agents` - List available agents and their capabilities\n", url));
            context.push_str(&format!("- `POST {}` - Send JSON-RPC request (method: \"tasks/send\", params: {{\"message\": {{\"role\": \"user\", \"parts\": [{{\"type\": \"text\", \"text\": \"...\"}}]}}}})\n\n", url));
        }
        context.push_str("---\n\n");
    }

    context
}

/// Execute a single task with retry logic and timeout.
async fn execute_single_task<T, A>(
    task_id: Uuid,
    goal_id: Option<Uuid>,
    task_repo: Arc<T>,
    agent_repo: Arc<A>,
    substrate: Arc<dyn Substrate>,
    config: ExecutorConfig,
    event_tx: mpsc::Sender<ExecutionEvent>,
    total_tokens: Arc<RwLock<u64>>,
    guardrails: Option<Arc<Guardrails>>,
    circuit_breaker: Option<Arc<CircuitBreakerService>>,
    active_goals: Vec<Goal>,
) -> TaskResult
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
{
    let start = std::time::Instant::now();

    // Check guardrails before starting
    if let Some(ref g) = guardrails {
        match g.check_task_start(task_id).await {
            GuardrailResult::Blocked(reason) => {
                return TaskResult {
                    task_id,
                    status: TaskStatus::Failed,
                    session: None,
                    error: Some(format!("Blocked by guardrails: {}", reason)),
                    duration_secs: start.elapsed().as_secs(),
                    retry_count: 0,
                };
            }
            GuardrailResult::Warning(msg) => {
                tracing::warn!("Guardrail warning for task {}: {}", task_id, msg);
            }
            GuardrailResult::Allowed => {}
        }
        g.register_task_start(task_id).await;
    }

    // Get full task from repository
    let task = match task_repo.get(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            if let Some(ref g) = guardrails {
                g.register_task_end(task_id, false).await;
            }
            return TaskResult {
                task_id,
                status: TaskStatus::Failed,
                session: None,
                error: Some("Task not found".to_string()),
                duration_secs: start.elapsed().as_secs(),
                retry_count: 0,
            };
        }
        Err(e) => {
            if let Some(ref g) = guardrails {
                g.register_task_end(task_id, false).await;
            }
            return TaskResult {
                task_id,
                status: TaskStatus::Failed,
                session: None,
                error: Some(e.to_string()),
                duration_secs: start.elapsed().as_secs(),
                retry_count: 0,
            };
        }
    };

    let _ = event_tx.send(ExecutionEvent::TaskStarted {
        task_id,
        task_title: task.title.clone(),
    }).await;

    // Update task status to Running
    let mut running_task = task.clone();
    if running_task.transition_to(TaskStatus::Running).is_ok() {
        let _ = task_repo.update(&running_task).await;
    }

    // Get system prompt from agent template
    let agent_type = task.agent_type.as_deref().unwrap_or("default");
    let base_system_prompt = match agent_repo.get_template_by_name(agent_type).await {
        Ok(Some(template)) => template.system_prompt,
        _ => format!(
            "You are a specialized agent for executing tasks.\n\
            Follow the task description carefully and complete the work.\n\
            Agent type: {}",
            agent_type
        ),
    };

    // Build enhanced system prompt with goal context and MCP services
    let goal_context = build_goal_context(&active_goals, task.goal_id);
    let mcp_context = build_mcp_context(&config);
    let system_prompt = format!("{}{}{}", base_system_prompt, goal_context, mcp_context);

    // Build substrate config with MCP servers if configured
    let mut substrate_config = SubstrateConfig::default().with_max_turns(config.default_max_turns);
    if let Some(ref url) = config.memory_server_url {
        substrate_config = substrate_config.with_mcp_server(url.clone());
    }
    if let Some(ref url) = config.tasks_server_url {
        substrate_config = substrate_config.with_mcp_server(url.clone());
    }
    if let Some(ref url) = config.a2a_gateway_url {
        substrate_config = substrate_config.with_mcp_server(url.clone());
    }

    // Execute with retries
    let mut last_error = None;
    let mut last_session = None;
    let mut retry_count = 0u32;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            retry_count = attempt;
            let _ = event_tx.send(ExecutionEvent::TaskRetrying {
                task_id,
                attempt,
                max_attempts: config.max_retries,
            }).await;

            // Update retry count in task
            let mut retry_task = task.clone();
            retry_task.retry_count = attempt;
            let _ = task_repo.update(&retry_task).await;
        }

        // Build request with enhanced context
        let request = SubstrateRequest::new(
            task_id,
            agent_type,
            &system_prompt,
            &task.description,
        ).with_config(substrate_config.clone());

        // Execute with timeout
        let execution_result = timeout(
            Duration::from_secs(config.task_timeout_secs),
            substrate.execute(request),
        ).await;

        match execution_result {
            Ok(Ok(session)) => {
                // Track tokens
                let tokens_used = session.total_tokens();
                {
                    let mut tokens = total_tokens.write().await;
                    *tokens += tokens_used;
                }

                // Record tokens with guardrails
                if let Some(ref g) = guardrails {
                    g.record_tokens(tokens_used);
                    // Also record cost if available
                    if let Some(cost_cents) = session.cost_cents {
                        g.record_cost(cost_cents);
                    }
                }

                if session.status == SessionStatus::Completed {
                    // Success - update task and return
                    let mut completed_task = task.clone();
                    completed_task.retry_count = retry_count;
                    if completed_task.transition_to(TaskStatus::Complete).is_ok() {
                        let _ = task_repo.update(&completed_task).await;
                    }

                    // Register task end with guardrails
                    if let Some(ref g) = guardrails {
                        g.register_task_end(task_id, true).await;
                    }

                    // Record success with circuit breaker
                    if let (Some(ref cb), Some(gid)) = (&circuit_breaker, goal_id) {
                        cb.record_success(CircuitScope::task_chain(gid)).await;
                    }

                    let result = TaskResult {
                        task_id,
                        status: TaskStatus::Complete,
                        error: None,
                        session: Some(session.clone()),
                        duration_secs: start.elapsed().as_secs(),
                        retry_count,
                    };

                    let _ = event_tx.send(ExecutionEvent::TaskCompleted {
                        task_id,
                        result: result.clone(),
                    }).await;

                    return result;
                } else {
                    // Session didn't complete successfully
                    last_error = session.error.clone().or(Some("Session did not complete".to_string()));
                    last_session = Some(session);
                }
            }
            Ok(Err(e)) => {
                last_error = Some(e.to_string());
            }
            Err(_) => {
                last_error = Some(format!("Task timed out after {} seconds", config.task_timeout_secs));
            }
        }
    }

    // All retries exhausted - mark as failed
    let mut failed_task = task.clone();
    failed_task.retry_count = retry_count;
    if failed_task.transition_to(TaskStatus::Failed).is_ok() {
        let _ = task_repo.update(&failed_task).await;
    }

    // Register task end with guardrails
    if let Some(ref g) = guardrails {
        g.register_task_end(task_id, false).await;
    }

    let error_msg = last_error.unwrap_or_else(|| "Unknown error".to_string());

    // Record failure with circuit breaker
    if let (Some(ref cb), Some(gid)) = (&circuit_breaker, goal_id) {
        cb.record_failure(CircuitScope::task_chain(gid), &error_msg).await;
    }

    let _ = event_tx.send(ExecutionEvent::TaskFailed {
        task_id,
        error: error_msg.clone(),
        retry_count,
    }).await;

    TaskResult {
        task_id,
        status: TaskStatus::Failed,
        session: last_session,
        error: Some(error_msg),
        duration_secs: start.elapsed().as_secs(),
        retry_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_test_pool, SqliteAgentRepository, SqliteGoalRepository, SqliteTaskRepository, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::MockSubstrate;

    async fn setup_executor() -> DagExecutor<SqliteTaskRepository, SqliteAgentRepository, SqliteGoalRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let agent_repo = Arc::new(SqliteAgentRepository::new(pool.clone()));
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool));
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let config = ExecutorConfig::default();

        DagExecutor::new(task_repo, agent_repo, substrate, config)
            .with_goal_repo(goal_repo)
    }

    #[tokio::test]
    async fn test_empty_dag() {
        let executor = setup_executor().await;
        let dag = TaskDag::from_tasks(vec![]);

        let results = executor.execute(&dag).await.unwrap();
        assert_eq!(results.total_tasks, 0);
        assert_eq!(results.status(), ExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn test_executor_config() {
        let config = ExecutorConfig::default();
        assert_eq!(config.max_concurrency, 4);
        assert_eq!(config.max_retries, 3);
        assert!(!config.fail_fast);
    }

    #[tokio::test]
    async fn test_success_rate() {
        let results = ExecutionResults {
            total_tasks: 10,
            completed_tasks: 8,
            failed_tasks: 2,
            skipped_tasks: 0,
            total_duration_secs: 100,
            task_results: vec![],
            total_tokens_used: 1000,
        };
        assert!((results.success_rate() - 0.8).abs() < 0.001);
        assert_eq!(results.status(), ExecutionStatus::PartialSuccess);
    }

    #[tokio::test]
    async fn test_execution_status() {
        let executor = setup_executor().await;
        assert_eq!(executor.status().await, ExecutionStatus::Pending);
    }
}

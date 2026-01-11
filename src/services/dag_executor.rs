//! DAG Executor service for wave-based parallel task execution.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    SessionStatus, SubstrateConfig, SubstrateRequest, SubstrateSession, TaskDag, TaskStatus,
};
use crate::domain::ports::{AgentRepository, Substrate, TaskRepository};
use crate::services::guardrails::{GuardrailResult, Guardrails};

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
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            task_timeout_secs: 600,
            max_retries: 3,
            default_max_turns: 25,
            fail_fast: false,
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
pub struct DagExecutor<T, A>
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
{
    task_repo: Arc<T>,
    agent_repo: Arc<A>,
    substrate: Arc<dyn Substrate>,
    config: ExecutorConfig,
    guardrails: Option<Arc<Guardrails>>,
    status: Arc<RwLock<ExecutionStatus>>,
    results: Arc<RwLock<ExecutionResults>>,
}

impl<T, A> DagExecutor<T, A>
where
    T: TaskRepository + 'static,
    A: AgentRepository + 'static,
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
            substrate,
            config,
            guardrails: None,
            status: Arc::new(RwLock::new(ExecutionStatus::Pending)),
            results: Arc::new(RwLock::new(ExecutionResults::default())),
        }
    }

    /// Add guardrails to the executor.
    pub fn with_guardrails(mut self, guardrails: Arc<Guardrails>) -> Self {
        self.guardrails = Some(guardrails);
        self
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

        for &task_id in wave {
            let _node = match dag.nodes.get(&task_id) {
                Some(n) => n.clone(),
                None => continue,
            };

            let permit = semaphore.clone().acquire_owned().await
                .map_err(|_| DomainError::ValidationFailed("Semaphore error".to_string()))?;

            let task_repo = self.task_repo.clone();
            let agent_repo = self.agent_repo.clone();
            let substrate = self.substrate.clone();
            let config = self.config.clone();
            let event_tx = event_tx.clone();
            let total_tokens = total_tokens.clone();
            let guardrails = guardrails.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                execute_single_task(
                    task_id,
                    task_repo,
                    agent_repo,
                    substrate,
                    config,
                    event_tx,
                    total_tokens,
                    guardrails,
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

/// Execute a single task with retry logic and timeout.
async fn execute_single_task<T, A>(
    task_id: Uuid,
    task_repo: Arc<T>,
    agent_repo: Arc<A>,
    substrate: Arc<dyn Substrate>,
    config: ExecutorConfig,
    event_tx: mpsc::Sender<ExecutionEvent>,
    total_tokens: Arc<RwLock<u64>>,
    guardrails: Option<Arc<Guardrails>>,
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
    let system_prompt = match agent_repo.get_template_by_name(agent_type).await {
        Ok(Some(template)) => template.system_prompt,
        _ => format!(
            "You are a specialized agent for executing tasks.\n\
            Follow the task description carefully and complete the work.\n\
            Agent type: {}",
            agent_type
        ),
    };

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

        // Build request
        let request = SubstrateRequest::new(
            task_id,
            agent_type,
            &system_prompt,
            &task.description,
        ).with_config(SubstrateConfig::default().with_max_turns(config.default_max_turns));

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
        create_test_pool, SqliteAgentRepository, SqliteTaskRepository, Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::MockSubstrate;

    async fn setup_executor() -> DagExecutor<SqliteTaskRepository, SqliteAgentRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let agent_repo = Arc::new(SqliteAgentRepository::new(pool));
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let config = ExecutorConfig::default();

        DagExecutor::new(task_repo, agent_repo, substrate, config)
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

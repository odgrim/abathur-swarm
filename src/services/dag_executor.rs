//! DAG Executor service for wave-based parallel task execution.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    SessionStatus, SubstrateConfig, SubstrateRequest, SubstrateSession, TaskDag, TaskStatus,
};
use crate::domain::ports::{Substrate, TaskRepository};

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
    TaskFailed { task_id: Uuid, error: String },
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
pub struct DagExecutor<T: TaskRepository + 'static> {
    task_repo: Arc<T>,
    substrate: Arc<dyn Substrate>,
    config: ExecutorConfig,
    status: Arc<RwLock<ExecutionStatus>>,
    results: Arc<RwLock<ExecutionResults>>,
}

impl<T: TaskRepository + 'static> DagExecutor<T> {
    pub fn new(task_repo: Arc<T>, substrate: Arc<dyn Substrate>, config: ExecutorConfig) -> Self {
        Self {
            task_repo,
            substrate,
            config,
            status: Arc::new(RwLock::new(ExecutionStatus::Pending)),
            results: Arc::new(RwLock::new(ExecutionResults::default())),
        }
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

        let _stats = dag.stats();
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
            let wave_results = self.execute_wave(wave, dag, &event_tx).await?;

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
            let substrate = self.substrate.clone();
            let config = self.config.clone();
            let event_tx = event_tx.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                let start = std::time::Instant::now();

                // Get full task from repository
                let task = match task_repo.get(task_id).await {
                    Ok(Some(t)) => t,
                    Ok(None) => {
                        return TaskResult {
                            task_id,
                            status: TaskStatus::Failed,
                            session: None,
                            error: Some("Task not found".to_string()),
                            duration_secs: start.elapsed().as_secs(),
                        };
                    }
                    Err(e) => {
                        return TaskResult {
                            task_id,
                            status: TaskStatus::Failed,
                            session: None,
                            error: Some(e.to_string()),
                            duration_secs: start.elapsed().as_secs(),
                        };
                    }
                };

                let _ = event_tx.send(ExecutionEvent::TaskStarted {
                    task_id,
                    task_title: task.title.clone(),
                }).await;

                // Execute task on substrate
                let request = SubstrateRequest::new(
                    task_id,
                    task.agent_type.as_deref().unwrap_or("default"),
                    "", // System prompt would come from agent template
                    &task.description,
                ).with_config(SubstrateConfig::default().with_max_turns(config.default_max_turns));

                let result = match substrate.execute(request).await {
                    Ok(session) => {
                        let status = if session.status == SessionStatus::Completed {
                            TaskStatus::Complete
                        } else {
                            TaskStatus::Failed
                        };

                        TaskResult {
                            task_id,
                            status,
                            error: session.error.clone(),
                            session: Some(session),
                            duration_secs: start.elapsed().as_secs(),
                        }
                    }
                    Err(e) => {
                        TaskResult {
                            task_id,
                            status: TaskStatus::Failed,
                            session: None,
                            error: Some(e.to_string()),
                            duration_secs: start.elapsed().as_secs(),
                        }
                    }
                };

                // Send completion event
                if result.status == TaskStatus::Complete {
                    let _ = event_tx.send(ExecutionEvent::TaskCompleted {
                        task_id,
                        result: result.clone(),
                    }).await;
                } else {
                    let _ = event_tx.send(ExecutionEvent::TaskFailed {
                        task_id,
                        error: result.error.clone().unwrap_or_default(),
                    }).await;
                }

                result
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, SqliteTaskRepository, Migrator, all_embedded_migrations};
    use crate::adapters::substrates::MockSubstrate;

    async fn setup_executor() -> DagExecutor<SqliteTaskRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let config = ExecutorConfig::default();

        DagExecutor::new(task_repo, substrate, config)
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
}

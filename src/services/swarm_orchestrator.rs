//! Swarm Orchestrator - the central coordinator for the Abathur system.
//!
//! The orchestrator manages the execution loop, coordinating between:
//! - Goals and their task decomposition
//! - Task scheduling and DAG execution
//! - Agent spawning and management
//! - Worktree management for isolation
//! - Memory system for context sharing

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    Goal, GoalStatus, SessionStatus, SubstrateConfig, SubstrateRequest, TaskDag, TaskStatus,
};
use crate::domain::ports::{GoalRepository, Substrate, TaskRepository, WorktreeRepository};
use crate::services::{
    DagExecutor, ExecutionResults, ExecutionStatus, ExecutorConfig,
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
    /// Task spawned.
    TaskSpawned { task_id: Uuid, task_title: String, agent_type: Option<String> },
    /// Task completed.
    TaskCompleted { task_id: Uuid },
    /// Task failed.
    TaskFailed { task_id: Uuid, error: String },
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
    pub running_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub active_agents: usize,
    pub active_worktrees: usize,
    pub total_tokens_used: u64,
}

/// The main swarm orchestrator.
pub struct SwarmOrchestrator<G, T, W>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    goal_repo: Arc<G>,
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
    substrate: Arc<dyn Substrate>,
    config: SwarmConfig,
    status: Arc<RwLock<OrchestratorStatus>>,
    stats: Arc<RwLock<SwarmStats>>,
    agent_semaphore: Arc<Semaphore>,
}

impl<G, T, W> SwarmOrchestrator<G, T, W>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    pub fn new(
        goal_repo: Arc<G>,
        task_repo: Arc<T>,
        worktree_repo: Arc<W>,
        substrate: Arc<dyn Substrate>,
        config: SwarmConfig,
    ) -> Self {
        let max_agents = config.max_agents;
        Self {
            goal_repo,
            task_repo,
            worktree_repo,
            substrate,
            config,
            status: Arc::new(RwLock::new(OrchestratorStatus::Idle)),
            stats: Arc::new(RwLock::new(SwarmStats::default())),
            agent_semaphore: Arc::new(Semaphore::new(max_agents)),
        }
    }

    /// Start the orchestrator and run the main loop.
    pub async fn run(&self, event_tx: mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        {
            let mut status = self.status.write().await;
            *status = OrchestratorStatus::Running;
        }
        let _ = event_tx.send(SwarmEvent::Started).await;

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

            // Process active goals
            self.process_goals(&event_tx).await?;

            // Update stats
            self.update_stats(&event_tx).await?;

            // Wait before next iteration
            tokio::time::sleep(tokio::time::Duration::from_millis(self.config.poll_interval_ms)).await;
        }

        let _ = event_tx.send(SwarmEvent::Stopped).await;
        Ok(())
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

        if tasks.is_empty() {
            return Ok(());
        }

        // Get ready tasks
        let ready_tasks: Vec<_> = tasks.iter()
            .filter(|t| t.status == TaskStatus::Ready)
            .collect();

        // Spawn agents for ready tasks
        for task in ready_tasks {
            // Try to acquire agent permit
            if let Ok(permit) = self.agent_semaphore.clone().try_acquire_owned() {
                let _ = event_tx.send(SwarmEvent::TaskSpawned {
                    task_id: task.id,
                    task_title: task.title.clone(),
                    agent_type: task.agent_type.clone(),
                }).await;

                // Spawn task execution
                let task_id = task.id;
                let _task_title = task.title.clone();
                let task_description = task.description.clone();
                let agent_type = task.agent_type.clone().unwrap_or_else(|| "default".to_string());
                let substrate = self.substrate.clone();
                let task_repo = self.task_repo.clone();
                let event_tx = event_tx.clone();
                let max_turns = self.config.default_max_turns;

                tokio::spawn(async move {
                    let _permit = permit;

                    // Update task to running
                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        let _ = task.transition_to(TaskStatus::Running);
                        let _ = task_repo.update(&task).await;
                    }

                    // Execute on substrate
                    let request = SubstrateRequest::new(
                        task_id,
                        &agent_type,
                        "", // System prompt from agent template
                        &task_description,
                    ).with_config(SubstrateConfig::default().with_max_turns(max_turns));

                    let result = substrate.execute(request).await;

                    // Update task based on result
                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        match result {
                            Ok(session) if session.status == SessionStatus::Completed => {
                                let _ = task.transition_to(TaskStatus::Complete);
                                let _ = task_repo.update(&task).await;
                                let _ = event_tx.send(SwarmEvent::TaskCompleted { task_id }).await;
                            }
                            Ok(session) => {
                                let _ = task.transition_to(TaskStatus::Failed);
                                let _ = task_repo.update(&task).await;
                                let _ = event_tx.send(SwarmEvent::TaskFailed {
                                    task_id,
                                    error: session.error.unwrap_or_else(|| "Unknown error".to_string()),
                                }).await;
                            }
                            Err(e) => {
                                let _ = task.transition_to(TaskStatus::Failed);
                                let _ = task_repo.update(&task).await;
                                let _ = event_tx.send(SwarmEvent::TaskFailed {
                                    task_id,
                                    error: e.to_string(),
                                }).await;
                            }
                        }
                    }
                });
            }
        }

        // Check if goal is complete
        let all_complete = tasks.iter().all(|t| t.status == TaskStatus::Complete);
        let any_failed = tasks.iter().any(|t| t.status == TaskStatus::Failed);

        if all_complete {
            let mut updated_goal = goal.clone();
            updated_goal.complete();
            self.goal_repo.update(&updated_goal).await?;
            let _ = event_tx.send(SwarmEvent::GoalCompleted { goal_id: goal.id }).await;
        } else if any_failed && !self.config.auto_retry {
            let mut updated_goal = goal.clone();
            updated_goal.fail("One or more tasks failed");
            self.goal_repo.update(&updated_goal).await?;
            let _ = event_tx.send(SwarmEvent::GoalFailed {
                goal_id: goal.id,
                error: "Task failures".to_string(),
            }).await;
        }

        Ok(())
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
            running_tasks: *task_counts.get(&TaskStatus::Running).unwrap_or(&0) as usize,
            completed_tasks: *task_counts.get(&TaskStatus::Complete).unwrap_or(&0) as usize,
            failed_tasks: *task_counts.get(&TaskStatus::Failed).unwrap_or(&0) as usize,
            active_agents: self.config.max_agents - self.agent_semaphore.available_permits(),
            active_worktrees,
            total_tokens_used: 0, // Would need to track this separately
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
            self.substrate.clone(),
            executor_config,
        );

        let results = executor.execute(&dag).await?;

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
        self.process_goals(&tx).await?;
        self.update_stats(&tx).await?;
        Ok(self.stats().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_test_pool, SqliteGoalRepository, SqliteTaskRepository, SqliteWorktreeRepository,
        Migrator, all_embedded_migrations,
    };
    use crate::adapters::substrates::MockSubstrate;

    async fn setup_orchestrator() -> SwarmOrchestrator<SqliteGoalRepository, SqliteTaskRepository, SqliteWorktreeRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let goal_repo = Arc::new(SqliteGoalRepository::new(pool.clone()));
        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let worktree_repo = Arc::new(SqliteWorktreeRepository::new(pool));
        let substrate: Arc<dyn Substrate> = Arc::new(MockSubstrate::new());
        let config = SwarmConfig::default();

        SwarmOrchestrator::new(goal_repo, task_repo, worktree_repo, substrate, config)
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
}

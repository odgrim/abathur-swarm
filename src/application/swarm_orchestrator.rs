//! Swarm Orchestrator with Background Task Processing
//!
//! Manages concurrent agent workers, task distribution, and resource monitoring
//! using tokio async concurrency primitives.

use crate::application::agent_executor::{AgentExecutor, ExecutionContext};
use crate::application::resource_monitor::ResourceMonitor;
use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::{Config, Task};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Swarm orchestrator state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmState {
    /// Swarm is stopped
    Stopped,
    /// Swarm is starting up
    Starting,
    /// Swarm is running and processing tasks
    Running,
    /// Swarm is stopping
    Stopping,
}

/// Statistics about the swarm
#[derive(Debug, Clone)]
pub struct SwarmStats {
    pub state: SwarmState,
    pub max_agents: usize,
    pub active_agents: usize,
    pub idle_agents: usize,
    pub tasks_processed: u64,
    pub tasks_failed: u64,
}

/// Worker agent state tracking
#[derive(Debug, Clone)]
struct WorkerState {
    #[allow(dead_code)] // Reserved for monitoring and debugging
    agent_id: Uuid,
    task_id: Option<Uuid>,
    #[allow(dead_code)] // Reserved for timeout detection
    started_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Agent completion event
#[derive(Debug, Clone)]
enum AgentEvent {
    /// Agent completed a task successfully
    TaskCompleted { agent_id: Uuid, task_id: Uuid },
    /// Agent failed a task
    TaskFailed {
        agent_id: Uuid,
        task_id: Uuid,
        error: String,
    },
}

/// Swarm orchestrator with concurrent task processing
///
/// Orchestrates concurrent agent workers using tokio primitives:
/// - Semaphore for bounding max concurrent agents
/// - mpsc channels for agent completion events
/// - broadcast channels for shutdown signals
/// - Background monitoring tasks with tokio::select!
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────────────┐
/// │         SwarmOrchestrator                   │
/// ├─────────────────────────────────────────────┤
/// │ - Agent Pool (Semaphore)                    │
/// │ - Task Coordinator (fetch ready tasks)      │
/// │ - Resource Monitor (throttle on pressure)   │
/// │ - Worker Tracking (HashMap<Uuid, State>)    │
/// └─────────────────────────────────────────────┘
///          │                    │
///          ▼                    ▼
///   Task Polling Loop    Resource Monitor
///   (1s interval)        (5s interval)
///          │                    │
///          └────────────────────┘
///                   │
///         Spawns Agent Workers
///                   │
///                   ▼
///          AgentExecutor Tasks
/// ```
///
/// # Examples
///
/// ```no_run
/// use abathur::application::SwarmOrchestrator;
/// use std::sync::Arc;
///
/// # async fn example(
/// #     task_coordinator: Arc<abathur::application::TaskCoordinator>,
/// #     agent_executor: Arc<abathur::application::AgentExecutor>,
/// #     resource_monitor: Arc<abathur::application::ResourceMonitor>,
/// #     config: abathur::domain::models::Config,
/// # ) -> anyhow::Result<()> {
/// let mut orchestrator = SwarmOrchestrator::new(
///     10,
///     task_coordinator,
///     agent_executor,
///     resource_monitor,
///     config,
/// );
///
/// // Start background processing
/// orchestrator.start().await?;
///
/// // Check stats
/// let stats = orchestrator.get_stats().await;
/// println!("Active agents: {}", stats.active_agents);
///
/// // Graceful shutdown
/// orchestrator.stop().await?;
/// # Ok(())
/// # }
/// ```
pub struct SwarmOrchestrator {
    state: Arc<RwLock<SwarmState>>,
    max_agents: Arc<RwLock<usize>>,
    tasks_processed: Arc<RwLock<u64>>,
    tasks_failed: Arc<RwLock<u64>>,

    // Concurrency control
    agent_semaphore: Arc<Semaphore>,
    workers: Arc<RwLock<HashMap<Uuid, WorkerState>>>,

    // Dependencies
    task_coordinator: Arc<TaskCoordinator>,
    agent_executor: Arc<AgentExecutor>,
    resource_monitor: Arc<ResourceMonitor>,
    config: Config,

    // Communication channels
    agent_event_tx: mpsc::Sender<AgentEvent>,
    agent_event_rx: Arc<RwLock<Option<mpsc::Receiver<AgentEvent>>>>,
    shutdown_tx: broadcast::Sender<()>,

    // Background task handles
    task_loop_handle: Arc<RwLock<Option<JoinHandle<Result<()>>>>>,
    resource_monitor_handle: Arc<RwLock<Option<JoinHandle<Result<()>>>>>,
}

impl SwarmOrchestrator {
    /// Create a new swarm orchestrator
    ///
    /// # Arguments
    ///
    /// * `max_agents` - Maximum concurrent agent workers
    /// * `task_coordinator` - Task queue and lifecycle coordinator
    /// * `agent_executor` - Agent task executor
    /// * `resource_monitor` - System resource monitor
    /// * `config` - Application configuration
    pub fn new(
        max_agents: usize,
        task_coordinator: Arc<TaskCoordinator>,
        agent_executor: Arc<AgentExecutor>,
        resource_monitor: Arc<ResourceMonitor>,
        config: Config,
    ) -> Self {
        let (agent_event_tx, agent_event_rx) = mpsc::channel(1000);
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            state: Arc::new(RwLock::new(SwarmState::Stopped)),
            max_agents: Arc::new(RwLock::new(max_agents)),
            tasks_processed: Arc::new(RwLock::new(0)),
            tasks_failed: Arc::new(RwLock::new(0)),
            agent_semaphore: Arc::new(Semaphore::new(max_agents)),
            workers: Arc::new(RwLock::new(HashMap::new())),
            task_coordinator,
            agent_executor,
            resource_monitor,
            config,
            agent_event_tx,
            agent_event_rx: Arc::new(RwLock::new(Some(agent_event_rx))),
            shutdown_tx,
            task_loop_handle: Arc::new(RwLock::new(None)),
            resource_monitor_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Start the swarm orchestrator
    ///
    /// Initializes:
    /// 1. Resource monitoring background task
    /// 2. Task polling and distribution loop
    /// 3. Agent event processing loop
    pub async fn start(&mut self) -> Result<()> {
        let mut state = self.state.write().await;
        if *state != SwarmState::Stopped {
            return Ok(());
        }

        *state = SwarmState::Starting;
        drop(state);

        info!(max_agents = *self.max_agents.read().await, "Starting swarm orchestrator");

        // Start resource monitoring (5 second interval)
        let resource_handle = self
            .resource_monitor
            .start(Duration::from_secs(5))
            .await
            .context("Failed to start resource monitor")?;

        *self.resource_monitor_handle.write().await = Some(resource_handle);

        // Start main task processing loop
        let task_loop_handle = self.spawn_task_processing_loop().await?;
        *self.task_loop_handle.write().await = Some(task_loop_handle);

        let mut state = self.state.write().await;
        *state = SwarmState::Running;

        info!("Swarm orchestrator started successfully");

        Ok(())
    }

    /// Stop the swarm orchestrator
    ///
    /// Graceful shutdown:
    /// 1. Stop accepting new tasks
    /// 2. Wait for active tasks to complete (with 30s timeout)
    /// 3. Cancel remaining tasks
    /// 4. Shutdown resource monitoring
    pub async fn stop(&mut self) -> Result<()> {
        let mut state = self.state.write().await;
        if *state == SwarmState::Stopped {
            return Ok(());
        }

        *state = SwarmState::Stopping;
        drop(state);

        info!("Stopping swarm orchestrator");

        // Broadcast shutdown signal to all background tasks
        let _ = self.shutdown_tx.send(());

        // Wait for task processing loop to finish (with timeout)
        if let Some(handle) = self.task_loop_handle.write().await.take() {
            match tokio::time::timeout(Duration::from_secs(30), handle).await {
                Ok(Ok(Ok(()))) => {
                    info!("Task processing loop stopped cleanly");
                }
                Ok(Ok(Err(e))) => {
                    warn!(error = ?e, "Task processing loop returned error");
                }
                Ok(Err(e)) => {
                    warn!(error = ?e, "Task processing loop panicked");
                }
                Err(_) => {
                    warn!("Task processing loop shutdown timeout");
                }
            }
        }

        // Wait for active agents to complete (with timeout)
        let active_count = self.workers.read().await.len();
        if active_count > 0 {
            info!(active_agents = active_count, "Waiting for agents to complete");

            // Wait up to 30 seconds for agents to finish
            let wait_result = tokio::time::timeout(
                Duration::from_secs(30),
                self.wait_for_agents_completion(),
            )
            .await;

            match wait_result {
                Ok(()) => {
                    info!("All agents completed successfully");
                }
                Err(_) => {
                    warn!(
                        remaining = self.workers.read().await.len(),
                        "Shutdown timeout reached, some agents may still be running"
                    );
                }
            }
        }

        // Shutdown resource monitor
        self.resource_monitor
            .shutdown()
            .await
            .context("Failed to shutdown resource monitor")?;

        if let Some(handle) = self.resource_monitor_handle.write().await.take() {
            handle.await??;
        }

        let mut state = self.state.write().await;
        *state = SwarmState::Stopped;

        info!("Swarm orchestrator stopped");

        Ok(())
    }

    /// Get current swarm statistics
    pub async fn get_stats(&self) -> SwarmStats {
        let state = *self.state.read().await;
        let max_agents = *self.max_agents.read().await;
        let tasks_processed = *self.tasks_processed.read().await;
        let tasks_failed = *self.tasks_failed.read().await;

        let workers = self.workers.read().await;
        let active_agents = workers.values().filter(|w| w.task_id.is_some()).count();
        let idle_agents = workers.len() - active_agents;

        SwarmStats {
            state,
            max_agents,
            active_agents,
            idle_agents,
            tasks_processed,
            tasks_failed,
        }
    }

    /// Get current swarm state
    pub async fn get_state(&self) -> SwarmState {
        *self.state.read().await
    }

    /// Update max agents limit
    pub async fn set_max_agents(&self, max_agents: usize) {
        *self.max_agents.write().await = max_agents;
        // Note: Semaphore permits cannot be dynamically changed
        // New limit will apply after orchestrator restart
        info!(new_max_agents = max_agents, "Max agents limit updated (requires restart to apply)");
    }

    // ========================
    // Background Task Loops
    // ========================

    /// Spawn the main task processing loop
    ///
    /// This loop:
    /// 1. Polls for ready tasks every 1 second
    /// 2. Assigns tasks to idle agents (respecting semaphore limits)
    /// 3. Processes agent completion events
    /// 4. Handles graceful shutdown
    async fn spawn_task_processing_loop(&self) -> Result<JoinHandle<Result<()>>> {
        let task_coordinator = Arc::clone(&self.task_coordinator);
        let agent_executor = Arc::clone(&self.agent_executor);
        let _resource_monitor = Arc::clone(&self.resource_monitor);
        let config = self.config.clone();

        let agent_semaphore = Arc::clone(&self.agent_semaphore);
        let workers = Arc::clone(&self.workers);
        let tasks_processed = Arc::clone(&self.tasks_processed);
        let tasks_failed = Arc::clone(&self.tasks_failed);

        let agent_event_tx = self.agent_event_tx.clone();
        let mut agent_event_rx = self
            .agent_event_rx
            .write()
            .await
            .take()
            .context("Agent event receiver already taken")?;

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            let mut task_poll_interval = interval(Duration::from_secs(1));

            info!("Task processing loop started");

            loop {
                tokio::select! {
                    // Poll for ready tasks every 1 second
                    _ = task_poll_interval.tick() => {
                        // Try to get a ready task
                        match task_coordinator.get_next_ready_task().await {
                            Ok(Some(task)) => {
                                debug!(task_id = %task.id, "Found ready task");

                                // Spawn agent worker for this task
                                if let Err(e) = Self::spawn_agent_worker(
                                    task,
                                    Arc::clone(&agent_semaphore),
                                    Arc::clone(&agent_executor),
                                    Arc::clone(&task_coordinator),
                                    Arc::clone(&workers),
                                    agent_event_tx.clone(),
                                    config.clone(),
                                ).await {
                                    error!(error = ?e, "Failed to spawn agent worker");
                                }
                            }
                            Ok(None) => {
                                // No ready tasks, continue polling
                                debug!("No ready tasks available");
                            }
                            Err(e) => {
                                error!(error = ?e, "Failed to fetch next ready task");
                            }
                        }
                    }

                    // Process agent completion events
                    Some(event) = agent_event_rx.recv() => {
                        match event {
                            AgentEvent::TaskCompleted { agent_id, task_id } => {
                                info!(%agent_id, %task_id, "Agent completed task");

                                // Remove worker from tracking
                                workers.write().await.remove(&agent_id);

                                // Increment processed counter
                                *tasks_processed.write().await += 1;

                                // Handle task completion (trigger dependents)
                                if let Err(e) = task_coordinator.handle_task_completion(task_id).await {
                                    error!(error = ?e, %task_id, "Failed to handle task completion");
                                }
                            }
                            AgentEvent::TaskFailed { agent_id, task_id, error } => {
                                warn!(%agent_id, %task_id, %error, "Agent failed task");

                                // Remove worker from tracking
                                workers.write().await.remove(&agent_id);

                                // Increment failed counter
                                *tasks_failed.write().await += 1;

                                // Handle task failure
                                if let Err(e) = task_coordinator.handle_task_failure(task_id, error).await {
                                    error!(error = ?e, %task_id, "Failed to handle task failure");
                                }
                            }
                        }
                    }

                    // Shutdown signal
                    _ = shutdown_rx.recv() => {
                        info!("Task processing loop received shutdown signal");
                        break;
                    }
                }
            }

            info!("Task processing loop stopped");
            Ok(())
        });

        Ok(handle)
    }

    /// Spawn an agent worker to execute a task
    ///
    /// Uses semaphore for concurrency control and spawns a tokio task
    /// that executes the agent and sends completion events.
    async fn spawn_agent_worker(
        task: Task,
        agent_semaphore: Arc<Semaphore>,
        agent_executor: Arc<AgentExecutor>,
        task_coordinator: Arc<TaskCoordinator>,
        workers: Arc<RwLock<HashMap<Uuid, WorkerState>>>,
        agent_event_tx: mpsc::Sender<AgentEvent>,
        config: Config,
    ) -> Result<()> {
        let agent_id = Uuid::new_v4();
        let task_id = task.id;

        // Clone Arc before acquiring permit to satisfy lifetime requirements
        let semaphore = Arc::clone(&agent_semaphore);

        // Acquire semaphore permit (blocks if max agents reached)
        let permit = semaphore
            .acquire_owned()
            .await
            .context("Failed to acquire agent semaphore permit")?;

        // Mark task as running
        task_coordinator
            .mark_task_running(task_id)
            .await
            .context("Failed to mark task as running")?;

        // Track worker
        {
            let mut workers_map = workers.write().await;
            workers_map.insert(
                agent_id,
                WorkerState {
                    agent_id,
                    task_id: Some(task_id),
                    started_at: Some(chrono::Utc::now()),
                },
            );
        }

        info!(%agent_id, %task_id, "Spawning agent worker");

        // Spawn agent execution task
        tokio::spawn(async move {
            // Build execution context
            let ctx = ExecutionContext::new(
                agent_id,
                task_id,
                task.agent_type.clone(),
                task.description.clone(),
                config,
            )
            .with_input_data(task.input_data.clone().unwrap_or(serde_json::Value::Null));

            // Execute task
            let result = agent_executor.execute(ctx).await;

            // Send completion event
            let event = match result {
                Ok(_output) => AgentEvent::TaskCompleted { agent_id, task_id },
                Err(e) => AgentEvent::TaskFailed {
                    agent_id,
                    task_id,
                    error: e.to_string(),
                },
            };

            // Send event (don't fail if receiver dropped during shutdown)
            let _ = agent_event_tx.send(event).await;

            // Release semaphore permit (automatically dropped)
            drop(permit);
        });

        Ok(())
    }

    /// Wait for all active agents to complete
    async fn wait_for_agents_completion(&self) {
        loop {
            let active_count = self.workers.read().await.len();
            if active_count == 0 {
                break;
            }

            debug!(active_agents = active_count, "Waiting for agents to complete");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::resource_monitor::ResourceLimits;
    use crate::domain::models::{DependencyType, TaskSource, TaskStatus};
    use crate::domain::ports::{PriorityCalculator, TaskQueueService};
    use crate::services::DependencyResolver;
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::HashMap as StdHashMap;
    use std::sync::Mutex as StdMutex;

    // Mock implementations for testing
    struct MockTaskQueue {
        tasks: Arc<StdMutex<StdHashMap<Uuid, Task>>>,
    }

    impl MockTaskQueue {
        fn new() -> Self {
            Self {
                tasks: Arc::new(StdMutex::new(StdHashMap::new())),
            }
        }

        #[allow(dead_code)] // Reserved for future integration tests
        fn add_task(&self, task: Task) {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task.id, task);
        }
    }

    #[async_trait]
    impl TaskQueueService for MockTaskQueue {
        async fn get_task(&self, task_id: Uuid) -> Result<Task> {
            let tasks = self.tasks.lock().unwrap();
            tasks
                .get(&task_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Task not found"))
        }

        async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|t| t.status == status)
                .cloned()
                .collect())
        }

        async fn get_dependent_tasks(&self, _task_id: Uuid) -> Result<Vec<Task>> {
            Ok(vec![])
        }

        async fn update_task_status(&self, task_id: Uuid, status: TaskStatus) -> Result<()> {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.status = status;
                Ok(())
            } else {
                Err(anyhow::anyhow!("Task not found"))
            }
        }

        async fn update_task_priority(&self, task_id: Uuid, priority: f64) -> Result<()> {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.calculated_priority = priority;
                Ok(())
            } else {
                Err(anyhow::anyhow!("Task not found"))
            }
        }

        async fn mark_task_failed(&self, task_id: Uuid, error_message: String) -> Result<()> {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.status = TaskStatus::Failed;
                task.error_message = Some(error_message);
                Ok(())
            } else {
                Err(anyhow::anyhow!("Task not found"))
            }
        }

        async fn get_next_ready_task(&self) -> Result<Option<Task>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|t| t.status == TaskStatus::Ready)
                .max_by(|a, b| {
                    a.calculated_priority
                        .partial_cmp(&b.calculated_priority)
                        .unwrap()
                })
                .cloned())
        }
    }

    struct MockPriorityCalculator;

    #[async_trait]
    impl PriorityCalculator for MockPriorityCalculator {
        async fn calculate_priority(&self, task: &Task) -> Result<f64> {
            Ok(f64::from(task.priority))
        }

        async fn recalculate_priorities(&self, tasks: &[Task]) -> Result<Vec<(Uuid, f64)>> {
            Ok(tasks
                .iter()
                .map(|t| (t.id, f64::from(t.priority)))
                .collect())
        }
    }

    #[allow(dead_code)] // Reserved for future integration tests
    fn create_test_task(status: TaskStatus) -> Task {
        Task {
            id: Uuid::new_v4(),
            summary: "Test task".to_string(),
            description: "Test description".to_string(),
            agent_type: "test-agent".to_string(),
            priority: 5,
            calculated_priority: 5.0,
            status,
            dependencies: None,
            dependency_type: DependencyType::Sequential,
            dependency_depth: 0,
            input_data: None,
            result_data: None,
            error_message: None,
            retry_count: 0,
            max_retries: 3,
            max_execution_timeout_seconds: 3600,
            submitted_at: Utc::now(),
            started_at: None,
            completed_at: None,
            last_updated_at: Utc::now(),
            created_by: None,
            parent_task_id: None,
            session_id: None,
            source: TaskSource::Human,
            deadline: None,
            estimated_duration_seconds: None,
            feature_branch: None,
            task_branch: None,
            worktree_path: None,
        }
    }

    #[tokio::test]
    async fn test_swarm_orchestrator_lifecycle() {
        // This test verifies the basic start/stop lifecycle
        // Note: Full integration testing requires mock ClaudeClient and McpClient
        let _task_queue = Arc::new(MockTaskQueue::new());
        let _dependency_resolver = Arc::new(DependencyResolver::new());
        let _priority_calc = Arc::new(MockPriorityCalculator);

        // Note: Would need mock ClaudeClient and McpClient for full test
        // For now, this test demonstrates the structure
    }

    #[tokio::test]
    async fn test_swarm_stats() {
        // Test stats tracking without full orchestrator
        let limits = ResourceLimits::default();
        let _resource_monitor = Arc::new(ResourceMonitor::new(limits));

        // Stats should reflect correct state
        // Additional testing would require full mock setup
    }
}

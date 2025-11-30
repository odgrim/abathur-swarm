//! Swarm Orchestrator with Background Task Processing
//!
//! Manages concurrent agent workers, task distribution, and resource monitoring
//! using tokio async concurrency primitives.

use crate::application::agent_executor::AgentExecutor;
use crate::application::resource_monitor::ResourceMonitor;
use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::{Config, Task};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
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
    /// Agent execution complete, validation requested
    ValidationRequested { agent_id: Uuid, task_id: Uuid },
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
    /// In-flight task IDs to prevent duplicate spawns during race conditions.
    /// A task is added here when picked up and removed when spawning completes or fails.
    in_flight_tasks: Arc<RwLock<HashSet<Uuid>>>,

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
    /// Buffer time added to task's max_execution_timeout_seconds before considering stale.
    /// This prevents false positives for tasks that are completing normally.
    const STALE_TASK_BUFFER_SECS: u64 = 120; // 2 minute buffer

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
            in_flight_tasks: Arc::new(RwLock::new(HashSet::new())),
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
    /// 2. Process all pending tasks (transition to Ready/Blocked)
    /// 3. Task polling and distribution loop
    /// 4. Agent event processing loop
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

        // Process all pending tasks to transition them to Ready or Blocked
        info!("Processing pending tasks on startup");
        match self.task_coordinator.process_pending_tasks().await {
            Ok(count) => {
                info!("Processed {} pending tasks", count);
            }
            Err(e) => {
                warn!("Error processing pending tasks: {:?}", e);
                // Continue anyway - this is not a fatal error
            }
        }

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
        let in_flight_tasks = Arc::clone(&self.in_flight_tasks);
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
            // Use configurable recovery intervals
            let stale_interval_secs = config.recovery.stale_task_check_interval_secs;
            let blocked_interval_secs = config.recovery.blocked_task_check_interval_secs;
            let chain_handoff_interval_secs = config.recovery.chain_handoff_check_interval_secs;

            info!(
                stale_check_interval_secs = stale_interval_secs,
                blocked_check_interval_secs = blocked_interval_secs,
                chain_handoff_check_interval_secs = chain_handoff_interval_secs,
                "Starting task processing with configurable recovery intervals"
            );

            let mut stale_check_interval = interval(Duration::from_secs(stale_interval_secs));
            let mut blocked_check_interval = interval(Duration::from_secs(blocked_interval_secs));
            let mut chain_handoff_check_interval = interval(Duration::from_secs(chain_handoff_interval_secs));

            info!("Task processing loop started");

            loop {
                tokio::select! {
                    // Poll for ready tasks every 1 second
                    _ = task_poll_interval.tick() => {
                        // Atomically claim and mark task as Running in a single DB transaction
                        match task_coordinator.claim_next_ready_task().await {
                            Ok(Some(task)) => {
                                let task_id = task.id;

                                // Atomic check-and-insert to prevent duplicate spawns
                                {
                                    let mut in_flight = in_flight_tasks.write().await;
                                    if !in_flight.insert(task_id) {
                                        // insert() returns false if already present
                                        debug!(
                                            task_id = %task_id,
                                            "Task already in-flight, skipping duplicate spawn"
                                        );
                                        continue;
                                    }
                                }

                                info!(
                                    task_id = %task.id,
                                    agent_type = %task.agent_type,
                                    summary = %task.summary,
                                    priority = task.calculated_priority,
                                    "Claimed task for execution (atomically marked as Running)"
                                );

                                // Spawn agent worker for this task
                                if let Err(e) = Self::spawn_agent_worker(
                                    task,
                                    Arc::clone(&agent_semaphore),
                                    Arc::clone(&agent_executor),
                                    Arc::clone(&task_coordinator),
                                    Arc::clone(&workers),
                                    Arc::clone(&in_flight_tasks),
                                    agent_event_tx.clone(),
                                    config.clone(),
                                ).await {
                                    error!(error = ?e, "Failed to spawn agent worker");
                                    // Remove from in-flight on spawn failure
                                    let mut in_flight = in_flight_tasks.write().await;
                                    in_flight.remove(&task_id);
                                }
                            }
                            Ok(None) => {
                                // No ready tasks, continue polling
                            }
                            Err(e) => {
                                error!(error = ?e, "Failed to fetch next ready task");
                            }
                        }
                    }

                    // Process agent completion events (for stats and worker cleanup only)
                    // Note: Task status is updated IMMEDIATELY in the spawned task, not here
                    Some(event) = agent_event_rx.recv() => {
                        match event {
                            AgentEvent::TaskCompleted { agent_id, task_id } => {
                                // Fetch task details for logging
                                match task_coordinator.get_task(task_id).await {
                                    Ok(task) => {
                                        info!(
                                            agent_id = %agent_id,
                                            task_id = %task_id,
                                            agent_type = %task.agent_type,
                                            summary = %task.summary,
                                            "Task completed successfully (status already updated)"
                                        );
                                    }
                                    Err(_) => {
                                        info!(%agent_id, %task_id, "Task completed successfully (status already updated)");
                                    }
                                }

                                // Remove worker from tracking
                                workers.write().await.remove(&agent_id);

                                // Remove from in-flight tasks
                                in_flight_tasks.write().await.remove(&task_id);

                                // Increment processed counter
                                *tasks_processed.write().await += 1;

                                // NOTE: Task status, hooks, and dependent triggering already handled
                                // in the spawned task immediately after subprocess exit
                            }
                            AgentEvent::TaskFailed { agent_id, task_id, error } => {
                                // Fetch task details for logging
                                match task_coordinator.get_task(task_id).await {
                                    Ok(task) => {
                                        warn!(
                                            agent_id = %agent_id,
                                            task_id = %task_id,
                                            agent_type = %task.agent_type,
                                            summary = %task.summary,
                                            retry_count = task.retry_count,
                                            max_retries = task.max_retries,
                                            error = %error,
                                            "Task failed (status already updated)"
                                        );
                                    }
                                    Err(_) => {
                                        warn!(%agent_id, %task_id, %error, "Task failed (status already updated)");
                                    }
                                }

                                // Remove worker from tracking
                                workers.write().await.remove(&agent_id);

                                // Remove from in-flight tasks
                                in_flight_tasks.write().await.remove(&task_id);

                                // Increment failed counter
                                *tasks_failed.write().await += 1;

                                // NOTE: Task status, error message, retry logic already handled
                                // in the spawned task immediately after subprocess exit
                            }
                            AgentEvent::ValidationRequested { agent_id, task_id } => {
                                info!(%agent_id, %task_id, "Validation requested for task (status already updated)");

                                // Remove worker from tracking
                                workers.write().await.remove(&agent_id);

                                // Remove from in-flight tasks
                                in_flight_tasks.write().await.remove(&task_id);

                                // Task is now AwaitingValidation status (already updated in spawned task)
                                // Validation task will run via normal task queue
                                // Don't increment completed counter yet - wait for validation
                                info!(%task_id, "Task awaiting validation, validation task will run next");
                            }
                        }
                    }

                    // Check for stale tasks periodically
                    _ = stale_check_interval.tick() => {
                        if let Err(e) = Self::recover_stale_tasks(
                            Arc::clone(&task_coordinator),
                            Arc::clone(&in_flight_tasks),
                        ).await {
                            error!(error = ?e, "Failed to recover stale tasks");
                        }
                    }

                    // Check for stuck blocked tasks periodically
                    _ = blocked_check_interval.tick() => {
                        if let Err(e) = task_coordinator.recover_stuck_blocked_tasks().await {
                            error!(error = ?e, "Failed to recover stuck blocked tasks");
                        }
                    }

                    // Check for stuck chain handoffs periodically
                    _ = chain_handoff_check_interval.tick() => {
                        let handoff_timeout_secs = config.recovery.chain_handoff_timeout_secs;
                        let handoff_max_attempts = config.recovery.chain_handoff_max_attempts;
                        if let Err(e) = Self::recover_stuck_chain_handoffs(
                            Arc::clone(&task_coordinator),
                            Arc::clone(&agent_executor),
                            handoff_timeout_secs,
                            handoff_max_attempts,
                        ).await {
                            error!(error = ?e, "Failed to recover stuck chain handoffs");
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
    ///
    /// NOTE: The task is expected to already be in Running status (claimed atomically).
    /// This function does NOT call mark_task_running() again.
    async fn spawn_agent_worker(
        task: Task,
        agent_semaphore: Arc<Semaphore>,
        agent_executor: Arc<AgentExecutor>,
        task_coordinator: Arc<TaskCoordinator>,
        workers: Arc<RwLock<HashMap<Uuid, WorkerState>>>,
        _in_flight_tasks: Arc<RwLock<HashSet<Uuid>>>,
        agent_event_tx: mpsc::Sender<AgentEvent>,
        _config: Config,
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

        // Task is already marked as Running by claim_next_ready_task() - no need to mark again

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
            // Execute task (automatically detects and executes prompt chains if chain_id is present)
            let result = agent_executor.execute_task(&task).await;

            // CRITICAL: Update task status IMMEDIATELY after subprocess exits,
            // BEFORE sending event to ensure database reflects reality
            // even if event loop is not running or channel is full.
            use crate::application::task_coordinator::{TaskCompletionResult, TaskCompletionError};

            /// Helper to handle task completion result and map to appropriate event
            async fn handle_completion_result(
                task_coordinator: &TaskCoordinator,
                task_id: Uuid,
                agent_id: Uuid,
            ) -> AgentEvent {
                match task_coordinator.handle_task_completion(task_id).await {
                    Ok(TaskCompletionResult::Success) => {
                        info!(%task_id, "Task completed successfully, all dependencies coordinated");
                        AgentEvent::TaskCompleted { agent_id, task_id }
                    }
                    Ok(TaskCompletionResult::CompletedWithDependencyFailures(failed_deps)) => {
                        // Task is completed but some dependencies couldn't be coordinated.
                        // This is OK - they will be recovered by background monitoring (30s).
                        // Still report as completed since the parent task's work IS done.
                        warn!(
                            %task_id,
                            failed_deps = ?failed_deps,
                            "Task completed but {} dependencies failed to coordinate (will recover in 30s)",
                            failed_deps.len()
                        );
                        AgentEvent::TaskCompleted { agent_id, task_id }
                    }
                    Err(TaskCompletionError::MarkCompletedFailed(e)) => {
                        // CRITICAL: Failed to mark task as completed - this is a serious error
                        // The task needs to be retried because the DB doesn't reflect completion
                        error!(%task_id, error = %e, "CRITICAL: Failed to mark task as completed in database");
                        let error_msg = format!("Failed to mark task completed: {}", e);
                        if let Err(err) = task_coordinator.handle_task_failure(task_id, error_msg.clone()).await {
                            error!(%task_id, error = ?err, "Also failed to mark task as failed");
                        }
                        AgentEvent::TaskFailed {
                            agent_id,
                            task_id,
                            error: error_msg,
                        }
                    }
                    Err(TaskCompletionError::HookBlocked) => {
                        // Hook blocked completion - this is intentional, not an error
                        // Don't mark as failed, just report that it didn't complete
                        warn!(%task_id, "Task completion blocked by pre-complete hook");
                        AgentEvent::TaskFailed {
                            agent_id,
                            task_id,
                            error: "Task completion blocked by pre-complete hook".to_string(),
                        }
                    }
                    Err(TaskCompletionError::TaskNotFound(id)) => {
                        error!(%task_id, "Task {} not found during completion", id);
                        AgentEvent::TaskFailed {
                            agent_id,
                            task_id,
                            error: format!("Task not found: {}", id),
                        }
                    }
                    Err(TaskCompletionError::Other(e)) => {
                        error!(%task_id, error = ?e, "Error during task completion");
                        let error_msg = format!("Task completion error: {}", e);
                        if let Err(err) = task_coordinator.handle_task_failure(task_id, error_msg.clone()).await {
                            error!(%task_id, error = ?err, "Also failed to mark task as failed");
                        }
                        AgentEvent::TaskFailed {
                            agent_id,
                            task_id,
                            error: error_msg,
                        }
                    }
                }
            }

            let event = match &result {
                Ok(_output) => {
                    // Check if this agent type requires validation
                    use crate::domain::models::AgentContractRegistry;
                    use crate::application::validation::{validate_task_completion, spawn_validation_task, ValidationResult};

                    let validation_req = AgentContractRegistry::get_validation_requirement(&task.agent_type);

                    match validation_req {
                        crate::domain::models::ValidationRequirement::None => {
                            // No validation required - mark as completed IMMEDIATELY
                            info!(%task_id, agent_type = %task.agent_type, "No validation required, marking as completed");
                            handle_completion_result(&task_coordinator, task_id, agent_id).await
                        }

                        crate::domain::models::ValidationRequirement::Contract { .. } => {
                            // Run inline contract validation
                            info!(%task_id, agent_type = %task.agent_type, "Running contract validation");
                            match validate_task_completion(task_id, &task, &task_coordinator).await {
                                Ok(ValidationResult::Passed) => {
                                    info!(%task_id, "Contract validation passed, marking as completed");
                                    handle_completion_result(&task_coordinator, task_id, agent_id).await
                                }
                                Ok(ValidationResult::Failed { reason }) => {
                                    error!(%task_id, reason = %reason, "Contract validation failed, marking as failed");
                                    let error_msg = format!("Contract validation failed: {}", reason);
                                    if let Err(e) = task_coordinator.handle_task_failure(task_id, error_msg.clone()).await {
                                        error!(%task_id, error = ?e, "Failed to mark task as failed");
                                    }
                                    AgentEvent::TaskFailed {
                                        agent_id,
                                        task_id,
                                        error: error_msg,
                                    }
                                }
                                Err(e) => {
                                    error!(%task_id, error = ?e, "Validation error, marking as failed");
                                    let error_msg = format!("Validation error: {}", e);
                                    if let Err(err) = task_coordinator.handle_task_failure(task_id, error_msg.clone()).await {
                                        error!(%task_id, error = ?err, "Failed to mark task as failed");
                                    }
                                    AgentEvent::TaskFailed {
                                        agent_id,
                                        task_id,
                                        error: error_msg,
                                    }
                                }
                            }
                        }

                        crate::domain::models::ValidationRequirement::Testing { .. } => {
                            // Spawn validation task for test-based validation
                            info!(%task_id, agent_type = %task.agent_type, "Spawning validation task");
                            match spawn_validation_task(task_id, &task, &task_coordinator).await {
                                Ok(validation_task_id) => {
                                    info!(%task_id, %validation_task_id, "Validation task spawned, marking as awaiting validation");
                                    // Task status is updated to AwaitingValidation in spawn_validation_task
                                    AgentEvent::ValidationRequested { agent_id, task_id }
                                }
                                Err(e) => {
                                    error!(%task_id, error = ?e, "Failed to spawn validation task, marking as failed");
                                    let error_msg = format!("Failed to spawn validation: {}", e);
                                    if let Err(err) = task_coordinator.handle_task_failure(task_id, error_msg.clone()).await {
                                        error!(%task_id, error = ?err, "Failed to mark task as failed");
                                    }
                                    AgentEvent::TaskFailed {
                                        agent_id,
                                        task_id,
                                        error: error_msg,
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(%task_id, error = ?e, "Task execution failed, marking as failed IMMEDIATELY");
                    let error_msg = e.to_string();
                    // Mark as failed IMMEDIATELY - don't wait for event loop
                    if let Err(err) = task_coordinator.handle_task_failure(task_id, error_msg.clone()).await {
                        error!(%task_id, error = ?err, "Failed to mark task as failed");
                    }
                    AgentEvent::TaskFailed {
                        agent_id,
                        task_id,
                        error: error_msg,
                    }
                }
            };

            // Send event for monitoring/stats (best-effort, don't fail if receiver dropped during shutdown)
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

    /// Check for and recover stale running tasks
    ///
    /// A task is considered stale if it has been running longer than its
    /// max_execution_timeout_seconds + STALE_TASK_BUFFER_SECS.
    /// This per-task timeout approach prevents false positives where legitimate
    /// long-running tasks were being marked stale with the old fixed threshold.
    async fn recover_stale_tasks(
        task_coordinator: Arc<TaskCoordinator>,
        in_flight_tasks: Arc<RwLock<HashSet<Uuid>>>,
    ) -> Result<()> {
        // Get ALL running tasks and filter by their individual timeouts
        // This is more correct than using a global threshold
        let running_tasks = task_coordinator
            .get_tasks_by_status(crate::domain::models::TaskStatus::Running)
            .await?;

        if running_tasks.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now();
        let mut stale_tasks = Vec::new();

        for task in running_tasks {
            if let Some(started_at) = task.started_at {
                // Calculate per-task stale threshold: timeout + buffer
                let task_threshold_secs = task.max_execution_timeout_seconds as i64
                    + Self::STALE_TASK_BUFFER_SECS as i64;
                let elapsed_secs = now.signed_duration_since(started_at).num_seconds();

                if elapsed_secs > task_threshold_secs {
                    info!(
                        task_id = %task.id,
                        elapsed_secs = elapsed_secs,
                        task_timeout = task.max_execution_timeout_seconds,
                        threshold = task_threshold_secs,
                        "Task exceeded its individual timeout threshold"
                    );
                    stale_tasks.push(task);
                }
            }
        }

        if stale_tasks.is_empty() {
            return Ok(());
        }

        info!(count = stale_tasks.len(), "Found stale running tasks to recover");

        for task in stale_tasks {
            // Remove from in-flight set if present (worker is gone anyway)
            {
                let mut in_flight = in_flight_tasks.write().await;
                in_flight.remove(&task.id);
            }

            // Recover the task (will mark as failed and trigger retry if available)
            if let Err(e) = task_coordinator.recover_stale_task(task.id).await {
                error!(
                    task_id = %task.id,
                    error = ?e,
                    "Failed to recover stale task"
                );
            }
        }

        Ok(())
    }

    /// Recover stuck chain handoffs
    ///
    /// Finds tasks that have `chain_handoff_state` set (indicating a pending handoff)
    /// that have been pending for longer than the configured timeout. For each stuck
    /// handoff, attempts to re-enqueue the next chain step.
    ///
    /// This prevents chains from hanging silently when:
    /// - The next step enqueue failed due to transient errors
    /// - The process crashed after saving handoff state but before enqueue
    ///
    /// After `max_attempts` failures, the handoff is marked as unrecoverable and
    /// cleared to prevent infinite retry loops.
    async fn recover_stuck_chain_handoffs(
        task_coordinator: Arc<TaskCoordinator>,
        agent_executor: Arc<AgentExecutor>,
        timeout_secs: u64,
        max_attempts: u32,
    ) -> Result<()> {
        // Get completed tasks that have chain_handoff_state set
        // These are tasks where the step completed but next step wasn't enqueued
        let completed_tasks = task_coordinator
            .get_tasks_by_status(crate::domain::models::TaskStatus::Completed)
            .await?;

        let now = chrono::Utc::now();
        let mut stuck_handoffs = Vec::new();

        for task in completed_tasks {
            if let Some(ref handoff_state) = task.chain_handoff_state {
                let elapsed_secs = now.signed_duration_since(handoff_state.pending_since).num_seconds();
                if elapsed_secs > timeout_secs as i64 {
                    info!(
                        task_id = %task.id,
                        chain_id = %handoff_state.chain_id,
                        pending_step_index = handoff_state.pending_next_step_index,
                        elapsed_secs = elapsed_secs,
                        enqueue_attempts = handoff_state.enqueue_attempts,
                        last_error = ?handoff_state.last_error,
                        "Found stuck chain handoff"
                    );
                    stuck_handoffs.push(task);
                }
            }
        }

        if stuck_handoffs.is_empty() {
            return Ok(());
        }

        info!(
            count = stuck_handoffs.len(),
            "Found stuck chain handoffs to recover"
        );

        for mut task in stuck_handoffs {
            let handoff_state = task.chain_handoff_state.clone().expect("checked above");

            // Check if we've exceeded max attempts
            if handoff_state.enqueue_attempts >= max_attempts {
                error!(
                    task_id = %task.id,
                    chain_id = %handoff_state.chain_id,
                    step_index = handoff_state.pending_next_step_index,
                    attempts = handoff_state.enqueue_attempts,
                    max_attempts = max_attempts,
                    last_error = ?handoff_state.last_error,
                    "Chain handoff exceeded max recovery attempts, marking as unrecoverable"
                );

                // Clear the handoff state and log the failure
                // The chain is now permanently stuck at this step, but the task is marked
                // complete so dependent tasks can still be analyzed
                task.chain_handoff_state = None;
                task.error_message = Some(format!(
                    "Chain handoff to step {} failed after {} attempts. Last error: {}",
                    handoff_state.pending_next_step_index,
                    handoff_state.enqueue_attempts,
                    handoff_state.last_error.as_deref().unwrap_or("unknown")
                ));

                if let Err(e) = task_coordinator.update_task(&task).await {
                    warn!(
                        task_id = %task.id,
                        error = ?e,
                        "Failed to mark chain handoff as unrecoverable"
                    );
                }
                continue;
            }

            // Attempt to retry the handoff
            match agent_executor
                .retry_chain_handoff(&task, &handoff_state)
                .await
            {
                Ok(_) => {
                    info!(
                        task_id = %task.id,
                        chain_id = %handoff_state.chain_id,
                        step_index = handoff_state.pending_next_step_index,
                        "Successfully recovered stuck chain handoff"
                    );
                    // Clear the handoff state
                    task.chain_handoff_state = None;
                    if let Err(e) = task_coordinator.update_task(&task).await {
                        warn!(
                            task_id = %task.id,
                            error = ?e,
                            "Failed to clear handoff state after successful recovery"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        task_id = %task.id,
                        chain_id = %handoff_state.chain_id,
                        step_index = handoff_state.pending_next_step_index,
                        error = ?e,
                        attempt = handoff_state.enqueue_attempts + 1,
                        max_attempts = max_attempts,
                        "Failed to recover stuck chain handoff, will retry later"
                    );
                    // Update the handoff state with the error and increment attempts
                    if let Some(ref mut state) = task.chain_handoff_state {
                        state.enqueue_attempts += 1;
                        state.last_error = Some(e.to_string());
                    }
                    let _ = task_coordinator.update_task(&task).await;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::resource_monitor::ResourceLimits;
    use crate::domain::models::{DependencyType, TaskSource, TaskStatus, ValidationRequirement};
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

        async fn get_children_by_parent(&self, _parent_id: Uuid) -> Result<Vec<Task>> {
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

        async fn update_task(&self, task: &Task) -> Result<()> {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task.id, task.clone());
            Ok(())
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

        async fn claim_next_ready_task(&self) -> Result<Option<Task>> {
            let mut tasks = self.tasks.lock().unwrap();
            let task = tasks
                .values()
                .filter(|t| t.status == TaskStatus::Ready)
                .max_by(|a, b| {
                    a.calculated_priority
                        .partial_cmp(&b.calculated_priority)
                        .unwrap()
                })
                .cloned();

            // Atomically mark as running
            if let Some(ref t) = task {
                if let Some(task_mut) = tasks.get_mut(&t.id) {
                    task_mut.status = TaskStatus::Running;
                }
            }

            Ok(task)
        }

        async fn submit_task(&self, task: Task) -> Result<Uuid> {
            let task_id = task.id;
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task_id, task);
            Ok(task_id)
        }

        async fn get_stale_running_tasks(&self, _stale_threshold_secs: u64) -> Result<Vec<Task>> {
            Ok(vec![]) // Mock returns no stale tasks
        }

        async fn task_exists_by_idempotency_key(&self, _idempotency_key: &str) -> Result<bool> {
            Ok(false) // Mock returns no existing tasks
        }

        async fn get_task_by_idempotency_key(&self, _idempotency_key: &str) -> Result<Option<Task>> {
            Ok(None) // Mock returns no existing tasks
        }

        async fn submit_task_idempotent(&self, task: Task) -> Result<crate::domain::ports::task_repository::IdempotentInsertResult> {
            use crate::domain::ports::task_repository::IdempotentInsertResult;
            let task_id = task.id;
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task_id, task);
            Ok(IdempotentInsertResult::Inserted(task_id))
        }

        async fn submit_tasks_transactional(&self, tasks_to_insert: Vec<Task>) -> Result<crate::domain::ports::task_repository::BatchInsertResult> {
            use crate::domain::ports::task_repository::BatchInsertResult;
            let mut result = BatchInsertResult::new();
            let mut tasks = self.tasks.lock().unwrap();
            for task in tasks_to_insert {
                let task_id = task.id;
                tasks.insert(task_id, task);
                result.inserted.push(task_id);
            }
            Ok(result)
        }

        async fn resolve_dependencies_for_completed_task(&self, _completed_task_id: Uuid) -> Result<usize> {
            Ok(0) // Mock returns 0 tasks updated
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
            branch: None,
            worktree_path: None,
            validation_requirement: ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
            chain_step_index: 0,
            chain_handoff_state: None,
            idempotency_key: None,
            version: 1,
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

use crate::domain::models::task::{Task, TaskStatus};
use crate::domain::models::{HookContext, HookEvent, HookResult};
use crate::domain::ports::{PriorityCalculator, TaskQueueService};
use crate::services::hook_executor::HookExecutor;
use crate::services::hook_registry::HookRegistry;
use crate::services::worktree_service::WorktreeService;
use crate::services::DependencyResolver;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

/// Default timeout for hook execution (60 seconds)
/// Hooks that take longer than this will be cancelled to prevent indefinite hangs
const HOOK_TIMEOUT_SECS: u64 = 60;

/// Status update message for task lifecycle events
#[derive(Debug, Clone)]
pub struct TaskStatusUpdate {
    pub task_id: Uuid,
    pub old_status: TaskStatus,
    pub new_status: TaskStatus,
}

/// Result of task completion operation
///
/// Distinguishes between primary success/failure (marking task completed)
/// and secondary issues (dependency coordination failures).
#[derive(Debug)]
pub enum TaskCompletionResult {
    /// Task completed successfully and all dependencies were coordinated
    Success,
    /// Task completed, but some dependency coordination failed.
    /// These will be recovered by background monitoring (30s interval).
    /// Contains the list of failed dependent task IDs.
    CompletedWithDependencyFailures(Vec<Uuid>),
}

/// Error type for task completion failures
#[derive(Debug, thiserror::Error)]
pub enum TaskCompletionError {
    /// Failed to mark the task as completed in the database
    #[error("Failed to mark task as completed: {0}")]
    MarkCompletedFailed(String),

    /// Pre-complete hook blocked the completion
    #[error("Pre-complete hook blocked task completion")]
    HookBlocked,

    /// Task not found
    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    /// Other error
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Coordinates task lifecycle, dependency resolution, and priority scheduling
///
/// The `TaskCoordinator` is the central orchestration component that:
/// - Resolves task dependencies using the `DependencyResolver`
/// - Calculates and updates task priorities using the `PriorityCalculator`
/// - Manages task status transitions through the `TaskQueueService`
/// - Triggers dependent tasks when prerequisites complete
/// - Handles task failures and cascading effects
///
/// # Concurrency Design
///
/// Uses tokio async runtime with:
/// - Arc-wrapped trait objects for dependency injection
/// - mpsc channels for status update notifications
/// - Async methods for all I/O operations
///
/// # Examples
///
/// ```no_run
/// use abathur::application::TaskCoordinator;
/// use abathur::domain::ports::{TaskQueueService, PriorityCalculator};
/// use abathur::services::DependencyResolver;
/// use std::sync::Arc;
/// use uuid::Uuid;
/// use anyhow::Result;
///
/// async fn example(
///     task_queue: Arc<dyn TaskQueueService>,
///     dependency_resolver: Arc<DependencyResolver>,
///     priority_calc: Arc<dyn PriorityCalculator>,
///     task_id: Uuid,
/// ) -> Result<()> {
///     let coordinator = TaskCoordinator::new(
///         task_queue,
///         dependency_resolver,
///         priority_calc,
///     );
///
///     // Coordinate a task through its lifecycle
///     coordinator.coordinate_task_lifecycle(task_id).await?;
///
///     // Get the next ready task
///     let next_task = coordinator.get_next_ready_task().await?;
///     Ok(())
/// }
/// ```
pub struct TaskCoordinator {
    task_queue: Arc<dyn TaskQueueService>,
    #[allow(dead_code)] // Reserved for future complex dependency resolution
    dependency_resolver: Arc<DependencyResolver>,
    priority_calc: Arc<dyn PriorityCalculator>,
    status_tx: mpsc::Sender<TaskStatusUpdate>,
    status_rx: Option<mpsc::Receiver<TaskStatusUpdate>>,
    hook_registry: Arc<RwLock<Option<Arc<HookRegistry>>>>,
    worktree_service: Arc<RwLock<Option<Arc<WorktreeService>>>>,
}

impl TaskCoordinator {
    /// Create a new `TaskCoordinator` with dependency injection
    ///
    /// # Arguments
    ///
    /// * `task_queue` - Service for task storage and retrieval
    /// * `dependency_resolver` - Service for dependency resolution
    /// * `priority_calc` - Service for priority calculation
    ///
    /// # Returns
    ///
    /// A new `TaskCoordinator` instance with a status update channel (buffer size: 1000)
    pub fn new(
        task_queue: Arc<dyn TaskQueueService>,
        dependency_resolver: Arc<DependencyResolver>,
        priority_calc: Arc<dyn PriorityCalculator>,
    ) -> Self {
        let (status_tx, status_rx) = mpsc::channel(1000);

        Self {
            task_queue,
            dependency_resolver,
            priority_calc,
            status_tx,
            status_rx: Some(status_rx),
            hook_registry: Arc::new(RwLock::new(None)),
            worktree_service: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the worktree service for this coordinator
    ///
    /// This must be called after construction to enable automatic worktree
    /// creation when tasks start running.
    pub async fn set_worktree_service(&self, worktree_service: Arc<WorktreeService>) {
        let mut service = self.worktree_service.write().await;
        *service = Some(worktree_service);
    }

    /// Set the hook registry for this coordinator
    ///
    /// This must be called after construction to enable hook execution.
    /// It's separate from the constructor to avoid circular dependencies
    /// (HookExecutor needs TaskCoordinator, TaskCoordinator needs HookRegistry).
    pub async fn set_hook_registry(&self, hook_registry: Arc<HookRegistry>) {
        let mut registry = self.hook_registry.write().await;
        *registry = Some(hook_registry);
    }

    /// Get a handle to send status updates
    ///
    /// Returns a clone of the status update sender for external components
    /// to publish task status changes.
    pub fn status_sender(&self) -> mpsc::Sender<TaskStatusUpdate> {
        self.status_tx.clone()
    }

    /// Take ownership of the status update receiver
    ///
    /// This should be called once to start the background status monitoring task.
    /// Returns None if the receiver has already been taken.
    pub const fn take_status_receiver(&mut self) -> Option<mpsc::Receiver<TaskStatusUpdate>> {
        self.status_rx.take()
    }

    /// Execute hooks with a timeout to prevent indefinite hangs
    ///
    /// Wraps hook execution in a tokio timeout to ensure hooks cannot block
    /// the task coordinator indefinitely. If a hook times out, it returns
    /// an error that can be handled appropriately.
    ///
    /// # Arguments
    /// * `registry` - The hook registry to execute hooks from
    /// * `event` - The hook event type (PreReady, PostReady, etc.)
    /// * `task` - The task being processed
    /// * `context` - The hook context with variables
    ///
    /// # Returns
    /// * `Ok(HookResult)` - If hooks complete within the timeout
    /// * `Err` - If hooks time out or fail
    async fn execute_hooks_with_timeout(
        registry: &HookRegistry,
        event: HookEvent,
        task: &Task,
        context: &HookContext,
    ) -> Result<HookResult> {
        tokio::time::timeout(
            Duration::from_secs(HOOK_TIMEOUT_SECS),
            registry.execute_hooks(event.clone(), task, context),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Hook execution timed out after {}s for event {:?} on task {}",
                HOOK_TIMEOUT_SECS,
                event,
                task.id
            )
        })?
        .context(format!("Failed to execute {:?} hooks", event))
    }

    /// Coordinate the complete lifecycle of a task
    ///
    /// Orchestrates:
    /// 1. Dependency resolution (check if all prerequisites are met)
    /// 2. Priority calculation and update
    /// 3. Status transition (pending -> blocked/ready)
    /// 4. Triggering dependent tasks on completion
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to coordinate
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task lifecycle coordinated successfully
    /// * `Err` - If task not found, dependency resolution fails, or database error
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Task does not exist
    /// - Dependency resolution fails (circular dependencies, missing tasks)
    /// - Priority calculation fails
    /// - Database operations fail
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn coordinate_task_lifecycle(&self, task_id: Uuid) -> Result<()> {
        info!("Coordinating task lifecycle for task {}", task_id);

        // 1. Get the task
        let task = self
            .task_queue
            .get_task(task_id)
            .await
            .context("Failed to get task from queue")?;

        // 2. Check if dependencies are resolved
        let dependencies_met = self.check_dependencies_met(&task).await?;

        // 3. Calculate priority
        let new_priority = self
            .priority_calc
            .calculate_priority(&task)
            .await
            .context("Failed to calculate task priority")?;

        // 4. Update priority in database
        self.task_queue
            .update_task_priority(task_id, new_priority)
            .await
            .context("Failed to update task priority")?;

        // 5. Update task status based on dependencies
        let new_status = if dependencies_met {
            TaskStatus::Ready
        } else {
            TaskStatus::Blocked
        };

        if task.status != new_status && new_status == TaskStatus::Ready {
            // Execute PreReady hooks with timeout
            let hook_registry_guard = self.hook_registry.read().await;
            if let Some(ref registry) = *hook_registry_guard {
                let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
                let hook_result = Self::execute_hooks_with_timeout(registry, HookEvent::PreReady, &task, &context).await?;

                if hook_result.should_block() {
                    warn!("PreReady hook blocked task {} from becoming ready", task_id);
                    return Ok(());
                }
            }
            drop(hook_registry_guard);

            // Update status to Ready
            self.task_queue
                .update_task_status(task_id, new_status)
                .await
                .context("Failed to update task status")?;

            // Notify status change
            let _ = self
                .status_tx
                .send(TaskStatusUpdate {
                    task_id,
                    old_status: task.status,
                    new_status,
                })
                .await;

            info!("Task {} transitioned to {:?}", task_id, new_status);

            // Execute PostReady hooks with timeout (non-blocking, log errors)
            let hook_registry_guard = self.hook_registry.read().await;
            if let Some(ref registry) = *hook_registry_guard {
                let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
                if let Err(e) = Self::execute_hooks_with_timeout(registry, HookEvent::PostReady, &task, &context).await {
                    warn!(task_id = %task_id, error = ?e, "PostReady hook execution failed or timed out");
                }
            }
            drop(hook_registry_guard);
        } else if task.status != new_status {
            // Non-ready status transitions (e.g., to Blocked)
            self.task_queue
                .update_task_status(task_id, new_status)
                .await
                .context("Failed to update task status")?;

            // Notify status change
            let _ = self
                .status_tx
                .send(TaskStatusUpdate {
                    task_id,
                    old_status: task.status,
                    new_status,
                })
                .await;

            info!("Task {} transitioned to {:?}", task_id, new_status);
        }

        Ok(())
    }

    /// Get the next ready task with highest priority
    ///
    /// Retrieves the task with status "ready" that has the highest calculated priority.
    /// This is used by the agent pool to pull the next task to execute.
    ///
    /// NOTE: This does NOT claim the task. Use `claim_next_ready_task` for
    /// atomic claim to prevent race conditions in multi-worker scenarios.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(Task))` - The highest priority ready task
    /// * `Ok(None)` - No ready tasks available
    /// * `Err` - If database error
    #[instrument(skip(self))]
    pub async fn get_next_ready_task(&self) -> Result<Option<Task>> {
        self.task_queue
            .get_next_ready_task()
            .await
            .context("Failed to get next ready task")
    }

    /// Atomically claim the next ready task for execution
    ///
    /// This performs an atomic SELECT + UPDATE operation to:
    /// 1. Find the highest-priority task with status=Ready
    /// 2. Immediately mark it as Running
    /// 3. Return the claimed task
    ///
    /// This prevents race conditions where multiple workers pick up the same task.
    /// The returned task is already marked as Running in the database.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(task))` - The claimed task (already marked as Running)
    /// * `Ok(None)` - No ready tasks available
    /// * `Err` - If database error occurs
    #[instrument(skip(self))]
    pub async fn claim_next_ready_task(&self) -> Result<Option<Task>> {
        // Atomically claim the task (marks as Running in DB)
        let task = self.task_queue
            .claim_next_ready_task()
            .await
            .context("Failed to atomically claim next ready task")?;

        // If we claimed a task, set up worktree and run hooks
        if let Some(mut task) = task {
            let task_id = task.id;

            // Set up worktree if the task needs one (has feature_branch)
            // This updates both the in-memory task and the database
            let worktree_service_guard = self.worktree_service.read().await;
            if let Some(ref worktree_service) = *worktree_service_guard {
                match worktree_service.setup_task_worktree(&mut task).await {
                    Ok(true) => {
                        info!(task_id = %task_id, "Task worktree created successfully");

                        // CRITICAL: Validate worktree actually exists after setup
                        // This catches cases where git commands failed silently or the
                        // worktree was cleaned up by another process
                        if let Err(e) = worktree_service.validate_worktree_exists(&task).await {
                            error!(task_id = %task_id, error = ?e, "Worktree validation failed after setup");
                            self.task_queue
                                .mark_task_failed(
                                    task_id,
                                    format!("Worktree validation failed: {}", e)
                                )
                                .await
                                .context("Failed to mark task as failed after worktree validation failure")?;
                            return Err(e.context("Worktree validation failed after setup"));
                        }
                    }
                    Ok(false) => {
                        debug!(task_id = %task_id, "Task does not need worktree (no feature_branch)");
                    }
                    Err(e) => {
                        // Worktree creation failed
                        error!(task_id = %task_id, error = ?e, version = task.version, "Failed to create task worktree");

                        // SAFEGUARD: If version is very high, it means this task has been
                        // repeatedly failing in a loop (claim→fail→revert→claim→...).
                        // Mark as Failed to break the loop instead of reverting to Ready.
                        const MAX_REVERT_VERSION: u32 = 100;
                        if task.version > MAX_REVERT_VERSION {
                            error!(
                                task_id = %task_id,
                                version = task.version,
                                "Task version too high, marking as failed to break infinite retry loop"
                            );
                            self.task_queue
                                .mark_task_failed(
                                    task_id,
                                    format!("Worktree creation failed repeatedly (version {}): {}", task.version, e)
                                )
                                .await
                                .context("Failed to mark task as failed after repeated worktree failures")?;
                        } else {
                            // Normal case: revert to Ready and return error
                            self.task_queue
                                .update_task_status(task_id, TaskStatus::Ready)
                                .await
                                .context("Failed to revert task status to Ready after worktree failure")?;
                        }
                        return Err(e.context("Failed to set up task worktree"));
                    }
                }
            }
            drop(worktree_service_guard);

            // Execute PreStart hooks with timeout
            let hook_registry_guard = self.hook_registry.read().await;
            if let Some(ref registry) = *hook_registry_guard {
                let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
                let hook_result = Self::execute_hooks_with_timeout(registry, HookEvent::PreStart, &task, &context).await?;

                if hook_result.should_block() {
                    warn!("PreStart hook blocked task {} - reverting to Ready status", task_id);

                    // SAFEGUARD: If version is very high, mark as failed instead of reverting
                    const MAX_REVERT_VERSION: u32 = 100;
                    if task.version > MAX_REVERT_VERSION {
                        error!(
                            task_id = %task_id,
                            version = task.version,
                            "Task version too high, marking as failed to break infinite hook block loop"
                        );
                        self.task_queue
                            .mark_task_failed(
                                task_id,
                                format!("PreStart hook blocked repeatedly (version {})", task.version)
                            )
                            .await
                            .context("Failed to mark task as failed after repeated hook blocks")?;
                    } else {
                        // Revert status to Ready if hook blocks
                        self.task_queue
                            .update_task_status(task_id, TaskStatus::Ready)
                            .await
                            .context("Failed to revert task status to Ready")?;
                    }
                    return Ok(None);
                }
            }
            drop(hook_registry_guard);

            // Execute PostStart hooks with timeout (non-blocking, log errors)
            let hook_registry_guard = self.hook_registry.read().await;
            if let Some(ref registry) = *hook_registry_guard {
                let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
                if let Err(e) = Self::execute_hooks_with_timeout(registry, HookEvent::PostStart, &task, &context).await {
                    warn!(task_id = %task_id, error = ?e, "PostStart hook execution failed or timed out");
                }
            }
            drop(hook_registry_guard);

            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    /// Mark a task as running
    ///
    /// Updates the task status to Running when an agent begins execution.
    /// This is called by the orchestrator when spawning an agent worker.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to mark as running
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task marked as running successfully
    /// * `Err` - If task not found or database error
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn mark_task_running(&self, task_id: Uuid) -> Result<()> {
        // Get the task for hook execution and worktree setup
        let mut task = self.task_queue.get_task(task_id).await
            .context("Failed to get task for mark_task_running")?;

        // CRITICAL: Update status to Running FIRST, BEFORE worktree/hooks execute.
        // This prevents race conditions where the polling loop picks up the same
        // task while worktree creation or hooks are still executing.
        self.task_queue
            .update_task_status(task_id, TaskStatus::Running)
            .await
            .context("Failed to mark task as running")?;

        // Notify status change
        let _ = self
            .status_tx
            .send(TaskStatusUpdate {
                task_id,
                old_status: TaskStatus::Ready,
                new_status: TaskStatus::Running,
            })
            .await;

        // Set up worktree if the task needs one (has feature_branch)
        // This updates both the in-memory task and the database
        let worktree_service_guard = self.worktree_service.read().await;
        if let Some(ref worktree_service) = *worktree_service_guard {
            match worktree_service.setup_task_worktree(&mut task).await {
                Ok(true) => {
                    info!(task_id = %task_id, "Task worktree created successfully");

                    // CRITICAL: Validate worktree actually exists after setup
                    // This catches cases where git commands failed silently or the
                    // worktree was cleaned up by another process
                    if let Err(e) = worktree_service.validate_worktree_exists(&task).await {
                        error!(task_id = %task_id, error = ?e, "Worktree validation failed after setup");
                        self.task_queue
                            .mark_task_failed(
                                task_id,
                                format!("Worktree validation failed: {}", e)
                            )
                            .await
                            .context("Failed to mark task as failed after worktree validation failure")?;
                        return Err(e.context("Worktree validation failed after setup"));
                    }
                }
                Ok(false) => {
                    debug!(task_id = %task_id, "Task does not need worktree (no feature_branch)");
                }
                Err(e) => {
                    // Worktree creation failed
                    error!(task_id = %task_id, error = ?e, version = task.version, "Failed to create task worktree");

                    // SAFEGUARD: If version is very high, mark as failed instead of reverting
                    const MAX_REVERT_VERSION: u32 = 100;
                    if task.version > MAX_REVERT_VERSION {
                        error!(
                            task_id = %task_id,
                            version = task.version,
                            "Task version too high, marking as failed to break infinite retry loop"
                        );
                        self.task_queue
                            .mark_task_failed(
                                task_id,
                                format!("Worktree creation failed repeatedly (version {}): {}", task.version, e)
                            )
                            .await
                            .context("Failed to mark task as failed after repeated worktree failures")?;
                    } else {
                        self.task_queue
                            .update_task_status(task_id, TaskStatus::Ready)
                            .await
                            .context("Failed to revert task status to Ready after worktree failure")?;
                    }
                    return Err(e.context("Failed to set up task worktree"));
                }
            }
        }
        drop(worktree_service_guard);

        // Execute PreStart hooks with timeout (after status is already Running and worktree is ready)
        let hook_registry_guard = self.hook_registry.read().await;
        if let Some(ref registry) = *hook_registry_guard {
            let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
            let hook_result = Self::execute_hooks_with_timeout(registry, HookEvent::PreStart, &task, &context).await?;

            if hook_result.should_block() {
                warn!("PreStart hook blocked task {} - reverting to Ready status", task_id);

                // SAFEGUARD: If version is very high, mark as failed instead of reverting
                const MAX_REVERT_VERSION: u32 = 100;
                if task.version > MAX_REVERT_VERSION {
                    error!(
                        task_id = %task_id,
                        version = task.version,
                        "Task version too high, marking as failed to break infinite hook block loop"
                    );
                    self.task_queue
                        .mark_task_failed(
                            task_id,
                            format!("PreStart hook blocked repeatedly (version {})", task.version)
                        )
                        .await
                        .context("Failed to mark task as failed after repeated hook blocks")?;
                } else {
                    self.task_queue
                        .update_task_status(task_id, TaskStatus::Ready)
                        .await
                        .context("Failed to revert task status to Ready")?;
                }
                return Ok(());
            }
        }
        drop(hook_registry_guard);

        // Execute PostStart hooks with timeout (non-blocking, log errors)
        let hook_registry_guard = self.hook_registry.read().await;
        if let Some(ref registry) = *hook_registry_guard {
            let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
            if let Err(e) = Self::execute_hooks_with_timeout(registry, HookEvent::PostStart, &task, &context).await {
                warn!(task_id = %task_id, error = ?e, "PostStart hook execution failed or timed out");
            }
        }
        drop(hook_registry_guard);

        Ok(())
    }

    /// Handle task completion and trigger dependent tasks
    ///
    /// When a task completes successfully:
    /// 1. Mark the task as completed
    /// 2. Find all tasks that depend on this task
    /// 3. Re-check dependencies for dependent tasks
    /// 4. Update their status (blocked -> ready) if dependencies are now met
    /// 5. Recalculate priorities for affected tasks
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task that completed
    ///
    /// # Returns
    ///
    /// * `Ok(TaskCompletionResult::Success)` - Task marked complete and all dependents triggered
    /// * `Ok(TaskCompletionResult::CompletedWithDependencyFailures)` - Task marked complete but
    ///   some dependency coordination failed. These will be recovered by background monitoring.
    /// * `Err(TaskCompletionError::MarkCompletedFailed)` - Failed to mark task as completed (serious)
    /// * `Err(TaskCompletionError::HookBlocked)` - Pre-complete hook blocked the completion
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn handle_task_completion(&self, task_id: Uuid) -> std::result::Result<TaskCompletionResult, TaskCompletionError> {
        info!("Handling task completion for task {}", task_id);

        // Get the task for hook execution
        let task = self.task_queue.get_task(task_id).await
            .map_err(|e| TaskCompletionError::Other(e.context("Failed to get task for handle_task_completion")))?;

        // Execute PreComplete hooks with timeout
        let hook_registry_guard = self.hook_registry.read().await;
        if let Some(ref registry) = *hook_registry_guard {
            let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
            let hook_result = Self::execute_hooks_with_timeout(registry, HookEvent::PreComplete, &task, &context).await
                .map_err(|e| TaskCompletionError::Other(e.context("PreComplete hooks failed or timed out")))?;

            if hook_result.should_block() {
                warn!("PreComplete hook blocked task {} from completing", task_id);
                return Err(TaskCompletionError::HookBlocked);
            }
        }
        drop(hook_registry_guard);

        // 1. Mark task as completed - THIS IS THE CRITICAL OPERATION
        // If this fails, the task status is inconsistent
        self.task_queue
            .update_task_status(task_id, TaskStatus::Completed)
            .await
            .map_err(|e| TaskCompletionError::MarkCompletedFailed(e.to_string()))?;

        // Notify status change
        let _ = self
            .status_tx
            .send(TaskStatusUpdate {
                task_id,
                old_status: TaskStatus::Running,
                new_status: TaskStatus::Completed,
            })
            .await;

        // Execute PostComplete hooks with timeout (non-blocking, log errors)
        let hook_registry_guard = self.hook_registry.read().await;
        if let Some(ref registry) = *hook_registry_guard {
            let context = HookContext::from_task(task_id, HookExecutor::build_variables(&task, &HookContext::from_task(task_id, std::collections::HashMap::new())));
            if let Err(e) = Self::execute_hooks_with_timeout(registry, HookEvent::PostComplete, &task, &context).await {
                warn!(task_id = %task_id, error = ?e, "PostComplete hook execution failed or timed out");
            }
        }
        drop(hook_registry_guard);

        // Check if this is a decomposition child task and handle parent completion
        // This implements fan-in for fan-out/fan-in decomposition pattern
        if let Err(e) = self.check_parent_child_completion(task_id).await {
            warn!(
                task_id = %task_id,
                error = ?e,
                "Failed to check parent child completion, parent may be stuck in AwaitingChildren"
            );
            // Don't fail the overall completion - parent can be recovered by background process
        }

        // 2. Get all dependent tasks
        let dependent_tasks = self
            .task_queue
            .get_dependent_tasks(task_id)
            .await
            .map_err(|e| TaskCompletionError::Other(e.context("Failed to get dependent tasks")))?;

        info!(
            "Found {} dependent tasks for task {}",
            dependent_tasks.len(),
            task_id
        );

        // 3. Trigger lifecycle coordination for each dependent task with retry logic
        // This is critical - if coordination fails, dependent tasks may be stuck forever
        // Use aggressive retries: 5 attempts with faster initial backoff (50ms)
        const MAX_RETRIES: u32 = 5;
        const INITIAL_BACKOFF_MS: u64 = 50;

        let mut failed_coordinations: Vec<Uuid> = Vec::new();

        for dependent_task in dependent_tasks {
            let mut last_error = None;
            let mut success = false;

            for attempt in 0..MAX_RETRIES {
                match self.coordinate_task_lifecycle(dependent_task.id).await {
                    Ok(()) => {
                        if attempt > 0 {
                            info!(
                                "Coordinated dependent task {} after {} retries",
                                dependent_task.id, attempt
                            );
                        }
                        success = true;
                        break;
                    }
                    Err(e) => {
                        let backoff_ms = INITIAL_BACKOFF_MS * (1 << attempt); // Exponential backoff: 50, 100, 200, 400, 800ms
                        warn!(
                            "Failed to coordinate dependent task {} (attempt {}/{}): {:?}. Retrying in {}ms",
                            dependent_task.id, attempt + 1, MAX_RETRIES, e, backoff_ms
                        );
                        last_error = Some(e.to_string());

                        if attempt < MAX_RETRIES - 1 {
                            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                        }
                    }
                }
            }

            if !success {
                let error_msg = last_error.unwrap_or_else(|| "Unknown error".to_string());
                error!(
                    "Failed to coordinate dependent task {} after {} retries: {}",
                    dependent_task.id, MAX_RETRIES, error_msg
                );
                failed_coordinations.push(dependent_task.id);
            }
        }

        // If coordinations failed, attempt immediate dependency resolution as fallback
        // This directly resolves dependencies without going through full lifecycle coordination
        if !failed_coordinations.is_empty() {
            info!(
                "Attempting immediate dependency resolution for {} failed coordination(s)",
                failed_coordinations.len()
            );

            // Try resolving through the task queue service directly
            match self.task_queue.resolve_dependencies_for_completed_task(task_id).await {
                Ok(resolved_count) => {
                    if resolved_count > 0 {
                        info!(
                            "Immediate resolution succeeded: {} tasks became ready after {} completion",
                            resolved_count, task_id
                        );
                        // Clear failed coordinations if we resolved them
                        failed_coordinations.clear();
                    }
                }
                Err(e) => {
                    warn!(
                        "Immediate dependency resolution also failed: {:?}. Falling back to background monitor.",
                        e
                    );
                }
            }
        }

        // Return result indicating whether all dependencies were coordinated
        // NOTE: The parent task IS completed at this point. Failed coordinations will be
        // recovered by the background blocked task recovery (configurable interval).
        if failed_coordinations.is_empty() {
            Ok(TaskCompletionResult::Success)
        } else {
            error!(
                "Failed to coordinate {} dependent task(s) after retries and fallback: {:?}. Will be recovered by background monitoring.",
                failed_coordinations.len(),
                failed_coordinations
            );
            Ok(TaskCompletionResult::CompletedWithDependencyFailures(failed_coordinations))
        }
    }

    /// Handle task failure with optional retry logic
    ///
    /// When a task fails:
    /// 1. Mark the task as failed with error message
    /// 2. Optionally implement retry logic (future enhancement)
    /// 3. Optionally cascade failure to dependent tasks (future enhancement)
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task that failed
    /// * `error_message` - Description of the failure
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task marked as failed
    /// * `Err` - If task not found or database error
    #[instrument(skip(self), fields(task_id = %task_id, error = %error_message))]
    pub async fn handle_task_failure(&self, task_id: Uuid, error_message: String) -> Result<()> {
        error!(
            "Handling task failure for task {}: {}",
            task_id, error_message
        );

        // Mark task as failed
        self.task_queue
            .mark_task_failed(task_id, error_message)
            .await
            .context("Failed to mark task as failed")?;

        // Notify status change
        let _ = self
            .status_tx
            .send(TaskStatusUpdate {
                task_id,
                old_status: TaskStatus::Running,
                new_status: TaskStatus::Failed,
            })
            .await;

        // Implement retry logic
        let mut task = self
            .task_queue
            .get_task(task_id)
            .await
            .context("Failed to get task for retry check")?;

        if task.can_retry() {
            // Task can be retried - increment retry count and reset to Pending
            info!(
                task_id = %task_id,
                retry_count = task.retry_count,
                max_retries = task.max_retries,
                "Task failed but can be retried, resetting to Pending"
            );

            // Use the domain model's retry method to handle state transition
            // This increments retry_count and sets status to Pending
            task.retry()
                .context("Failed to transition task to retry state")?;

            // Update the FULL task in the database to persist the incremented retry_count
            // Previously this only updated status, losing the retry_count increment
            self.task_queue
                .update_task(&task)
                .await
                .context("Failed to update task for retry")?;

            // Re-coordinate the task lifecycle to check dependencies and set priority
            self.coordinate_task_lifecycle(task_id)
                .await
                .context("Failed to re-coordinate task after retry")?;

            info!(
                task_id = %task_id,
                retry_count = task.retry_count,
                "Task retry initiated successfully"
            );
        } else {
            // Max retries exceeded or cannot retry
            warn!(
                task_id = %task_id,
                retry_count = task.retry_count,
                max_retries = task.max_retries,
                "Task failed and max retries exceeded, implementing cascade failure"
            );

            // Implement cascade failure logic
            // Get all dependent tasks
            let dependent_tasks = self
                .task_queue
                .get_dependent_tasks(task_id)
                .await
                .context("Failed to get dependent tasks for cascade failure")?;

            if !dependent_tasks.is_empty() {
                info!(
                    task_id = %task_id,
                    dependent_count = dependent_tasks.len(),
                    "Cascading failure to dependent tasks"
                );

                // Mark all dependent tasks as failed due to dependency failure
                for dependent_task in dependent_tasks {
                    if !dependent_task.is_terminal() {
                        let cascade_error = format!(
                            "Dependency task {} failed after max retries: {}",
                            task_id,
                            task.error_message.as_deref().unwrap_or("unknown error")
                        );

                        warn!(
                            dependent_task_id = %dependent_task.id,
                            "Marking dependent task as failed due to dependency failure"
                        );

                        // Mark dependent task as failed
                        self.task_queue
                            .mark_task_failed(dependent_task.id, cascade_error)
                            .await
                            .context("Failed to mark dependent task as failed")?;

                        // Recursively handle cascade failures for this dependent task
                        // (without retries since this is a cascade failure)
                        self.cascade_failure_to_dependents(dependent_task.id)
                            .await
                            .context("Failed to cascade failure to nested dependents")?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Cascade failure to dependent tasks recursively
    ///
    /// Marks all dependent tasks as failed when a dependency fails.
    /// This is called recursively to handle nested dependencies.
    ///
    /// # Arguments
    ///
    /// * `failed_task_id` - UUID of the task that failed
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Cascade completed
    /// * `Err` - If database operations fail
    fn cascade_failure_to_dependents<'a>(
        &'a self,
        failed_task_id: Uuid,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let dependent_tasks = self
                .task_queue
                .get_dependent_tasks(failed_task_id)
                .await
                .context("Failed to get dependent tasks for cascade")?;

            for dependent_task in dependent_tasks {
                if !dependent_task.is_terminal() {
                    let cascade_error = format!(
                        "Dependency task {} failed (cascade failure)",
                        failed_task_id
                    );

                    debug!(
                        dependent_task_id = %dependent_task.id,
                        failed_task_id = %failed_task_id,
                        "Cascading failure to dependent task"
                    );

                    self.task_queue
                        .mark_task_failed(dependent_task.id, cascade_error)
                        .await
                        .context("Failed to mark cascaded dependent task as failed")?;

                    // Recursively cascade to nested dependents
                    self.cascade_failure_to_dependents(dependent_task.id)
                        .await?;
                }
            }

            Ok(())
        })
    }

    /// Get a task by ID
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to retrieve
    ///
    /// # Returns
    ///
    /// The task if found
    pub async fn get_task(&self, task_id: Uuid) -> Result<Task> {
        self.task_queue
            .get_task(task_id)
            .await
            .context("Failed to get task")
    }

    /// Get child tasks spawned by a parent task
    ///
    /// # Arguments
    ///
    /// * `parent_task_id` - UUID of the parent task
    ///
    /// # Returns
    ///
    /// Vector of child tasks
    pub async fn get_child_tasks(&self, parent_task_id: Uuid) -> Result<Vec<Task>> {
        self.task_queue
            .get_children_by_parent(parent_task_id)
            .await
            .context("Failed to get child tasks")
    }

    /// Get tasks by status
    ///
    /// # Arguments
    ///
    /// * `status` - Task status to filter by
    ///
    /// # Returns
    ///
    /// Vector of tasks with the given status
    pub async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        self.task_queue
            .get_tasks_by_status(status)
            .await
            .context("Failed to get tasks by status")
    }

    /// Update workflow state for a task
    ///
    /// Stores information about which children were spawned and whether
    /// workflow expectations were met.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task
    /// * `_workflow_state` - Updated workflow state
    ///
    /// # Returns
    ///
    /// Ok if update succeeded
    ///
    /// # Note
    ///
    /// Currently a placeholder - requires repository access for full task update
    pub async fn update_workflow_state(
        &self,
        task_id: Uuid,
        _workflow_state: crate::domain::models::WorkflowState,
    ) -> Result<()> {
        // TODO: Implement full task update when repository is available
        info!("Workflow state updated for task {}", task_id);
        Ok(())
    }

    /// Link a validation task to the task being validated
    ///
    /// # Arguments
    ///
    /// * `original_task_id` - UUID of the task being validated
    /// * `_validation_task_id` - UUID of the validation task
    ///
    /// # Returns
    ///
    /// Ok if link succeeded
    ///
    /// # Note
    ///
    /// Sets status to AwaitingValidation
    pub async fn link_validation_task(
        &self,
        original_task_id: Uuid,
        _validation_task_id: Uuid,
    ) -> Result<()> {
        // Update status to AwaitingValidation
        self.task_queue
            .update_task_status(original_task_id, TaskStatus::AwaitingValidation)
            .await
            .context("Failed to update task status to AwaitingValidation")
    }

    /// Submit a new task to the queue
    ///
    /// # Arguments
    ///
    /// * `task` - Task to submit
    ///
    /// # Returns
    ///
    /// UUID of the submitted task
    ///
    /// # Note
    ///
    /// Currently a placeholder  - full implementation requires service access
    pub async fn submit_task(&self, task: Task) -> Result<Uuid> {
        info!("Submitting task: {}", task.id);
        self.task_queue.submit_task(task).await
    }

    /// Process all pending tasks on startup
    ///
    /// This method should be called when the swarm orchestrator starts to transition
    /// all pending tasks to either Ready (if dependencies are met) or Blocked (if not).
    ///
    /// # Returns
    ///
    /// * `Ok(usize)` - Number of tasks processed
    /// * `Err` - If database error or coordination fails
    #[instrument(skip(self))]
    pub async fn process_pending_tasks(&self) -> Result<usize> {
        info!("Processing all pending tasks on startup");

        // Get all pending tasks
        let pending_tasks = self
            .task_queue
            .get_tasks_by_status(TaskStatus::Pending)
            .await
            .context("Failed to get pending tasks")?;

        let task_count = pending_tasks.len();
        info!("Found {} pending tasks to process", task_count);

        // Process each pending task
        for task in pending_tasks {
            if let Err(e) = self.coordinate_task_lifecycle(task.id).await {
                warn!(
                    "Failed to coordinate pending task {}: {:?}",
                    task.id, e
                );
            }
        }

        info!("Finished processing {} pending tasks", task_count);
        Ok(task_count)
    }

    /// Update a task's status directly
    ///
    /// This method bypasses the normal lifecycle coordination and directly updates
    /// the task status. Use with caution - prefer lifecycle methods like
    /// `mark_task_running`, `handle_task_completion`, etc. when possible.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to update
    /// * `status` - New status to set
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Status updated successfully
    /// * `Err` - If task not found or database error
    pub async fn update_task_status(&self, task_id: Uuid, status: TaskStatus) -> Result<()> {
        self.task_queue
            .update_task_status(task_id, status)
            .await
            .context("Failed to update task status")
    }

    /// Update a task's calculated priority directly
    ///
    /// This method bypasses the normal priority calculation and directly updates
    /// the task priority. Use with caution - prefer `coordinate_task_lifecycle`
    /// which recalculates priority based on the priority calculator.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to update
    /// * `priority` - New calculated priority value
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Priority updated successfully
    /// * `Err` - If task not found or database error
    pub async fn update_task_priority(&self, task_id: Uuid, priority: f64) -> Result<()> {
        self.task_queue
            .update_task_priority(task_id, priority)
            .await
            .context("Failed to update task priority")
    }

    /// Update a task with all its fields
    ///
    /// Used for updating task state like `chain_handoff_state` during recovery.
    ///
    /// # Arguments
    ///
    /// * `task` - The task with updated fields
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task updated successfully
    /// * `Err` - If task not found or database error
    pub async fn update_task(&self, task: &Task) -> Result<()> {
        self.task_queue
            .update_task(task)
            .await
            .context("Failed to update task")
    }

    /// Update a task with automatic retry on optimistic lock conflict
    ///
    /// This method handles the common pattern of:
    /// 1. Apply modifications to a task
    /// 2. If update fails due to version conflict, re-read and retry
    ///
    /// This is essential for reliable updates when multiple processes may
    /// be modifying the same task concurrently.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to update
    /// * `update_fn` - Closure that modifies the task. Will be called again on retry.
    /// * `max_retries` - Maximum number of retry attempts (default: 3)
    ///
    /// # Returns
    ///
    /// * `Ok(Task)` - The updated task with fresh version
    /// * `Err` - If all retries exhausted or non-retryable error
    ///
    /// # Example
    ///
    /// ```ignore
    /// coordinator.update_task_with_retry(task_id, |task| {
    ///     task.chain_handoff_state = None;
    /// }, 3).await?;
    /// ```
    pub async fn update_task_with_retry<F>(
        &self,
        task_id: Uuid,
        update_fn: F,
        max_retries: u32,
    ) -> Result<Task>
    where
        F: Fn(&mut Task),
    {
        let mut last_error = None;

        for attempt in 0..=max_retries {
            // Get fresh task from database
            let mut task = self
                .task_queue
                .get_task(task_id)
                .await
                .context("Failed to get task for retry update")?;

            // Apply the update function
            update_fn(&mut task);

            // Attempt to save
            match self.task_queue.update_task(&task).await {
                Ok(()) => {
                    if attempt > 0 {
                        debug!(
                            task_id = %task_id,
                            attempts = attempt + 1,
                            "Task update succeeded after optimistic lock retry"
                        );
                    }
                    // Return task with updated version
                    // Note: The version in memory is still old, re-read if caller needs fresh version
                    return self.task_queue.get_task(task_id).await
                        .context("Failed to get task after successful update");
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // Check if this is an optimistic lock conflict
                    if error_str.contains("OptimisticLockConflict") || error_str.contains("version") {
                        if attempt < max_retries {
                            debug!(
                                task_id = %task_id,
                                attempt = attempt + 1,
                                max_retries = max_retries,
                                "Optimistic lock conflict, retrying with fresh task"
                            );
                            // Small backoff to reduce contention
                            tokio::time::sleep(tokio::time::Duration::from_millis(10 * (attempt as u64 + 1))).await;
                            last_error = Some(e);
                            continue;
                        }
                    }
                    // Non-retryable error or max retries exceeded
                    return Err(e).context("Failed to update task");
                }
            }
        }

        Err(last_error
            .map(|e| e.context(format!(
                "Task update failed after {} retries due to optimistic lock conflicts",
                max_retries
            )))
            .unwrap_or_else(|| anyhow::anyhow!("Task update failed with unknown error")))
    }

    /// Get tasks that have been running longer than the threshold
    pub async fn get_stale_running_tasks(&self, threshold_secs: u64) -> Result<Vec<Task>> {
        self.task_queue
            .get_stale_running_tasks(threshold_secs)
            .await
            .context("Failed to get stale running tasks")
    }

    /// Recover a stale task by marking it as failed
    ///
    /// This is called when a task has been in Running status too long,
    /// indicating the worker may have crashed.
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the stale task
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task recovered (marked as failed for retry)
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn recover_stale_task(&self, task_id: Uuid) -> Result<()> {
        warn!(task_id = %task_id, "Recovering stale task - worker may have crashed");

        // Mark as failed with a recoverable error message
        // The existing retry logic will handle re-queuing if retries remain
        self.handle_task_failure(
            task_id,
            "Task stalled - worker may have crashed or timed out. Recovered by stale task monitor.".to_string()
        ).await
    }

    /// Recover blocked tasks whose dependencies are actually completed
    ///
    /// This handles the case where dependency resolution failed (e.g., due to
    /// transient database errors) and tasks are stuck in Blocked state even
    /// though their dependencies have completed.
    ///
    /// # Returns
    ///
    /// * `Ok(usize)` - Number of tasks recovered
    /// * `Err` - If database operations fail
    #[instrument(skip(self))]
    pub async fn recover_stuck_blocked_tasks(&self) -> Result<usize> {
        // Get all blocked tasks
        let blocked_tasks = self
            .task_queue
            .get_tasks_by_status(TaskStatus::Blocked)
            .await
            .context("Failed to get blocked tasks")?;

        if blocked_tasks.is_empty() {
            return Ok(0);
        }

        debug!("Checking {} blocked tasks for stuck state", blocked_tasks.len());

        let mut recovered_count = 0;

        for task in blocked_tasks {
            // Check if all dependencies are actually completed
            match self.check_dependencies_met(&task).await {
                Ok(true) => {
                    // Dependencies are met but task is still Blocked - this is stuck
                    warn!(
                        task_id = %task.id,
                        summary = %task.summary,
                        "Found stuck blocked task with completed dependencies, recovering"
                    );

                    // Re-coordinate the task lifecycle
                    if let Err(e) = self.coordinate_task_lifecycle(task.id).await {
                        error!(
                            task_id = %task.id,
                            error = ?e,
                            "Failed to recover stuck blocked task"
                        );
                    } else {
                        info!(task_id = %task.id, "Successfully recovered stuck blocked task");
                        recovered_count += 1;
                    }
                }
                Ok(false) => {
                    // Dependencies not met - this is correctly blocked
                    debug!(task_id = %task.id, "Blocked task has unmet dependencies (correct state)");
                }
                Err(e) => {
                    // Error checking dependencies - log but continue
                    warn!(
                        task_id = %task.id,
                        error = ?e,
                        "Error checking dependencies for blocked task"
                    );
                }
            }
        }

        if recovered_count > 0 {
            info!("Recovered {} stuck blocked task(s)", recovered_count);
        }

        Ok(recovered_count)
    }

    // Private helper methods

    /// Check if all dependencies for a task are met
    ///
    /// A task's dependencies are met if:
    /// - The task has no dependencies (dependencies field is None or empty)
    /// - All tasks in the dependencies list have status "completed"
    ///
    /// # Arguments
    ///
    /// * `task` - The task to check dependencies for
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - All dependencies are met
    /// * `Ok(false)` - Some dependencies are not yet completed
    /// * `Err` - If dependency task not found or database error
    async fn check_dependencies_met(&self, task: &Task) -> Result<bool> {
        // No dependencies means dependencies are met
        let Some(ref deps) = task.dependencies else {
            return Ok(true);
        };

        if deps.is_empty() {
            return Ok(true);
        }

        // Check each dependency
        for &dep_id in deps {
            let dep_task = self
                .task_queue
                .get_task(dep_id)
                .await
                .context(format!("Failed to get dependency task {dep_id}"))?;

            if dep_task.status != TaskStatus::Completed {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check if a completed task is a child task, and if so, check if all siblings are complete.
    /// If all children of a parent task are complete, transition the parent from AwaitingChildren
    /// to Ready so it can continue its chain workflow.
    ///
    /// This implements the fan-out/fan-in pattern for decomposition tasks.
    #[instrument(skip(self), fields(task_id = %completed_task_id))]
    pub async fn check_parent_child_completion(&self, completed_task_id: Uuid) -> Result<()> {
        // 1. Get the completed task to check if it has a parent
        let completed_task = self.task_queue.get_task(completed_task_id).await?;

        // 2. Check if this task was spawned by a parent (decomposition child)
        let parent_task_id = match completed_task.spawned_by_task_id {
            Some(parent_id) => parent_id,
            None => {
                debug!(
                    task_id = %completed_task_id,
                    "Task has no spawned_by_task_id, not a decomposition child"
                );
                return Ok(());
            }
        };

        info!(
            task_id = %completed_task_id,
            parent_task_id = %parent_task_id,
            "Checking if parent task's children are all complete"
        );

        // 3. Get the parent task
        let parent_task = self.task_queue.get_task(parent_task_id).await?;

        // 4. Check if parent is in AwaitingChildren status
        if parent_task.status != TaskStatus::AwaitingChildren {
            debug!(
                parent_task_id = %parent_task_id,
                parent_status = ?parent_task.status,
                "Parent task is not in AwaitingChildren status, skipping"
            );
            return Ok(());
        }

        // 5. Get the list of children the parent is waiting for
        let awaiting_children = match &parent_task.awaiting_children {
            Some(children) if !children.is_empty() => children.clone(),
            _ => {
                warn!(
                    parent_task_id = %parent_task_id,
                    "Parent in AwaitingChildren status but has no awaiting_children list"
                );
                return Ok(());
            }
        };

        // 6. Check if all children are in terminal state
        let mut all_complete = true;
        let mut any_failed = false;

        for child_id in &awaiting_children {
            match self.task_queue.get_task(*child_id).await {
                Ok(child) => {
                    match child.status {
                        TaskStatus::Completed => {
                            // Child completed successfully
                        }
                        TaskStatus::Failed | TaskStatus::Cancelled => {
                            any_failed = true;
                            // Still consider as terminal
                        }
                        _ => {
                            // Child still in progress
                            all_complete = false;
                            debug!(
                                child_id = %child_id,
                                child_status = ?child.status,
                                "Child task not yet complete"
                            );
                            break; // Early exit, not all complete
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        child_id = %child_id,
                        error = ?e,
                        "Failed to get child task, assuming not complete"
                    );
                    all_complete = false;
                    break;
                }
            }
        }

        if !all_complete {
            info!(
                parent_task_id = %parent_task_id,
                "Not all children complete yet, parent remains in AwaitingChildren"
            );
            return Ok(());
        }

        // 7. All children are in terminal state - transition parent
        if any_failed {
            // If any child failed, we need to decide: fail the parent or let it handle?
            // For now, we'll still transition to Ready and let the chain decide
            warn!(
                parent_task_id = %parent_task_id,
                "All children complete but some failed, transitioning parent to Ready anyway"
            );
        }

        // 8. Prepare parent task for chain continuation
        // We need to update:
        // - chain_step_index to the next step from chain_handoff_state
        // - clear awaiting_children
        // - set status to Ready
        let mut updated_parent = parent_task.clone();

        // Update chain_step_index from handoff state
        if let Some(ref handoff_state) = parent_task.chain_handoff_state {
            let next_step = handoff_state.pending_next_step_index;
            info!(
                parent_task_id = %parent_task_id,
                current_step = parent_task.chain_step_index,
                next_step = next_step,
                "Advancing parent chain step index from handoff state"
            );
            updated_parent.chain_step_index = next_step;
        }

        // Clear awaiting_children - they're all done
        updated_parent.awaiting_children = None;

        // Set status to Ready
        updated_parent.status = TaskStatus::Ready;

        // Update timestamp
        updated_parent.last_updated_at = chrono::Utc::now();

        info!(
            parent_task_id = %parent_task_id,
            child_count = awaiting_children.len(),
            new_step_index = updated_parent.chain_step_index,
            "All children complete! Transitioning parent from AwaitingChildren to Ready"
        );

        // Save all updates atomically
        self.task_queue
            .update_task(&updated_parent)
            .await
            .context("Failed to update parent task for chain continuation")?;

        // Notify status change
        let _ = self
            .status_tx
            .send(TaskStatusUpdate {
                task_id: parent_task_id,
                old_status: TaskStatus::AwaitingChildren,
                new_status: TaskStatus::Ready,
            })
            .await;

        info!(
            parent_task_id = %parent_task_id,
            "Parent task transitioned to Ready, will continue chain workflow at step {}",
            updated_parent.chain_step_index
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::task::{DependencyType, TaskSource};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    // Mock TaskQueueService for testing
    struct MockTaskQueue {
        tasks: Arc<StdMutex<HashMap<Uuid, Task>>>,
    }

    impl MockTaskQueue {
        fn new() -> Self {
            Self {
                tasks: Arc::new(StdMutex::new(HashMap::new())),
            }
        }

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

        async fn get_dependent_tasks(&self, task_id: Uuid) -> Result<Vec<Task>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|t| {
                    t.dependencies
                        .as_ref()
                        .is_some_and(|deps| deps.contains(&task_id))
                })
                .cloned()
                .collect())
        }

        async fn get_children_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>> {
            let tasks = self.tasks.lock().unwrap();
            Ok(tasks
                .values()
                .filter(|t| t.parent_task_id == Some(parent_id))
                .cloned()
                .collect())
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

        async fn update_parent_and_insert_children_atomic(
            &self,
            parent_task: &Task,
            child_tasks: Vec<Task>,
        ) -> Result<crate::domain::ports::task_repository::DecompositionResult> {
            use crate::domain::ports::task_repository::DecompositionResult;
            let mut tasks = self.tasks.lock().unwrap();

            // Update parent
            tasks.insert(parent_task.id, parent_task.clone());

            // Insert children
            let mut children_inserted = Vec::new();
            for child in child_tasks {
                children_inserted.push(child.id);
                tasks.insert(child.id, child);
            }

            Ok(DecompositionResult {
                parent_id: parent_task.id,
                parent_new_version: parent_task.version + 1,
                children_inserted,
                children_already_existed: vec![],
            })
        }
    }

    // Mock PriorityCalculator for testing
    struct MockPriorityCalculator;

    #[async_trait]
    impl PriorityCalculator for MockPriorityCalculator {
        async fn calculate_priority(&self, task: &Task) -> Result<f64> {
            // Simple mock: return base priority
            Ok(f64::from(task.priority))
        }

        async fn recalculate_priorities(&self, tasks: &[Task]) -> Result<Vec<(Uuid, f64)>> {
            Ok(tasks
                .iter()
                .map(|t| (t.id, f64::from(t.priority)))
                .collect())
        }
    }

    fn create_test_task(id: &str, status: TaskStatus, dependencies: Option<Vec<&str>>) -> Task {
        let task_id = Uuid::parse_str(id).unwrap();
        let deps = dependencies.map(|d| d.iter().map(|&s| Uuid::parse_str(s).unwrap()).collect());

        Task {
            id: task_id,
            summary: format!("Task {id}"),
            description: "Test task".to_string(),
            agent_type: "test".to_string(),
            priority: 5,
            calculated_priority: 5.0,
            status,
            dependencies: deps,
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
            validation_requirement: crate::domain::models::task::ValidationRequirement::None,
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
            awaiting_children: None,
            spawned_by_task_id: None,
        }
    }

    #[tokio::test]
    async fn test_coordinate_task_lifecycle_no_dependencies() {
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Pending,
            None,
        );

        task_queue.add_task(task);

        // Coordinate task lifecycle
        coordinator
            .coordinate_task_lifecycle(task_id)
            .await
            .unwrap();

        // Task should be ready (no dependencies)
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_coordinate_task_lifecycle_blocked_by_dependencies() {
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Task 1 is pending, Task 2 depends on Task 1
        let task1 = create_test_task(id1, TaskStatus::Pending, None);
        let task2 = create_test_task(id2, TaskStatus::Pending, Some(vec![id1]));

        task_queue.add_task(task1);
        task_queue.add_task(task2);

        // Coordinate task 2 lifecycle
        let task2_id = Uuid::parse_str(id2).unwrap();
        coordinator
            .coordinate_task_lifecycle(task2_id)
            .await
            .unwrap();

        // Task 2 should be blocked (dependency not completed)
        let updated_task = task_queue.get_task(task2_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Blocked);
    }

    #[tokio::test]
    async fn test_get_next_ready_task() {
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Two ready tasks with different priorities
        let mut task1 = create_test_task(id1, TaskStatus::Ready, None);
        task1.calculated_priority = 5.0;

        let mut task2 = create_test_task(id2, TaskStatus::Ready, None);
        task2.calculated_priority = 10.0;

        task_queue.add_task(task1);
        task_queue.add_task(task2);

        // Get next ready task (should be task2 with higher priority)
        let next_task = coordinator.get_next_ready_task().await.unwrap();
        assert!(next_task.is_some());
        assert_eq!(next_task.unwrap().id, Uuid::parse_str(id2).unwrap());
    }

    #[tokio::test]
    async fn test_handle_task_completion_triggers_dependents() {
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let id1 = "00000000-0000-0000-0000-000000000001";
        let id2 = "00000000-0000-0000-0000-000000000002";

        // Task 1 is running, Task 2 is blocked waiting for Task 1
        let task1 = create_test_task(id1, TaskStatus::Running, None);
        let task2 = create_test_task(id2, TaskStatus::Blocked, Some(vec![id1]));

        task_queue.add_task(task1);
        task_queue.add_task(task2);

        // Complete task 1
        let task1_id = Uuid::parse_str(id1).unwrap();
        coordinator.handle_task_completion(task1_id).await.unwrap();

        // Task 1 should be completed
        let task1_updated = task_queue.get_task(task1_id).await.unwrap();
        assert_eq!(task1_updated.status, TaskStatus::Completed);

        // Task 2 should now be ready (dependency met)
        let task2_id = Uuid::parse_str(id2).unwrap();
        let task2_updated = task_queue.get_task(task2_id).await.unwrap();
        assert_eq!(task2_updated.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_handle_task_failure_with_retry() {
        // Test that tasks with retries remaining get re-queued
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Running,
            None,
        );
        // Task has max_retries: 3 and retry_count: 0, so retries are available

        task_queue.add_task(task);

        // Mark task as failed
        coordinator
            .handle_task_failure(task_id, "Test error".to_string())
            .await
            .unwrap();

        // Task should be re-queued (Ready) since retries are available
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Ready);
        // Error message is cleared when retrying (task gets a fresh start)
        assert!(updated_task.error_message.is_none());
        // Retry count should be incremented
        assert_eq!(updated_task.retry_count, 1);
    }

    #[tokio::test]
    async fn test_handle_task_failure_max_retries_exceeded() {
        // Test that tasks with no retries remaining stay failed
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let mut task = create_test_task(
            "00000000-0000-0000-0000-000000000002",
            TaskStatus::Running,
            None,
        );
        // Exhaust retries
        task.retry_count = 3;
        task.max_retries = 3;

        task_queue.add_task(task);

        // Mark task as failed
        coordinator
            .handle_task_failure(task_id, "Final error".to_string())
            .await
            .unwrap();

        // Task should stay failed since max retries exceeded
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Failed);
        assert_eq!(updated_task.error_message, Some("Final error".to_string()));
    }

    #[tokio::test]
    async fn test_hooks_integration_pre_ready() {
        use crate::domain::models::{HookAction, HookEvent, HooksConfig, TaskHook};

        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator = Arc::new(TaskCoordinator::new(
            task_queue.clone(),
            dependency_resolver,
            priority_calc,
        ));

        // Create a hook configuration with a log message on PreReady
        let hook = TaskHook {
            id: "test-pre-ready".to_string(),
            description: Some("Test PreReady hook".to_string()),
            event: HookEvent::PreReady,
            conditions: vec![],
            actions: vec![HookAction::LogMessage {
                level: "info".to_string(),
                message: "PreReady hook executed for ${task_id}".to_string(),
            }],
            priority: 10,
            enabled: true,
        };

        let config = HooksConfig { hooks: vec![hook] };

        // Initialize hooks
        let hook_executor = Arc::new(HookExecutor::new(Some(coordinator.clone()), None));
        let mut hook_registry = HookRegistry::new(hook_executor);
        hook_registry.load_from_config(config).unwrap();

        // Set hook registry on coordinator
        coordinator
            .set_hook_registry(Arc::new(hook_registry))
            .await;

        // Create a pending task
        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Pending,
            None,
        );

        task_queue.add_task(task);

        // Coordinate task lifecycle - should trigger PreReady hook
        coordinator
            .coordinate_task_lifecycle(task_id)
            .await
            .unwrap();

        // Task should be ready (hook didn't block)
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_hooks_integration_post_complete() {
        use crate::domain::models::{HookAction, HookEvent, HooksConfig, TaskHook};

        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator = Arc::new(TaskCoordinator::new(
            task_queue.clone(),
            dependency_resolver,
            priority_calc,
        ));

        // Create a hook configuration with a log message on PostComplete
        let hook = TaskHook {
            id: "test-post-complete".to_string(),
            description: Some("Test PostComplete hook".to_string()),
            event: HookEvent::PostComplete,
            conditions: vec![],
            actions: vec![HookAction::LogMessage {
                level: "info".to_string(),
                message: "Task ${task_id} completed successfully".to_string(),
            }],
            priority: 10,
            enabled: true,
        };

        let config = HooksConfig { hooks: vec![hook] };

        // Initialize hooks
        let hook_executor = Arc::new(HookExecutor::new(Some(coordinator.clone()), None));
        let mut hook_registry = HookRegistry::new(hook_executor);
        hook_registry.load_from_config(config).unwrap();

        // Set hook registry on coordinator
        coordinator
            .set_hook_registry(Arc::new(hook_registry))
            .await;

        // Create a running task
        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Running,
            None,
        );

        task_queue.add_task(task);

        // Handle task completion - should trigger PostComplete hook
        coordinator.handle_task_completion(task_id).await.unwrap();

        // Task should be completed
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn test_hooks_integration_blocking_hook() {
        use crate::domain::models::{HookAction, HookEvent, HooksConfig, TaskHook};

        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator = Arc::new(TaskCoordinator::new(
            task_queue.clone(),
            dependency_resolver,
            priority_calc,
        ));

        // Create a hook that blocks transition
        let hook = TaskHook {
            id: "test-blocking".to_string(),
            description: Some("Test blocking hook".to_string()),
            event: HookEvent::PreStart,
            conditions: vec![],
            actions: vec![HookAction::BlockTransition {
                reason: "Task blocked by test hook".to_string(),
            }],
            priority: 10,
            enabled: true,
        };

        let config = HooksConfig { hooks: vec![hook] };

        // Initialize hooks
        let hook_executor = Arc::new(HookExecutor::new(Some(coordinator.clone()), None));
        let mut hook_registry = HookRegistry::new(hook_executor);
        hook_registry.load_from_config(config).unwrap();

        // Set hook registry on coordinator
        coordinator
            .set_hook_registry(Arc::new(hook_registry))
            .await;

        // Create a ready task
        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Ready,
            None,
        );

        task_queue.add_task(task);

        // Try to mark task as running - should be blocked by hook
        coordinator.mark_task_running(task_id).await.unwrap();

        // Task should still be ready (blocked from starting)
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_mark_task_running_without_worktree_service() {
        // Test that mark_task_running works without a worktree service configured
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let mut task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Ready,
            None,
        );
        task.feature_branch = Some("feature/test".to_string());

        task_queue.add_task(task);

        // Mark task as running - should work without worktree service
        coordinator.mark_task_running(task_id).await.unwrap();

        // Task should be running
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Running);
        // Worktree fields should still be None (no worktree service configured)
        assert!(updated_task.branch.is_none());
        assert!(updated_task.worktree_path.is_none());
    }

    #[tokio::test]
    async fn test_mark_task_running_without_feature_branch() {
        // Test that tasks without feature_branch are marked running without worktree setup
        let task_queue = Arc::new(MockTaskQueue::new());
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(MockPriorityCalculator);

        let coordinator =
            TaskCoordinator::new(task_queue.clone(), dependency_resolver, priority_calc);

        // Create worktree service and set it
        let worktree_service = Arc::new(crate::services::WorktreeService::new(task_queue.clone()));
        coordinator.set_worktree_service(worktree_service).await;

        let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let task = create_test_task(
            "00000000-0000-0000-0000-000000000001",
            TaskStatus::Ready,
            None,
        );
        // No feature_branch set

        task_queue.add_task(task);

        // Mark task as running
        coordinator.mark_task_running(task_id).await.unwrap();

        // Task should be running
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Running);
        // Worktree fields should still be None (no feature_branch)
        assert!(updated_task.branch.is_none());
        assert!(updated_task.worktree_path.is_none());
    }
}

use crate::domain::models::task::{Task, TaskStatus};
use crate::domain::ports::task_repository::{BatchInsertResult, DecompositionResult, IdempotentInsertResult};
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

/// Port for task queue operations following hexagonal architecture
///
/// Defines the interface for task storage and retrieval operations.
/// Implementations can use `SQLite`, `PostgreSQL`, or in-memory storage.
///
/// # Examples
///
/// ```no_run
/// use abathur::domain::ports::TaskQueueService;
/// use abathur::domain::models::task::TaskStatus;
/// use anyhow::Result;
/// use uuid::Uuid;
///
/// async fn example(queue: &dyn TaskQueueService, task_id: Uuid) -> Result<()> {
///     let task = queue.get_task(task_id).await?;
///     queue.update_task_status(task_id, TaskStatus::Running).await?;
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait TaskQueueService: Send + Sync {
    /// Get a task by ID
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(Task)` - The task if found
    /// * `Err` - If task not found or database error
    async fn get_task(&self, task_id: Uuid) -> Result<Task>;

    /// Get all tasks with a specific status
    ///
    /// # Arguments
    ///
    /// * `status` - Filter tasks by this status
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Task>)` - List of tasks with the given status
    /// * `Err` - If database error
    async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>>;

    /// Get all tasks that depend on a specific task
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to find dependents for
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Task>)` - List of tasks that depend on the given task
    /// * `Err` - If database error
    async fn get_dependent_tasks(&self, task_id: Uuid) -> Result<Vec<Task>>;

    /// Get all child tasks spawned by a parent task
    ///
    /// Returns tasks where parent_task_id matches the given task ID.
    /// This is used for contract validation to verify that agents spawned
    /// the required child tasks.
    ///
    /// # Arguments
    ///
    /// * `parent_id` - UUID of the parent task
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Task>)` - List of child tasks spawned by the parent
    /// * `Err` - If database error
    async fn get_children_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>>;

    /// Update the status of a task
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
    async fn update_task_status(&self, task_id: Uuid, status: TaskStatus) -> Result<()>;

    /// Update a task's calculated priority
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
    async fn update_task_priority(&self, task_id: Uuid, priority: f64) -> Result<()>;

    /// Update a task's fields
    ///
    /// Updates the entire task record. Useful for updating multiple fields
    /// at once, such as worktree_path, branch, and feature_branch.
    ///
    /// # Arguments
    ///
    /// * `task` - The task with updated fields
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task updated successfully
    /// * `Err` - If task not found or database error
    async fn update_task(&self, task: &Task) -> Result<()>;

    /// Mark a task as failed with an error message
    ///
    /// # Arguments
    ///
    /// * `task_id` - UUID of the task to mark as failed
    /// * `error_message` - Description of the failure
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Task marked as failed successfully
    /// * `Err` - If task not found or database error
    async fn mark_task_failed(&self, task_id: Uuid, error_message: String) -> Result<()>;

    /// Get the next ready task with highest priority
    ///
    /// Returns the task with status "ready" that has the highest calculated priority.
    /// NOTE: This does NOT mark the task as claimed. Use `claim_next_ready_task` for
    /// atomic claim to prevent race conditions.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(Task))` - The highest priority ready task
    /// * `Ok(None)` - No ready tasks available
    /// * `Err` - If database error
    async fn get_next_ready_task(&self) -> Result<Option<Task>>;

    /// Atomically claim the next ready task with highest priority
    ///
    /// This performs an atomic SELECT + UPDATE operation to:
    /// 1. Find the highest-priority task with status=Ready
    /// 2. Immediately mark it as Running
    /// 3. Return the claimed task
    ///
    /// This prevents race conditions where multiple workers pick up the same task.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(Task))` - The claimed task (already marked as Running)
    /// * `Ok(None)` - No ready tasks available
    /// * `Err` - If database error
    async fn claim_next_ready_task(&self) -> Result<Option<Task>>;

    /// Submit a new task to the queue
    ///
    /// # Arguments
    ///
    /// * `task` - The task to submit
    ///
    /// # Returns
    ///
    /// * `Ok(Uuid)` - The task ID
    /// * `Err` - If submission fails
    async fn submit_task(&self, task: Task) -> Result<Uuid>;

    /// Get tasks that have been in Running status longer than the threshold
    ///
    /// Used to detect stale tasks that may have been abandoned due to worker crashes.
    ///
    /// # Arguments
    ///
    /// * `stale_threshold_secs` - Tasks running longer than this are considered stale
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Task>)` - List of stale running tasks
    /// * `Err` - If database error
    async fn get_stale_running_tasks(&self, stale_threshold_secs: u64) -> Result<Vec<Task>>;

    /// Check if a task with the given idempotency key already exists
    ///
    /// Used to prevent duplicate task creation when chain steps retry.
    /// The idempotency key is typically derived from parent_task_id + step_output_hash.
    ///
    /// # Arguments
    ///
    /// * `idempotency_key` - The unique idempotency key to check
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - A task with this idempotency key exists
    /// * `Ok(false)` - No task with this idempotency key exists
    /// * `Err` - If database error
    async fn task_exists_by_idempotency_key(&self, idempotency_key: &str) -> Result<bool>;

    /// Get a task by its idempotency key
    ///
    /// Used to retrieve an existing task when a duplicate insert is detected.
    /// This is more efficient than scanning dependent tasks to find duplicates.
    ///
    /// # Arguments
    ///
    /// * `idempotency_key` - The unique idempotency key to look up
    ///
    /// # Returns
    ///
    /// * `Ok(Some(Task))` - The task with this idempotency key
    /// * `Ok(None)` - No task with this idempotency key exists
    /// * `Err` - If database error
    async fn get_task_by_idempotency_key(&self, idempotency_key: &str) -> Result<Option<Task>>;

    /// Atomically submit a task if no task with the same idempotency key exists
    ///
    /// This method performs an atomic insert operation that prevents race conditions
    /// where multiple concurrent executions might both check for existence and then
    /// both attempt to insert. This is critical for prompt chain task spawning.
    ///
    /// # Arguments
    ///
    /// * `task` - The task to submit
    ///
    /// # Returns
    ///
    /// * `Ok(IdempotentInsertResult::Inserted(uuid))` - Task was submitted successfully
    /// * `Ok(IdempotentInsertResult::AlreadyExists)` - Task with same idempotency key exists
    /// * `Err` - If database error (not including unique violations)
    async fn submit_task_idempotent(&self, task: Task) -> Result<IdempotentInsertResult>;

    /// Atomically submit multiple tasks in a single transaction
    ///
    /// This method performs a transactional batch insert that:
    /// 1. Opens a database transaction
    /// 2. Attempts to insert all tasks (using idempotent insert for each)
    /// 3. If any non-duplicate insert fails, rolls back the entire transaction
    /// 4. Returns results indicating which tasks were inserted vs already existed
    ///
    /// This is critical for chain step task spawning where all spawned tasks
    /// must be inserted atomically to prevent partial state on crash/retry.
    ///
    /// # Arguments
    ///
    /// * `tasks` - The tasks to submit
    ///
    /// # Returns
    ///
    /// * `Ok(BatchInsertResult)` - All tasks processed successfully (inserted or already existed)
    /// * `Err` - Transaction failed and was rolled back
    async fn submit_tasks_transactional(&self, tasks: Vec<Task>) -> Result<BatchInsertResult>;

    /// Resolve dependencies for tasks that depend on a specific completed task
    ///
    /// This is a targeted resolution that only checks tasks that explicitly depend
    /// on the completed task, rather than scanning all tasks. This is O(k) where k
    /// is the number of dependent tasks, vs O(n) for full resolution.
    ///
    /// Should be called when a task transitions to Completed status.
    ///
    /// # Arguments
    ///
    /// * `completed_task_id` - The ID of the task that just completed
    ///
    /// # Returns
    ///
    /// * `Ok(usize)` - Number of tasks that were updated to Ready status
    /// * `Err` - If database error
    async fn resolve_dependencies_for_completed_task(&self, completed_task_id: Uuid) -> Result<usize>;

    /// Atomically update a parent task and insert child tasks in a single transaction
    ///
    /// This is critical for decomposition workflows where:
    /// - Parent must be updated to AwaitingChildren status
    /// - Child tasks must be spawned
    /// - Both must happen atomically to prevent orphaned children on parent update failure
    ///
    /// # Arguments
    ///
    /// * `parent_task` - The parent task with updated fields (status, awaiting_children, etc.)
    /// * `child_tasks` - The child tasks to insert
    ///
    /// # Returns
    ///
    /// * `Ok(DecompositionResult)` - Transaction succeeded
    /// * `Err` - Transaction failed (either optimistic lock conflict or other error)
    ///
    /// # Atomicity Guarantee
    /// If the parent update fails (e.g., version conflict), no children are inserted.
    /// If any child insert fails, the parent update is rolled back.
    async fn update_parent_and_insert_children_atomic(
        &self,
        parent_task: &Task,
        child_tasks: Vec<Task>,
    ) -> Result<DecompositionResult>;
}

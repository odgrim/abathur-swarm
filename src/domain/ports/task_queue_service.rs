use crate::domain::models::task::{Task, TaskStatus};
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

/// Port for task queue operations following hexagonal architecture
///
/// Defines the interface for task storage and retrieval operations.
/// Implementations can use SQLite, PostgreSQL, or in-memory storage.
///
/// # Examples
///
/// ```
/// use abathur::domain::ports::TaskQueueService;
/// use uuid::Uuid;
///
/// async fn example(queue: &dyn TaskQueueService) -> Result<()> {
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
    ///
    /// # Returns
    ///
    /// * `Ok(Some(Task))` - The highest priority ready task
    /// * `Ok(None)` - No ready tasks available
    /// * `Err` - If database error
    async fn get_next_ready_task(&self) -> Result<Option<Task>>;
}

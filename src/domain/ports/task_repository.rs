//! Task repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Task, TaskPriority, TaskStatus};

/// Filter criteria for listing tasks.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub goal_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub agent_type: Option<String>,
}

/// Repository interface for Task persistence.
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Create a new task.
    async fn create(&self, task: &Task) -> DomainResult<()>;

    /// Get a task by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<Task>>;

    /// Update an existing task.
    async fn update(&self, task: &Task) -> DomainResult<()>;

    /// Delete a task by ID.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// List tasks with optional filters.
    async fn list(&self, filter: TaskFilter) -> DomainResult<Vec<Task>>;

    /// Get tasks by goal.
    async fn list_by_goal(&self, goal_id: Uuid) -> DomainResult<Vec<Task>>;

    /// Get tasks by status.
    async fn list_by_status(&self, status: TaskStatus) -> DomainResult<Vec<Task>>;

    /// Get subtasks of a parent task.
    async fn get_subtasks(&self, parent_id: Uuid) -> DomainResult<Vec<Task>>;

    /// Get ready tasks (dependencies met, ordered by priority).
    async fn get_ready_tasks(&self, limit: usize) -> DomainResult<Vec<Task>>;

    /// Get tasks assigned to a specific agent type.
    async fn get_by_agent(&self, agent_type: &str) -> DomainResult<Vec<Task>>;

    /// Get all dependencies of a task.
    async fn get_dependencies(&self, task_id: Uuid) -> DomainResult<Vec<Task>>;

    /// Get all tasks that depend on a given task.
    async fn get_dependents(&self, task_id: Uuid) -> DomainResult<Vec<Task>>;

    /// Add a dependency between tasks.
    async fn add_dependency(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<()>;

    /// Remove a dependency.
    async fn remove_dependency(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<()>;

    /// Count descendants of a task.
    async fn count_descendants(&self, task_id: Uuid) -> DomainResult<u64>;

    /// Get task by idempotency key.
    async fn get_by_idempotency_key(&self, key: &str) -> DomainResult<Option<Task>>;

    /// Count tasks by status.
    async fn count_by_status(&self) -> DomainResult<std::collections::HashMap<TaskStatus, u64>>;
}

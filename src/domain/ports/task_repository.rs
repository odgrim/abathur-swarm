use crate::domain::models::{Task, TaskSource, TaskStatus};
use crate::domain::ports::errors::DatabaseError;
use async_trait::async_trait;
use uuid::Uuid;

/// Filters for querying tasks
#[derive(Default, Debug, Clone)]
pub struct TaskFilters {
    pub status: Option<TaskStatus>,
    pub agent_type: Option<String>,
    pub feature_branch: Option<String>,
    pub session_id: Option<Uuid>,
    pub source: Option<TaskSource>,
    pub exclude_status: Option<TaskStatus>,
    pub limit: Option<i64>,
}

/// Repository port for task persistence operations
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Insert a new task
    async fn insert(&self, task: &Task) -> Result<(), DatabaseError>;

    /// Get a task by ID
    async fn get(&self, id: Uuid) -> Result<Option<Task>, DatabaseError>;

    /// Update an existing task
    async fn update(&self, task: &Task) -> Result<(), DatabaseError>;

    /// Delete a task by ID
    async fn delete(&self, id: Uuid) -> Result<(), DatabaseError>;

    /// List tasks with optional filters
    async fn list(&self, filters: TaskFilters) -> Result<Vec<Task>, DatabaseError>;

    /// Count tasks matching filters
    async fn count(&self, filters: TaskFilters) -> Result<i64, DatabaseError>;

    /// Get ready tasks ordered by calculated priority
    async fn get_ready_tasks(&self, limit: usize) -> Result<Vec<Task>, DatabaseError>;

    /// Update task status and last_updated_at timestamp
    async fn update_status(&self, id: Uuid, status: TaskStatus) -> Result<(), DatabaseError>;

    /// Get tasks by feature branch
    async fn get_by_feature_branch(&self, feature_branch: &str) -> Result<Vec<Task>, DatabaseError>;

    /// Get tasks by parent task ID
    async fn get_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>, DatabaseError>;
}

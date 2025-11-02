use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::models::{Task, TaskSource, TaskStatus};
use crate::infrastructure::database::DatabaseError;

/// Repository interface for task persistence operations following hexagonal architecture.
///
/// This trait defines the contract for task data access, enabling dependency injection
/// and testability. It abstracts the underlying storage mechanism (`SQLite`, `PostgreSQL`, etc.)
/// from the domain and application layers.
///
/// # Design Rationale
/// - **Hexagonal Architecture**: This port defines WHAT operations are needed for task
///   persistence without specifying HOW they're implemented. Adapters in the infrastructure
///   layer provide concrete implementations.
/// - **Testability**: The trait enables easy mocking for unit tests by allowing test
///   implementations that use in-memory storage.
/// - **Technology Independence**: Domain logic depends on this trait, not on specific
///   database libraries like sqlx, enabling future migration to different storage backends.
///
/// # Thread Safety
/// Implementations must be Send + Sync to support concurrent access in async contexts.
/// Most implementations will use connection pooling to handle concurrent requests safely.
///
/// # Usage Example
/// ```rust,ignore
/// async fn process_ready_tasks(repo: Arc<dyn TaskRepository>) -> Result<(), DatabaseError> {
///     let tasks = repo.get_ready_tasks(10).await?;
///     for task in tasks {
///         // Process task...
///     }
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait TaskRepository: Send + Sync {
    /// Insert a new task into the repository.
    ///
    /// # Arguments
    /// * `task` - The task to insert
    ///
    /// # Returns
    /// * `Ok(())` on successful insertion
    /// * `Err(DatabaseError)` on failure (e.g., duplicate ID, constraint violation)
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ValidationError`: Task validation failed (e.g., invalid priority)
    async fn insert(&self, task: &Task) -> Result<(), DatabaseError>;

    /// Retrieve a task by its unique identifier.
    ///
    /// # Arguments
    /// * `id` - The task's UUID
    ///
    /// # Returns
    /// * `Ok(Some(task))` if the task exists
    /// * `Ok(None)` if no task with the given ID exists
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn get(&self, id: Uuid) -> Result<Option<Task>, DatabaseError>;

    /// Update an existing task with new values.
    ///
    /// This replaces all fields of the task with the provided values.
    /// The task must exist in the repository.
    ///
    /// # Arguments
    /// * `task` - The task with updated fields
    ///
    /// # Returns
    /// * `Ok(())` on successful update
    /// * `Err(DatabaseError)` on failure
    ///
    /// # Errors
    /// - `DatabaseError::NotFound`: Task with the given ID doesn't exist
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ValidationError`: Updated task validation failed
    async fn update(&self, task: &Task) -> Result<(), DatabaseError>;

    /// Delete a task by its unique identifier.
    ///
    /// This permanently removes the task from the repository.
    ///
    /// # Arguments
    /// * `id` - The UUID of the task to delete
    ///
    /// # Returns
    /// * `Ok(())` on successful deletion
    /// * `Err(DatabaseError)` on failure
    ///
    /// # Errors
    /// - `DatabaseError::NotFound`: Task with the given ID doesn't exist
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    async fn delete(&self, id: Uuid) -> Result<(), DatabaseError>;

    /// List tasks matching the specified filters.
    ///
    /// Returns tasks that match all specified filter criteria. Empty filters
    /// will match all tasks (subject to pagination limits).
    ///
    /// # Arguments
    /// * `filters` - Filter criteria (status, `agent_type`, priority, pagination)
    ///
    /// # Returns
    /// * `Ok(Vec<Task>)` - List of matching tasks (may be empty)
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn list(&self, filters: &TaskFilters) -> Result<Vec<Task>, DatabaseError>;

    /// Count tasks matching the specified filters.
    ///
    /// Returns the total count of tasks matching the filter criteria,
    /// useful for pagination UI.
    ///
    /// # Arguments
    /// * `filters` - Filter criteria (status, `agent_type`, priority)
    ///
    /// # Returns
    /// * `Ok(count)` - Number of matching tasks
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    async fn count(&self, filters: &TaskFilters) -> Result<i64, DatabaseError>;

    /// Get tasks that are ready for execution.
    ///
    /// Returns tasks with status=Ready, ordered by calculated priority (highest first).
    /// This is the primary method used by task executors to fetch work.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of tasks to return
    ///
    /// # Returns
    /// * `Ok(Vec<Task>)` - List of ready tasks ordered by priority (may be empty)
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn get_ready_tasks(&self, limit: usize) -> Result<Vec<Task>, DatabaseError>;

    /// Get tasks by their feature branch.
    ///
    /// Returns all tasks associated with a specific feature branch,
    /// useful for tracking feature progress and cleanup.
    ///
    /// # Arguments
    /// * `feature_branch` - The feature branch name
    ///
    /// # Returns
    /// * `Ok(Vec<Task>)` - List of tasks for the feature branch (may be empty)
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn get_by_feature_branch(&self, feature_branch: &str)
    -> Result<Vec<Task>, DatabaseError>;

    /// Get tasks that have a specific task as a dependency.
    ///
    /// Returns tasks that depend on the given task ID, useful for
    /// cascade operations and dependency graph analysis.
    ///
    /// # Arguments
    /// * `dependency_id` - The UUID of the dependency task
    ///
    /// # Returns
    /// * `Ok(Vec<Task>)` - List of dependent tasks (may be empty)
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn get_dependents(&self, dependency_id: Uuid) -> Result<Vec<Task>, DatabaseError>;

    /// Get all tasks in a session.
    ///
    /// Returns all tasks associated with a specific session ID,
    /// useful for session cleanup and tracking.
    ///
    /// # Arguments
    /// * `session_id` - The session UUID
    ///
    /// # Returns
    /// * `Ok(Vec<Task>)` - List of tasks in the session (may be empty)
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn get_by_session(&self, session_id: Uuid) -> Result<Vec<Task>, DatabaseError>;

    /// Update a task's status efficiently.
    ///
    /// This updates only the status field and the last_updated_at timestamp,
    /// without requiring a full task update.
    ///
    /// # Arguments
    /// * `id` - The UUID of the task
    /// * `status` - The new status
    ///
    /// # Returns
    /// * `Ok(())` on successful update
    /// * `Err(DatabaseError)` on failure
    ///
    /// # Errors
    /// - `DatabaseError::NotFound`: Task with the given ID doesn't exist
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    async fn update_status(&self, id: Uuid, status: TaskStatus) -> Result<(), DatabaseError>;

    /// Get tasks by parent task ID.
    ///
    /// Returns all tasks that have the specified task as their parent,
    /// useful for hierarchical task management and subtask tracking.
    ///
    /// # Arguments
    /// * `parent_id` - The UUID of the parent task
    ///
    /// # Returns
    /// * `Ok(Vec<Task>)` - List of child tasks (may be empty)
    /// * `Err(DatabaseError)` on query failure
    ///
    /// # Errors
    /// - `DatabaseError::QueryFailed`: Database query execution failed
    /// - `DatabaseError::ParseError`: Failed to deserialize task data
    async fn get_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>, DatabaseError>;
}

/// Filter criteria for task queries.
///
/// All fields are optional. When a field is None, it is not used as a filter criterion.
/// Multiple filters are combined with AND logic.
///
/// # Examples
/// ```rust,ignore
/// // Get all pending tasks
/// let filters = TaskFilters {
///     status: Some(TaskStatus::Pending),
///     ..Default::default()
/// };
///
/// // Get high-priority tasks for a specific agent
/// let filters = TaskFilters {
///     agent_type: Some("rust-specialist".to_string()),
///     priority_min: Some(7),
///     limit: Some(10),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct TaskFilters {
    /// Filter by task status
    pub status: Option<TaskStatus>,

    /// Filter by agent type (exact match)
    pub agent_type: Option<String>,

    /// Filter by task source
    pub source: Option<TaskSource>,

    /// Filter by minimum priority (inclusive)
    pub priority_min: Option<u8>,

    /// Filter by maximum priority (inclusive)
    pub priority_max: Option<u8>,

    /// Filter by feature branch name
    pub feature_branch: Option<String>,

    /// Filter by task branch name
    pub task_branch: Option<String>,

    /// Filter by session ID
    pub session_id: Option<uuid::Uuid>,

    /// Exclude tasks with this status
    pub exclude_status: Option<TaskStatus>,

    /// Maximum number of results to return
    pub limit: Option<usize>,

    /// Number of results to skip (for pagination)
    pub offset: Option<usize>,
}

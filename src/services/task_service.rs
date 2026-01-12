//! Task service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Task, TaskContext, TaskPriority, TaskStatus};
use crate::domain::ports::{GoalRepository, TaskFilter, TaskRepository};

/// Configuration for spawn limits.
#[derive(Debug, Clone)]
pub struct SpawnLimitConfig {
    /// Maximum depth of subtask nesting.
    pub max_subtask_depth: u32,
    /// Maximum number of direct subtasks per task.
    pub max_subtasks_per_task: u32,
    /// Maximum total descendants from a root task.
    pub max_total_descendants: u32,
    /// Whether to allow extension requests when limits are reached.
    pub allow_limit_extensions: bool,
}

impl Default for SpawnLimitConfig {
    fn default() -> Self {
        Self {
            max_subtask_depth: 5,
            max_subtasks_per_task: 10,
            max_total_descendants: 100,
            allow_limit_extensions: true,
        }
    }
}

/// Result of spawn limit checking.
#[derive(Debug, Clone)]
pub enum SpawnLimitResult {
    /// Task creation is allowed.
    Allowed,
    /// Limit exceeded but extension may be granted.
    LimitExceeded {
        limit_type: SpawnLimitType,
        current_value: u32,
        limit_value: u32,
        can_request_extension: bool,
    },
    /// Hard limit - cannot create task.
    HardLimit {
        limit_type: SpawnLimitType,
        reason: String,
    },
}

impl SpawnLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn requires_specialist(&self) -> bool {
        matches!(self, Self::LimitExceeded { can_request_extension: true, .. })
    }
}

/// Type of spawn limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnLimitType {
    SubtaskDepth,
    SubtasksPerTask,
    TotalDescendants,
}

impl SpawnLimitType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SubtaskDepth => "subtask_depth",
            Self::SubtasksPerTask => "subtasks_per_task",
            Self::TotalDescendants => "total_descendants",
        }
    }
}

pub struct TaskService<T: TaskRepository, G: GoalRepository> {
    task_repo: Arc<T>,
    goal_repo: Arc<G>,
    spawn_limits: SpawnLimitConfig,
}

impl<T: TaskRepository, G: GoalRepository> TaskService<T, G> {
    pub fn new(task_repo: Arc<T>, goal_repo: Arc<G>) -> Self {
        Self {
            task_repo,
            goal_repo,
            spawn_limits: SpawnLimitConfig::default(),
        }
    }

    /// Create with custom spawn limits.
    pub fn with_spawn_limits(mut self, limits: SpawnLimitConfig) -> Self {
        self.spawn_limits = limits;
        self
    }

    /// Check spawn limits for creating a subtask under a parent.
    ///
    /// Returns `SpawnLimitResult` indicating whether the task can be created,
    /// and if not, whether a limit evaluation specialist should be triggered.
    pub async fn check_spawn_limits(&self, parent_id: Option<Uuid>) -> DomainResult<SpawnLimitResult> {
        let Some(parent_id) = parent_id else {
            // No parent = root task, no spawn limits apply
            return Ok(SpawnLimitResult::Allowed);
        };

        let parent = self.task_repo.get(parent_id).await?
            .ok_or(DomainError::TaskNotFound(parent_id))?;

        // Check subtask depth
        let depth = self.calculate_depth(&parent).await?;
        if depth >= self.spawn_limits.max_subtask_depth {
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::SubtaskDepth,
                current_value: depth,
                limit_value: self.spawn_limits.max_subtask_depth,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        // Check direct subtasks count
        let direct_subtasks = self.count_direct_subtasks(parent_id).await?;
        if direct_subtasks >= self.spawn_limits.max_subtasks_per_task {
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::SubtasksPerTask,
                current_value: direct_subtasks,
                limit_value: self.spawn_limits.max_subtasks_per_task,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        // Check total descendants from root
        let root_id = self.find_root_task(&parent).await?;
        let total_descendants = self.count_all_descendants(root_id).await?;
        if total_descendants >= self.spawn_limits.max_total_descendants {
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::TotalDescendants,
                current_value: total_descendants,
                limit_value: self.spawn_limits.max_total_descendants,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        Ok(SpawnLimitResult::Allowed)
    }

    /// Calculate the depth of a task in the hierarchy (0 = root).
    async fn calculate_depth(&self, task: &Task) -> DomainResult<u32> {
        let mut depth = 0;
        let mut current = task.clone();

        while let Some(parent_id) = current.parent_id {
            depth += 1;
            if depth > 100 {
                // Safety limit to prevent infinite loops
                break;
            }
            match self.task_repo.get(parent_id).await? {
                Some(parent) => current = parent,
                None => break,
            }
        }

        Ok(depth)
    }

    /// Count direct subtasks of a task.
    async fn count_direct_subtasks(&self, parent_id: Uuid) -> DomainResult<u32> {
        let filter = TaskFilter {
            parent_id: Some(parent_id),
            ..Default::default()
        };
        let subtasks = self.task_repo.list(filter).await?;
        Ok(subtasks.len() as u32)
    }

    /// Find the root task (task with no parent).
    async fn find_root_task(&self, task: &Task) -> DomainResult<Uuid> {
        let mut current = task.clone();

        while let Some(parent_id) = current.parent_id {
            match self.task_repo.get(parent_id).await? {
                Some(parent) => current = parent,
                None => break,
            }
        }

        Ok(current.id)
    }

    /// Count all descendants of a task using iterative BFS.
    async fn count_all_descendants(&self, task_id: Uuid) -> DomainResult<u32> {
        let mut count = 0u32;
        let mut queue = vec![task_id];

        while let Some(current_id) = queue.pop() {
            let filter = TaskFilter {
                parent_id: Some(current_id),
                ..Default::default()
            };
            let children = self.task_repo.list(filter).await?;

            count += children.len() as u32;
            for child in children {
                queue.push(child.id);
            }

            // Safety limit
            if count > 10000 {
                break;
            }
        }

        Ok(count)
    }

    /// Submit a new task.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_task(
        &self,
        title: String,
        description: String,
        goal_id: Option<Uuid>,
        parent_id: Option<Uuid>,
        priority: TaskPriority,
        agent_type: Option<String>,
        depends_on: Vec<Uuid>,
        context: Option<TaskContext>,
        idempotency_key: Option<String>,
    ) -> DomainResult<Task> {
        // Check for duplicate by idempotency key
        if let Some(ref key) = idempotency_key {
            if let Some(existing) = self.task_repo.get_by_idempotency_key(key).await? {
                return Ok(existing);
            }
        }

        // Validate goal exists if specified
        if let Some(gid) = goal_id {
            let goal = self.goal_repo.get(gid).await?;
            if goal.is_none() {
                return Err(DomainError::GoalNotFound(gid));
            }
        }

        // Validate parent exists if specified
        if let Some(pid) = parent_id {
            let parent = self.task_repo.get(pid).await?;
            if parent.is_none() {
                return Err(DomainError::TaskNotFound(pid));
            }
        }

        // Validate dependencies exist
        for dep_id in &depends_on {
            let dep = self.task_repo.get(*dep_id).await?;
            if dep.is_none() {
                return Err(DomainError::TaskNotFound(*dep_id));
            }
        }

        let mut task = Task::new(title, description)
            .with_priority(priority);

        if let Some(gid) = goal_id {
            task = task.with_goal(gid);
        }
        if let Some(pid) = parent_id {
            task = task.with_parent(pid);
        }
        if let Some(agent) = agent_type {
            task = task.with_agent(agent);
        }
        if let Some(key) = idempotency_key {
            task = task.with_idempotency_key(key);
        }

        for dep in depends_on {
            task = task.with_dependency(dep);
        }

        if let Some(ctx) = context {
            task.context = ctx;
        }

        task.validate().map_err(DomainError::ValidationFailed)?;
        self.task_repo.create(&task).await?;

        // Check if task is ready
        self.check_and_update_readiness(&mut task).await?;
        self.task_repo.update(&task).await?;

        Ok(task)
    }

    /// Get a task by ID.
    pub async fn get_task(&self, id: Uuid) -> DomainResult<Option<Task>> {
        self.task_repo.get(id).await
    }

    /// List tasks with optional filters.
    pub async fn list_tasks(&self, filter: TaskFilter) -> DomainResult<Vec<Task>> {
        self.task_repo.list(filter).await
    }

    /// Get ready tasks ordered by priority.
    pub async fn get_ready_tasks(&self, limit: usize) -> DomainResult<Vec<Task>> {
        self.task_repo.get_ready_tasks(limit).await
    }

    /// Get tasks for a specific goal.
    pub async fn get_tasks_for_goal(&self, goal_id: Uuid) -> DomainResult<Vec<Task>> {
        self.task_repo.list_by_goal(goal_id).await
    }

    /// Transition task to Running state (claim it).
    pub async fn claim_task(&self, task_id: Uuid, agent_type: &str) -> DomainResult<Task> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.status != TaskStatus::Ready {
            return Err(DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "running".to_string(),
            });
        }

        task.agent_type = Some(agent_type.to_string());
        task.transition_to(TaskStatus::Running).map_err(|_| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "running".to_string(),
        })?;

        self.task_repo.update(&task).await?;
        Ok(task)
    }

    /// Mark task as complete.
    pub async fn complete_task(&self, task_id: Uuid) -> DomainResult<Task> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Complete).map_err(|_| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "complete".to_string(),
        })?;

        self.task_repo.update(&task).await?;

        // Update dependent tasks that might now be ready
        self.update_dependents_readiness(task_id).await?;

        Ok(task)
    }

    /// Mark task as failed.
    pub async fn fail_task(&self, task_id: Uuid, error_message: Option<String>) -> DomainResult<Task> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Failed).map_err(|_| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "failed".to_string(),
        })?;

        if let Some(msg) = error_message {
            task.context.hints.push(format!("Error: {}", msg));
        }

        self.task_repo.update(&task).await?;

        // Mark dependent tasks as blocked
        self.mark_dependents_blocked(task_id).await?;

        Ok(task)
    }

    /// Retry a failed task.
    pub async fn retry_task(&self, task_id: Uuid) -> DomainResult<Task> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if !task.can_retry() {
            return Err(DomainError::ValidationFailed(
                "Task cannot be retried: either not failed or max retries exceeded".to_string()
            ));
        }

        task.retry().map_err(DomainError::ValidationFailed)?;
        self.task_repo.update(&task).await?;

        // Unblock dependents
        self.update_dependents_readiness(task_id).await?;

        Ok(task)
    }

    /// Cancel a task.
    pub async fn cancel_task(&self, task_id: Uuid) -> DomainResult<Task> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.is_terminal() {
            return Err(DomainError::ValidationFailed(
                "Cannot cancel a terminal task".to_string()
            ));
        }

        task.transition_to(TaskStatus::Canceled).map_err(|_| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "canceled".to_string(),
        })?;

        self.task_repo.update(&task).await?;

        // Mark dependent tasks as blocked
        self.mark_dependents_blocked(task_id).await?;

        Ok(task)
    }

    /// Get task status counts.
    pub async fn get_status_counts(&self) -> DomainResult<std::collections::HashMap<TaskStatus, u64>> {
        self.task_repo.count_by_status().await
    }

    /// Check if a task's dependencies are all complete.
    async fn are_dependencies_complete(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(true);
        }

        let deps = self.task_repo.get_dependencies(task.id).await?;
        Ok(deps.iter().all(|d| d.status == TaskStatus::Complete))
    }

    /// Check if any dependency has failed.
    async fn has_failed_dependency(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(false);
        }

        let deps = self.task_repo.get_dependencies(task.id).await?;
        Ok(deps.iter().any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled))
    }

    /// Check and update task readiness.
    async fn check_and_update_readiness(&self, task: &mut Task) -> DomainResult<()> {
        if task.status != TaskStatus::Pending {
            return Ok(());
        }

        if self.has_failed_dependency(task).await? {
            task.transition_to(TaskStatus::Blocked).ok();
        } else if self.are_dependencies_complete(task).await? {
            task.transition_to(TaskStatus::Ready).ok();
        }

        Ok(())
    }

    /// Update readiness of tasks that depend on a given task.
    async fn update_dependents_readiness(&self, task_id: Uuid) -> DomainResult<()> {
        let dependents = self.task_repo.get_dependents(task_id).await?;

        for mut dep in dependents {
            if dep.status == TaskStatus::Pending || dep.status == TaskStatus::Blocked {
                self.check_and_update_readiness(&mut dep).await?;
                self.task_repo.update(&dep).await?;
            }
        }

        Ok(())
    }

    /// Mark dependent tasks as blocked.
    async fn mark_dependents_blocked(&self, task_id: Uuid) -> DomainResult<()> {
        let dependents = self.task_repo.get_dependents(task_id).await?;

        for mut dep in dependents {
            if dep.status == TaskStatus::Pending || dep.status == TaskStatus::Ready {
                dep.transition_to(TaskStatus::Blocked).ok();
                self.task_repo.update(&dep).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_test_pool, SqliteGoalRepository, SqliteTaskRepository, Migrator, all_embedded_migrations
    };

    async fn setup_service() -> TaskService<SqliteTaskRepository, SqliteGoalRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let task_repo = Arc::new(SqliteTaskRepository::new(pool.clone()));
        let goal_repo = Arc::new(SqliteGoalRepository::new(pool));
        TaskService::new(task_repo, goal_repo)
    }

    #[tokio::test]
    async fn test_submit_task() {
        let service = setup_service().await;

        let task = service.submit_task(
            "Test Task".to_string(),
            "Description".to_string(),
            None,
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
        ).await.unwrap();

        assert_eq!(task.title, "Test Task");
        assert_eq!(task.status, TaskStatus::Ready); // No deps, should be ready
    }

    #[tokio::test]
    async fn test_task_dependencies_block_ready() {
        let service = setup_service().await;

        // Create a dependency task
        let dep = service.submit_task(
            "Dependency".to_string(),
            "Must complete first".to_string(),
            None,
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
        ).await.unwrap();

        // Create main task that depends on it
        let main = service.submit_task(
            "Main Task".to_string(),
            "Depends on first".to_string(),
            None,
            None,
            TaskPriority::Normal,
            None,
            vec![dep.id],
            None,
            None,
        ).await.unwrap();

        // Main should be pending (dependency not complete)
        assert_eq!(main.status, TaskStatus::Pending);

        // Complete the dependency
        service.claim_task(dep.id, "test-agent").await.unwrap();
        service.complete_task(dep.id).await.unwrap();

        // Check main task - should now be ready
        let main_updated = service.get_task(main.id).await.unwrap().unwrap();
        assert_eq!(main_updated.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_idempotency() {
        let service = setup_service().await;

        let task1 = service.submit_task(
            "Task".to_string(),
            "Description".to_string(),
            None,
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
        ).await.unwrap();

        let task2 = service.submit_task(
            "Different Task".to_string(),
            "Different Description".to_string(),
            None,
            None,
            TaskPriority::High,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
        ).await.unwrap();

        // Should return same task
        assert_eq!(task1.id, task2.id);
        assert_eq!(task2.title, "Task"); // Original title
    }

    #[tokio::test]
    async fn test_claim_and_complete() {
        let service = setup_service().await;

        let task = service.submit_task(
            "Test".to_string(),
            "Desc".to_string(),
            None,
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
        ).await.unwrap();

        let claimed = service.claim_task(task.id, "test-agent").await.unwrap();
        assert_eq!(claimed.status, TaskStatus::Running);
        assert_eq!(claimed.agent_type, Some("test-agent".to_string()));

        let completed = service.complete_task(task.id).await.unwrap();
        assert_eq!(completed.status, TaskStatus::Complete);
        assert!(completed.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_fail_and_retry() {
        let service = setup_service().await;

        let task = service.submit_task(
            "Test".to_string(),
            "Desc".to_string(),
            None,
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
        ).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        let failed = service.fail_task(task.id, Some("Test error".to_string())).await.unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);

        let retried = service.retry_task(task.id).await.unwrap();
        assert_eq!(retried.status, TaskStatus::Ready);
        assert_eq!(retried.retry_count, 1);
    }
}

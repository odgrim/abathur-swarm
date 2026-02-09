//! Task service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use async_trait::async_trait;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Task, TaskContext, TaskPriority, TaskSource, TaskStatus};
use crate::domain::ports::{TaskFilter, TaskRepository};
use crate::services::command_bus::{CommandError, CommandOutcome, CommandResult, TaskCommand, TaskCommandHandler};
use crate::services::event_bus::{
    EventCategory, EventPayload, EventSeverity, UnifiedEvent,
};
use crate::services::event_factory;

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

#[derive(Clone)]
pub struct TaskService<T: TaskRepository> {
    task_repo: Arc<T>,
    spawn_limits: SpawnLimitConfig,
}

impl<T: TaskRepository> TaskService<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self {
            task_repo,
            spawn_limits: SpawnLimitConfig::default(),
        }
    }

    /// Create with custom spawn limits.
    pub fn with_spawn_limits(mut self, limits: SpawnLimitConfig) -> Self {
        self.spawn_limits = limits;
        self
    }

    /// Helper to build a UnifiedEvent with standard fields.
    fn make_event(
        severity: EventSeverity,
        category: EventCategory,
        goal_id: Option<uuid::Uuid>,
        task_id: Option<uuid::Uuid>,
        payload: EventPayload,
    ) -> UnifiedEvent {
        event_factory::make_event(severity, category, goal_id, task_id, payload)
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

    /// Submit a new task. Returns the task and events to be journaled.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_task(
        &self,
        title: Option<String>,
        description: String,
        parent_id: Option<Uuid>,
        priority: TaskPriority,
        agent_type: Option<String>,
        depends_on: Vec<Uuid>,
        context: Option<TaskContext>,
        idempotency_key: Option<String>,
        source: TaskSource,
        deadline: Option<chrono::DateTime<chrono::Utc>>,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut events = Vec::new();

        // Check for duplicate by idempotency key
        if let Some(ref key) = idempotency_key {
            if let Some(existing) = self.task_repo.get_by_idempotency_key(key).await? {
                return Ok((existing, events));
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

        let mut task = match title {
            Some(t) => Task::with_title(t, description),
            None => Task::new(description),
        };
        task = task.with_priority(priority)
            .with_source(source);

        if let Some(pid) = parent_id {
            task = task.with_parent(pid);
        }
        if let Some(agent) = agent_type {
            task = task.with_agent(agent);
        }
        if let Some(key) = idempotency_key {
            task = task.with_idempotency_key(key);
        }
        task.deadline = deadline;

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

        // Collect TaskSubmitted event
        let goal_id = task.parent_id.unwrap_or_else(Uuid::new_v4);
        events.push(Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            Some(goal_id),
            Some(task.id),
            EventPayload::TaskSubmitted {
                task_id: task.id,
                task_title: task.title.clone(),
                goal_id,
            },
        ));

        // If the task is immediately ready (no deps), collect TaskReady event
        if task.status == TaskStatus::Ready {
            events.push(Self::make_event(
                EventSeverity::Debug,
                EventCategory::Task,
                Some(goal_id),
                Some(task.id),
                EventPayload::TaskReady {
                    task_id: task.id,
                    task_title: task.title.clone(),
                },
            ));
        }

        Ok((task, events))
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

    /// Transition task to Running state (claim it).
    pub async fn claim_task(&self, task_id: Uuid, agent_type: &str) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
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

        let events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskClaimed {
                task_id,
                agent_type: agent_type.to_string(),
            },
        )];

        Ok((task, events))
    }

    /// Mark task as complete.
    pub async fn complete_task(&self, task_id: Uuid) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Complete).map_err(|_| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "complete".to_string(),
        })?;

        self.task_repo.update(&task).await?;

        let events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskCompleted {
                task_id,
                tokens_used: 0,
            },
        )];

        Ok((task, events))
    }

    /// Mark task as failed.
    pub async fn fail_task(&self, task_id: Uuid, error_message: Option<String>) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Failed).map_err(|_| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "failed".to_string(),
        })?;

        let error_str = error_message.clone().unwrap_or_default();
        if let Some(msg) = error_message {
            task.context.hints.push(format!("Error: {}", msg));
        }

        self.task_repo.update(&task).await?;

        let events = vec![Self::make_event(
            EventSeverity::Error,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskFailed {
                task_id,
                error: error_str,
                retry_count: task.retry_count,
            },
        )];

        Ok((task, events))
    }

    /// Retry a failed task.
    pub async fn retry_task(&self, task_id: Uuid) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if !task.can_retry() {
            return Err(DomainError::ValidationFailed(
                "Task cannot be retried: either not failed or max retries exceeded".to_string()
            ));
        }

        task.retry().map_err(DomainError::ValidationFailed)?;
        self.task_repo.update(&task).await?;

        let events = vec![Self::make_event(
            EventSeverity::Warning,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskRetrying {
                task_id,
                attempt: task.retry_count,
                max_attempts: task.max_retries,
            },
        )];

        Ok((task, events))
    }

    /// Cancel a task.
    pub async fn cancel_task(&self, task_id: Uuid, reason: &str) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
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

        let events = vec![Self::make_event(
            EventSeverity::Warning,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskCanceled {
                task_id,
                reason: reason.to_string(),
            },
        )];

        Ok((task, events))
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

}

#[async_trait]
impl<T: TaskRepository + 'static> TaskCommandHandler for TaskService<T> {
    async fn handle(&self, cmd: TaskCommand) -> Result<CommandOutcome, CommandError> {
        match cmd {
            TaskCommand::Submit {
                title,
                description,
                parent_id,
                priority,
                agent_type,
                depends_on,
                context,
                idempotency_key,
                source,
                deadline,
            } => {
                let (task, events) = self
                    .submit_task(
                        title,
                        description,
                        parent_id,
                        priority,
                        agent_type,
                        depends_on,
                        *context,
                        idempotency_key,
                        source,
                        deadline,
                    )
                    .await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Claim {
                task_id,
                agent_type,
            } => {
                let (task, events) = self.claim_task(task_id, &agent_type).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Complete { task_id, .. } => {
                let (task, events) = self.complete_task(task_id).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Fail { task_id, error } => {
                let (task, events) = self.fail_task(task_id, error).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Retry { task_id } => {
                let (task, events) = self.retry_task(task_id).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Cancel { task_id, reason } => {
                let (task, events) = self.cancel_task(task_id, &reason).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Transition {
                task_id,
                new_status,
            } => {
                // Direct transition for reconciliation — load, transition, save.
                let mut task = self
                    .task_repo
                    .get(task_id)
                    .await?
                    .ok_or(DomainError::TaskNotFound(task_id))?;
                task.transition_to(new_status).map_err(|_| {
                    DomainError::InvalidStateTransition {
                        from: task.status.as_str().to_string(),
                        to: new_status.as_str().to_string(),
                    }
                })?;
                self.task_repo.update(&task).await?;

                // Collect event for the transition so handlers can react
                let mut events = Vec::new();
                let payload = match new_status {
                    TaskStatus::Ready => Some(EventPayload::TaskReady {
                        task_id,
                        task_title: task.title.clone(),
                    }),
                    TaskStatus::Complete => Some(EventPayload::TaskCompleted {
                        task_id,
                        tokens_used: 0,
                    }),
                    TaskStatus::Failed => Some(EventPayload::TaskFailed {
                        task_id,
                        error: "reconciliation-transition".into(),
                        retry_count: task.retry_count,
                    }),
                    TaskStatus::Canceled => Some(EventPayload::TaskCanceled {
                        task_id,
                        reason: "reconciliation-transition".into(),
                    }),
                    _ => None,
                };
                if let Some(payload) = payload {
                    events.push(Self::make_event(
                        EventSeverity::Info,
                        EventCategory::Task,
                        None,
                        Some(task_id),
                        payload,
                    ));
                }

                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_migrated_test_pool, SqliteTaskRepository};

    async fn setup_service() -> TaskService<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        TaskService::new(task_repo)
    }

    #[tokio::test]
    async fn test_submit_task() {
        let service = setup_service().await;

        let (task, events) = service.submit_task(
            Some("Test Task".to_string()),
            "Description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        ).await.unwrap();

        assert_eq!(task.title, "Test Task");
        assert_eq!(task.status, TaskStatus::Ready); // No deps, should be ready
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn test_task_dependencies_block_ready() {
        let service = setup_service().await;

        // Create a dependency task
        let (dep, _) = service.submit_task(
            Some("Dependency".to_string()),
            "Must complete first".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        ).await.unwrap();

        // Create main task that depends on it
        let (main, _) = service.submit_task(
            Some("Main Task".to_string()),
            "Depends on first".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![dep.id],
            None,
            None,
            TaskSource::Human,
            None,
        ).await.unwrap();

        // Main should be pending (dependency not complete)
        assert_eq!(main.status, TaskStatus::Pending);

        // Complete the dependency
        service.claim_task(dep.id, "test-agent").await.unwrap();
        service.complete_task(dep.id).await.unwrap();

        // TaskService emits a TaskCompleted event; readiness cascading is handled
        // by the TaskCompletedReadinessHandler in the event reactor, not by
        // TaskService directly. In this unit test (no reactor), the dependent
        // task stays Pending. Full cascade is tested in integration tests.
        let main_updated = service.get_task(main.id).await.unwrap().unwrap();
        assert_eq!(main_updated.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn test_idempotency() {
        let service = setup_service().await;

        let (task1, _) = service.submit_task(
            Some("Task".to_string()),
            "Description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
            TaskSource::Human,
            None,
        ).await.unwrap();

        let (task2, _) = service.submit_task(
            Some("Different Task".to_string()),
            "Different Description".to_string(),
            None,
            TaskPriority::High,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
            TaskSource::Human,
            None,
        ).await.unwrap();

        // Should return same task
        assert_eq!(task1.id, task2.id);
        assert_eq!(task2.title, "Task"); // Original title
    }

    #[tokio::test]
    async fn test_claim_and_complete() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        ).await.unwrap();

        let (claimed, _) = service.claim_task(task.id, "test-agent").await.unwrap();
        assert_eq!(claimed.status, TaskStatus::Running);
        assert_eq!(claimed.agent_type, Some("test-agent".to_string()));

        let (completed, _) = service.complete_task(task.id).await.unwrap();
        assert_eq!(completed.status, TaskStatus::Complete);
        assert!(completed.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_fail_and_retry() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
        ).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        let (failed, _) = service.fail_task(task.id, Some("Test error".to_string())).await.unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);

        let (retried, _) = service.retry_task(task.id).await.unwrap();
        assert_eq!(retried.status, TaskStatus::Ready);
        assert_eq!(retried.retry_count, 1);
    }
}

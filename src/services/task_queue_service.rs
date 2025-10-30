use crate::domain::models::{Task, TaskStatus};
use crate::domain::ports::{TaskFilters, TaskRepository};
use crate::services::{DependencyResolver, PriorityCalculator};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, instrument, warn};
use uuid::Uuid;

/// Service for managing task queue operations.
///
/// Coordinates task submission, dependency resolution, and priority
/// calculation using domain models and infrastructure repositories.
///
/// # Examples
///
/// ```no_run
/// use abathur::services::{TaskQueueService, DependencyResolver, PriorityCalculator};
/// use std::sync::Arc;
///
/// # async fn example(repo: Arc<dyn abathur::domain::ports::TaskRepository>) {
/// let service = TaskQueueService::new(
///     repo,
///     DependencyResolver::new(),
///     PriorityCalculator::new(),
/// );
/// # }
/// ```
pub struct TaskQueueService {
    pub(crate) repo: Arc<dyn TaskRepository>,
    dependency_resolver: DependencyResolver,
    priority_calc: PriorityCalculator,
}

impl TaskQueueService {
    /// Create a new TaskQueueService with the given dependencies
    pub fn new(
        repo: Arc<dyn TaskRepository>,
        dependency_resolver: DependencyResolver,
        priority_calc: PriorityCalculator,
    ) -> Self {
        Self {
            repo,
            dependency_resolver,
            priority_calc,
        }
    }

    /// Submit a new task to the queue
    ///
    /// # Steps:
    /// 1. Validate task (summary, priority)
    /// 2. Validate dependencies exist
    /// 3. Check for circular dependencies
    /// 4. Calculate dependency depth
    /// 5. Calculate priority
    /// 6. Insert into repository
    ///
    /// # Arguments
    /// * `task` - The task to submit
    ///
    /// # Returns
    /// The UUID of the submitted task
    ///
    /// # Errors
    /// - Invalid task data (summary too long, invalid priority)
    /// - Missing dependencies
    /// - Circular dependencies
    /// - Database errors
    #[instrument(skip(self, task), fields(task_id = %task.id), err)]
    pub async fn submit(&self, mut task: Task) -> Result<Uuid> {
        // 1. Validate task
        task.validate_summary()
            .context("Task summary validation failed")?;
        task.validate_priority()
            .context("Task priority validation failed")?;

        // 2. Fetch all tasks to validate dependencies
        let all_tasks = self
            .repo
            .list(&TaskFilters::default())
            .await
            .context("Failed to fetch existing tasks")?;

        // 3. Validate dependencies exist
        if task.has_dependencies() {
            self.dependency_resolver
                .validate_dependencies(&task, &all_tasks)
                .context("Dependency validation failed")?;

            // 4. Check for circular dependencies by adding this task to the graph
            let mut tasks_with_new = all_tasks.clone();
            tasks_with_new.push(task.clone());

            if let Some(cycle) = self.dependency_resolver.detect_cycle(&tasks_with_new) {
                warn!("Circular dependency detected: {:?}", cycle);
                return Err(anyhow::anyhow!("Circular dependency detected: {:?}", cycle));
            }

            // 5. Calculate dependency depth
            let depth = self
                .dependency_resolver
                .calculate_depth(&task, &all_tasks)
                .context("Failed to calculate dependency depth")?;

            // 6. Calculate and update priority
            self.priority_calc.update_task_priority(&mut task, depth);

            info!(
                "Task {} submitted with depth {} and priority {}",
                task.id, depth, task.calculated_priority
            );
        } else {
            // No dependencies - depth is 0
            self.priority_calc.update_task_priority(&mut task, 0);
            info!(
                "Task {} submitted with no dependencies and priority {}",
                task.id, task.calculated_priority
            );
        }

        // 7. Insert into repository
        self.repo
            .insert(&task)
            .await
            .context("Failed to insert task into repository")?;

        // 8. Re-resolve dependencies in case this new task completes dependencies for other tasks
        // This handles the case where tasks were submitted out of order
        self.resolve_dependencies().await
            .context("Failed to resolve dependencies after task submission")?;

        Ok(task.id)
    }

    /// List tasks with optional filters
    ///
    /// # Arguments
    /// * `filters` - Optional filters to apply
    ///
    /// # Returns
    /// Vector of tasks matching the filters
    #[instrument(skip(self), err)]
    pub async fn list(&self, filters: TaskFilters) -> Result<Vec<Task>> {
        self.repo
            .list(&filters)
            .await
            .context("Failed to list tasks")
    }

    /// Cancel a task and cascade cancellation to dependent tasks
    ///
    /// # Arguments
    /// * `id` - The UUID of the task to cancel
    ///
    /// # Errors
    /// - Task not found
    /// - Database errors
    #[instrument(skip(self), err)]
    pub async fn cancel(&self, id: Uuid) -> Result<()> {
        // 1. Get the task to cancel
        let mut task = self
            .repo
            .get(id)
            .await
            .context("Failed to fetch task")?
            .ok_or_else(|| anyhow::anyhow!("Task {} not found", id))?;

        if task.status == TaskStatus::Cancelled {
            warn!("Task {} is already cancelled", id);
            return Ok(());
        }

        if task.status == TaskStatus::Completed {
            warn!("Cannot cancel completed task {}", id);
            return Err(anyhow::anyhow!("Cannot cancel completed task"));
        }

        // 2. Cancel this task
        task.status = TaskStatus::Cancelled;
        self.repo
            .update(&task)
            .await
            .context("Failed to update task status")?;

        info!("Task {} cancelled", id);

        // 3. Find and cancel all dependent tasks
        let all_tasks = self
            .repo
            .list(&TaskFilters::default())
            .await
            .context("Failed to fetch all tasks")?;

        let mut to_cancel = Vec::new();
        self.find_dependent_tasks(id, &all_tasks, &mut to_cancel);

        for dep_id in to_cancel {
            // Only cancel if not already completed/cancelled
            if let Ok(Some(mut dep_task)) = self.repo.get(dep_id).await {
                if dep_task.status != TaskStatus::Completed
                    && dep_task.status != TaskStatus::Cancelled
                {
                    dep_task.status = TaskStatus::Cancelled;
                    self.repo
                        .update(&dep_task)
                        .await
                        .context("Failed to cancel dependent task")?;

                    info!("Dependent task {} cancelled", dep_id);
                }
            }
        }

        Ok(())
    }

    /// Recursively find all tasks that depend on the given task
    fn find_dependent_tasks(&self, task_id: Uuid, all_tasks: &[Task], result: &mut Vec<Uuid>) {
        find_dependent_tasks_recursive(task_id, all_tasks, result);
    }

    /// Resolve task dependencies and update statuses
    ///
    /// This function:
    /// 1. Fetches all tasks with status Pending or Blocked
    /// 2. Checks if their dependencies are satisfied
    /// 3. Updates status to Ready if all dependencies are met
    ///
    /// # Returns
    /// Number of tasks that were updated to Ready status
    ///
    /// # Errors
    /// - Database errors
    #[instrument(skip(self), err)]
    pub async fn resolve_dependencies(&self) -> Result<usize> {
        // 1. Fetch all tasks to build dependency graph
        let all_tasks = self
            .repo
            .list(&TaskFilters::default())
            .await
            .context("Failed to fetch all tasks")?;

        let mut updated_count = 0;

        // 2. Find tasks that are Pending or Blocked
        let tasks_to_check: Vec<_> = all_tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::Blocked))
            .collect();

        info!("Checking {} tasks for dependency resolution", tasks_to_check.len());

        // 3. Check each task's dependencies
        for task in tasks_to_check {
            let should_be_ready = if task.has_dependencies() {
                // Check if all dependencies are met
                self.dependency_resolver
                    .check_dependencies_met(task, &all_tasks)
            } else {
                // No dependencies means it should be ready
                true
            };

            if should_be_ready {
                // Update task status to Ready
                let mut updated_task = (*task).clone();
                updated_task.status = TaskStatus::Ready;
                updated_task.last_updated_at = chrono::Utc::now();

                self.repo
                    .update(&updated_task)
                    .await
                    .context(format!("Failed to update task {} to Ready", task.id))?;

                info!("Task {} status updated: {:?} -> Ready", task.id, task.status);
                updated_count += 1;
            } else if task.status == TaskStatus::Pending {
                // Has dependencies but not all met, update to Blocked
                let mut updated_task = (*task).clone();
                updated_task.status = TaskStatus::Blocked;
                updated_task.last_updated_at = chrono::Utc::now();

                self.repo
                    .update(&updated_task)
                    .await
                    .context(format!("Failed to update task {} to Blocked", task.id))?;

                info!("Task {} status updated: Pending -> Blocked", task.id);
            }
        }

        info!("Resolved dependencies: {} tasks updated to Ready", updated_count);
        Ok(updated_count)
    }

    /// Get ready tasks ordered by calculated priority
    ///
    /// Returns tasks with status=Ready, ordered by calculated_priority descending
    ///
    /// # Arguments
    /// * `limit` - Maximum number of tasks to return (default: unlimited)
    ///
    /// # Returns
    /// Vector of ready tasks
    #[instrument(skip(self), err)]
    pub async fn get_ready_tasks(&self, limit: Option<usize>) -> Result<Vec<Task>> {
        let limit = limit.unwrap_or(usize::MAX);

        self.repo
            .get_ready_tasks(limit)
            .await
            .context("Failed to get ready tasks")
    }

    /// Get a task by ID
    ///
    /// # Arguments
    /// * `id` - The UUID of the task
    ///
    /// # Returns
    /// The task if found, None otherwise
    #[instrument(skip(self), err)]
    pub async fn get(&self, id: Uuid) -> Result<Option<Task>> {
        self.repo.get(id).await.context("Failed to get task")
    }

    /// Update a task's status
    ///
    /// # Arguments
    /// * `id` - The UUID of the task
    /// * `status` - The new status
    ///
    /// # Errors
    /// - Task not found
    /// - Database errors
    #[instrument(skip(self), err)]
    pub async fn update_status(&self, id: Uuid, status: TaskStatus) -> Result<()> {
        let mut task = self
            .repo
            .get(id)
            .await
            .context("Failed to fetch task")?
            .ok_or_else(|| anyhow::anyhow!("Task {} not found", id))?;

        task.status = status;
        self.repo
            .update(&task)
            .await
            .context("Failed to update task status")
    }

    /// Count tasks matching filters
    ///
    /// # Arguments
    /// * `filters` - Filters to apply
    ///
    /// # Returns
    /// Number of tasks matching the filters
    #[instrument(skip(self), err)]
    pub async fn count(&self, filters: TaskFilters) -> Result<i64> {
        self.repo
            .count(&filters)
            .await
            .context("Failed to count tasks")
    }
}

// Standalone recursive function for finding dependent tasks
fn find_dependent_tasks_recursive(task_id: Uuid, all_tasks: &[Task], result: &mut Vec<Uuid>) {
    for task in all_tasks {
        if task.get_dependencies().contains(&task_id) && !result.contains(&task.id) {
            result.push(task.id);
            // Recursively find tasks that depend on this one
            find_dependent_tasks_recursive(task.id, all_tasks, result);
        }
    }
}

// Implement the TaskQueueService trait for the TaskQueueService struct
#[async_trait]
impl crate::domain::ports::TaskQueueService for TaskQueueService {
    async fn get_task(&self, task_id: Uuid) -> Result<Task> {
        self.get(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task {} not found", task_id))
    }

    async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        let filters = TaskFilters {
            status: Some(status),
            ..Default::default()
        };
        self.list(filters).await
    }

    async fn get_dependent_tasks(&self, task_id: Uuid) -> Result<Vec<Task>> {
        self.repo
            .get_dependents(task_id)
            .await
            .context("Failed to get dependent tasks")
    }

    async fn get_children_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>> {
        self.repo
            .get_by_parent(parent_id)
            .await
            .context("Failed to get children by parent")
    }

    async fn update_task_status(&self, task_id: Uuid, status: TaskStatus) -> Result<()> {
        self.update_status(task_id, status).await?;

        // If task is now completed, re-resolve dependencies for dependent tasks
        if status == TaskStatus::Completed {
            let dependent_tasks = self.get_dependent_tasks(task_id).await?;
            if !dependent_tasks.is_empty() {
                info!(
                    "Task {} completed, re-resolving dependencies for {} dependent tasks",
                    task_id,
                    dependent_tasks.len()
                );
                self.resolve_dependencies().await?;
            }
        }

        Ok(())
    }

    async fn update_task_priority(&self, task_id: Uuid, priority: f64) -> Result<()> {
        let mut task = self
            .get_task(task_id)
            .await
            .context("Failed to fetch task")?;

        task.calculated_priority = priority;
        self.repo
            .update(&task)
            .await
            .context("Failed to update task priority")
    }

    async fn mark_task_failed(&self, task_id: Uuid, error_message: String) -> Result<()> {
        let mut task = self
            .get_task(task_id)
            .await
            .context("Failed to fetch task")?;

        task.status = TaskStatus::Failed;
        task.error_message = Some(error_message.clone());
        self.repo
            .update(&task)
            .await
            .context("Failed to mark task as failed")?;

        // Re-resolve dependencies to ensure dependent tasks remain blocked
        let dependent_tasks = self.get_dependent_tasks(task_id).await?;
        if !dependent_tasks.is_empty() {
            warn!(
                "Task {} failed with error: '{}'. {} dependent tasks will remain blocked",
                task_id,
                error_message,
                dependent_tasks.len()
            );
            self.resolve_dependencies().await?;
        }

        Ok(())
    }

    async fn get_next_ready_task(&self) -> Result<Option<Task>> {
        let tasks = self.get_ready_tasks(Some(1)).await?;
        Ok(tasks.into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::database::DatabaseError;
    use mockall::mock;
    use mockall::predicate::*;

    // Mock TaskRepository
    mock! {
        pub TaskRepo {}

        #[async_trait::async_trait]
        impl TaskRepository for TaskRepo {
            async fn insert(&self, task: &Task) -> Result<(), DatabaseError>;
            async fn get(&self, id: Uuid) -> Result<Option<Task>, DatabaseError>;
            async fn update(&self, task: &Task) -> Result<(), DatabaseError>;
            async fn delete(&self, id: Uuid) -> Result<(), DatabaseError>;
            async fn list(&self, filters: &TaskFilters) -> Result<Vec<Task>, DatabaseError>;
            async fn count(&self, filters: &TaskFilters) -> Result<i64, DatabaseError>;
            async fn get_ready_tasks(&self, limit: usize) -> Result<Vec<Task>, DatabaseError>;
            async fn get_by_feature_branch(&self, feature_branch: &str) -> Result<Vec<Task>, DatabaseError>;
            async fn get_dependents(&self, dependency_id: Uuid) -> Result<Vec<Task>, DatabaseError>;
            async fn get_by_session(&self, session_id: Uuid) -> Result<Vec<Task>, DatabaseError>;
            async fn update_status(&self, id: Uuid, status: TaskStatus) -> Result<(), DatabaseError>;
            async fn get_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>, DatabaseError>;
        }
    }

    fn create_test_task(summary: &str) -> Task {
        Task::new(summary.to_string(), "Test description".to_string())
    }

    #[tokio::test]
    async fn test_submit_simple_task() {
        let mut mock_repo = MockTaskRepo::new();

        // Expect list to return empty (no existing tasks) - called twice:
        // 1. During validation
        // 2. During resolve_dependencies after insert
        mock_repo.expect_list().times(2).returning(|_| Ok(vec![]));

        // Expect insert to be called once
        mock_repo.expect_insert().times(1).returning(|_| Ok(()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let task = create_test_task("Test task");
        let result = service.submit(task.clone()).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), task.id);
    }

    #[tokio::test]
    async fn test_submit_task_with_dependencies() {
        let mut mock_repo = MockTaskRepo::new();

        let dep_task = create_test_task("Dependency task");
        let mut main_task = create_test_task("Main task");
        main_task.dependencies = Some(vec![dep_task.id]);

        // Expect list to return the dependency task - called twice:
        // 1. During validation
        // 2. During resolve_dependencies after insert
        let dep_task_clone = dep_task.clone();
        mock_repo
            .expect_list()
            .times(2)
            .returning(move |_| Ok(vec![dep_task_clone.clone()]));

        // Expect insert
        mock_repo.expect_insert().times(1).returning(|_| Ok(()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.submit(main_task.clone()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_submit_task_with_missing_dependency() {
        let mut mock_repo = MockTaskRepo::new();

        let mut task = create_test_task("Test task");
        task.dependencies = Some(vec![Uuid::new_v4()]); // Non-existent dependency

        // Expect list to return empty
        mock_repo.expect_list().returning(|_| Ok(vec![]));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.submit(task).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_submit_task_with_circular_dependency() {
        let mut mock_repo = MockTaskRepo::new();

        let task1_id = Uuid::new_v4();
        let task2_id = Uuid::new_v4();

        let mut task1 = create_test_task("Task 1");
        task1.id = task1_id;
        task1.dependencies = Some(vec![task2_id]);

        let mut task2 = create_test_task("Task 2");
        task2.id = task2_id;
        task2.dependencies = Some(vec![task1_id]);

        // task2 already exists, trying to submit task1
        let task2_clone = task2.clone();
        mock_repo
            .expect_list()
            .returning(move |_| Ok(vec![task2_clone.clone()]));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.submit(task1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_tasks() {
        let mut mock_repo = MockTaskRepo::new();

        let task1 = create_test_task("Task 1");
        let task2 = create_test_task("Task 2");

        let tasks = vec![task1.clone(), task2.clone()];
        let tasks_clone = tasks.clone();

        mock_repo
            .expect_list()
            .returning(move |_| Ok(tasks_clone.clone()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.list(TaskFilters::default()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_cancel_task() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task to cancel");
        let task_id = task.id;
        let task_clone = task.clone();

        // Expect get to return the task
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // Expect update to be called when cancelling
        mock_repo
            .expect_update()
            .returning(|_| Ok(()));

        // Expect list to check for dependent tasks
        mock_repo.expect_list().returning(|_| Ok(vec![]));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.cancel(task_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cancel_task_with_dependents() {
        let mut mock_repo = MockTaskRepo::new();

        let task1 = create_test_task("Task 1");
        let task1_id = task1.id;

        let mut task2 = create_test_task("Task 2");
        task2.dependencies = Some(vec![task1_id]);
        let task2_id = task2.id;

        let task1_clone = task1.clone();
        let task2_clone = task2.clone();

        // Expect get for task1
        mock_repo
            .expect_get()
            .with(eq(task1_id))
            .returning(move |_| Ok(Some(task1_clone.clone())));

        // Expect get for task2 when checking if it should be cancelled
        let task2_for_get = task2.clone();
        mock_repo
            .expect_get()
            .with(eq(task2_id))
            .returning(move |_| Ok(Some(task2_for_get.clone())));

        // Expect update for both tasks
        mock_repo
            .expect_update()
            .times(2)
            .returning(|_| Ok(()));

        // Expect list to return both tasks
        let all_tasks = vec![task1.clone(), task2_clone.clone()];
        mock_repo
            .expect_list()
            .returning(move |_| Ok(all_tasks.clone()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.cancel(task1_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_ready_tasks() {
        let mut mock_repo = MockTaskRepo::new();

        let mut task1 = create_test_task("Ready task 1");
        task1.status = TaskStatus::Ready;

        let mut task2 = create_test_task("Ready task 2");
        task2.status = TaskStatus::Ready;

        let ready_tasks = vec![task1, task2];
        let ready_clone = ready_tasks.clone();

        mock_repo
            .expect_get_ready_tasks()
            .returning(move |_| Ok(ready_clone.clone()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.get_ready_tasks(Some(10)).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_validate_summary_too_long() {
        let mock_repo = MockTaskRepo::new();

        let mut task = create_test_task("Test");
        task.summary = "a".repeat(141);

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.submit(task).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_priority_out_of_range() {
        let mock_repo = MockTaskRepo::new();

        let mut task = create_test_task("Test");
        task.priority = 11;

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.submit(task).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resolve_dependencies_after_task_completion() {
        let mut mock_repo = MockTaskRepo::new();

        // Create task1 (dependency)
        let mut task1 = create_test_task("Task 1");
        task1.status = TaskStatus::Running;
        let task1_id = task1.id;

        // Create task2 (depends on task1)
        let mut task2 = create_test_task("Task 2");
        task2.dependencies = Some(vec![task1_id]);
        task2.status = TaskStatus::Blocked;
        let _task2_id = task2.id;

        // Mock get_dependents to return task2 when task1 completes
        let task2_clone = task2.clone();
        mock_repo
            .expect_get_dependents()
            .with(eq(task1_id))
            .returning(move |_| Ok(vec![task2_clone.clone()]));

        // Mock get for task1 status update
        let task1_clone = task1.clone();
        mock_repo
            .expect_get()
            .with(eq(task1_id))
            .returning(move |_| Ok(Some(task1_clone.clone())));

        // Mock update for task1 status change to Completed
        mock_repo
            .expect_update()
            .times(1)
            .returning(|_| Ok(()));

        // Mock list to return both tasks for dependency resolution
        let mut completed_task1 = task1.clone();
        completed_task1.status = TaskStatus::Completed;
        let all_tasks = vec![completed_task1.clone(), task2.clone()];
        mock_repo
            .expect_list()
            .returning(move |_| Ok(all_tasks.clone()));

        // Mock update for task2 when it transitions to Ready
        mock_repo
            .expect_update()
            .times(1)
            .returning(|t| {
                assert_eq!(t.status, TaskStatus::Ready);
                Ok(())
            });

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        // Complete task1 via the trait method
        use crate::domain::ports::TaskQueueService as TaskQueueServiceTrait;
        let result = service.update_task_status(task1_id, TaskStatus::Completed).await;
        assert!(result.is_ok());
    }
}

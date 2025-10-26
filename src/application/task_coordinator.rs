use crate::domain::models::task::{Task, TaskStatus};
use crate::domain::ports::{PriorityCalculator, TaskQueueService};
use crate::services::DependencyResolver;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

/// Status update message for task lifecycle events
#[derive(Debug, Clone)]
pub struct TaskStatusUpdate {
    pub task_id: Uuid,
    pub old_status: TaskStatus,
    pub new_status: TaskStatus,
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
/// ```
/// use abathur::application::TaskCoordinator;
/// use std::sync::Arc;
///
/// let coordinator = TaskCoordinator::new(
///     Arc::clone(&task_queue),
///     Arc::clone(&dependency_resolver),
///     Arc::clone(&priority_calc),
/// );
///
/// // Coordinate a task through its lifecycle
/// coordinator.coordinate_task_lifecycle(task_id).await?;
///
/// // Get the next ready task
/// let next_task = coordinator.get_next_ready_task().await?;
/// ```
pub struct TaskCoordinator {
    task_queue: Arc<dyn TaskQueueService>,
    #[allow(dead_code)] // Reserved for future complex dependency resolution
    dependency_resolver: Arc<DependencyResolver>,
    priority_calc: Arc<dyn PriorityCalculator>,
    status_tx: mpsc::Sender<TaskStatusUpdate>,
    status_rx: Option<mpsc::Receiver<TaskStatusUpdate>>,
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
        }
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

        if task.status != new_status {
            self.task_queue
                .update_task_status(task_id, new_status.clone())
                .await
                .context("Failed to update task status")?;

            // Notify status change
            let _ = self
                .status_tx
                .send(TaskStatusUpdate {
                    task_id,
                    old_status: task.status.clone(),
                    new_status: new_status.clone(),
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
    /// * `Ok(())` - Task marked complete and dependents triggered
    /// * `Err` - If task not found or database error
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub async fn handle_task_completion(&self, task_id: Uuid) -> Result<()> {
        info!("Handling task completion for task {}", task_id);

        // 1. Mark task as completed
        self.task_queue
            .update_task_status(task_id, TaskStatus::Completed)
            .await
            .context("Failed to mark task as completed")?;

        // Notify status change
        let _ = self
            .status_tx
            .send(TaskStatusUpdate {
                task_id,
                old_status: TaskStatus::Running,
                new_status: TaskStatus::Completed,
            })
            .await;

        // 2. Get all dependent tasks
        let dependent_tasks = self
            .task_queue
            .get_dependent_tasks(task_id)
            .await
            .context("Failed to get dependent tasks")?;

        info!(
            "Found {} dependent tasks for task {}",
            dependent_tasks.len(),
            task_id
        );

        // 3. Trigger lifecycle coordination for each dependent task
        for dependent_task in dependent_tasks {
            if let Err(e) = self.coordinate_task_lifecycle(dependent_task.id).await {
                warn!(
                    "Failed to coordinate dependent task {}: {:?}",
                    dependent_task.id, e
                );
            }
        }

        Ok(())
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

        // TODO: Implement retry logic
        // - Check retry_count vs max_retries
        // - Reset task to pending if retries remain
        // - Exponential backoff for retry scheduling

        // TODO: Implement cascade failure logic
        // - Optionally mark dependent tasks as blocked or cancelled
        // - Provide configuration for failure handling strategy

        Ok(())
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
                    t.dependencies.as_ref().is_some_and(|deps| deps.contains(&task_id))
                })
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
            Ok(tasks.iter().map(|t| (t.id, f64::from(t.priority))).collect())
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
            task_branch: None,
            worktree_path: None,
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
    async fn test_handle_task_failure() {
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

        task_queue.add_task(task);

        // Mark task as failed
        coordinator
            .handle_task_failure(task_id, "Test error".to_string())
            .await
            .unwrap();

        // Task should be failed with error message
        let updated_task = task_queue.get_task(task_id).await.unwrap();
        assert_eq!(updated_task.status, TaskStatus::Failed);
        assert_eq!(updated_task.error_message, Some("Test error".to_string()));
    }
}

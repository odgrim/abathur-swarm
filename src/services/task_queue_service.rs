use crate::domain::models::{Task, TaskStatus, PruneResult, BlockedTask};
use crate::domain::ports::{TaskFilters, TaskRepository};
use crate::services::{DependencyResolver, PriorityCalculator, MemoryService};
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
    memory_service: Option<Arc<MemoryService>>,
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
            memory_service: None,
        }
    }

    /// Create a new TaskQueueService with memory service for task memory cleanup
    pub fn with_memory_service(
        repo: Arc<dyn TaskRepository>,
        dependency_resolver: DependencyResolver,
        priority_calc: PriorityCalculator,
        memory_service: Arc<MemoryService>,
    ) -> Self {
        Self {
            repo,
            dependency_resolver,
            priority_calc,
            memory_service: Some(memory_service),
        }
    }

    /// Delete all memory associated with a task
    ///
    /// Tasks store memory with namespace pattern `task:{task_id}:*`.
    /// This method searches for and deletes all memory entries under that namespace.
    ///
    /// # Arguments
    /// * `task_id` - The UUID of the task whose memory should be deleted
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of memory entries deleted
    /// * `Err(_)` - If memory deletion fails
    #[instrument(skip(self), err)]
    async fn delete_task_memory(&self, task_id: Uuid) -> Result<usize> {
        // If no memory service is configured, skip memory deletion
        let Some(ref memory_service) = self.memory_service else {
            info!("No memory service configured, skipping memory deletion for task {}", task_id);
            return Ok(0);
        };

        // Search for all memory entries with namespace prefix task:{task_id}:
        let namespace_prefix = format!("task:{}:", task_id);

        info!("Searching for task memory with prefix: {}", namespace_prefix);
        let memories = memory_service
            .search(&namespace_prefix, None, Some(1000))
            .await
            .context("Failed to search for task memory")?;

        let count = memories.len();
        if count == 0 {
            info!("No memory entries found for task {}", task_id);
            return Ok(0);
        }

        info!("Found {} memory entries for task {}, deleting...", count, task_id);

        // Delete each memory entry
        for memory in memories {
            if let Err(e) = memory_service.delete(&memory.namespace, &memory.key).await {
                warn!(
                    "Failed to delete memory {}:{} for task {}: {}",
                    memory.namespace, memory.key, task_id, e
                );
            }
        }

        info!("Deleted {} memory entries for task {}", count, task_id);
        Ok(count)
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

    /// Validate and prune tasks with dependency validation.
    ///
    /// This method validates whether tasks can be safely deleted by checking
    /// if all their dependent tasks are in terminal states (completed, failed,
    /// or cancelled). It performs a two-phase operation:
    ///
    /// **Phase 1 - Validation**:
    /// - Verify each task exists
    /// - Get all tasks that depend on each task
    /// - Check if ALL dependents are in terminal states
    /// - Categorize tasks as deletable or blocked
    ///
    /// **Phase 2 - Deletion** (only if `dry_run = false`):
    /// - Delete all tasks that passed validation
    ///
    /// # Arguments
    /// * `task_ids` - UUIDs of tasks to validate and prune
    /// * `dry_run` - If true, only validate without performing deletion
    ///
    /// # Returns
    /// `PruneResult` containing:
    /// - `deleted_count`: Number of tasks actually deleted
    /// - `deleted_ids`: UUIDs of deleted tasks
    /// - `blocked_tasks`: Tasks that couldn't be deleted with reasons
    /// - `dry_run`: Whether this was a dry-run
    ///
    /// # Errors
    /// - Database query failures
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use abathur::services::TaskQueueService;
    /// # use std::sync::Arc;
    /// # async fn example(service: TaskQueueService, task_id: uuid::Uuid) -> anyhow::Result<()> {
    /// // Dry run to check what would be deleted
    /// let result = service.validate_and_prune_tasks(vec![task_id], true).await?;
    /// println!("Would delete {} tasks", result.deleted_ids.len());
    /// println!("Blocked: {} tasks", result.blocked_tasks.len());
    ///
    /// // Actually delete if safe
    /// if result.blocked_tasks.is_empty() {
    ///     let result = service.validate_and_prune_tasks(vec![task_id], false).await?;
    ///     println!("Deleted {} tasks", result.deleted_count);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), err)]
    pub async fn validate_and_prune_tasks(
        &self,
        task_ids: Vec<Uuid>,
        dry_run: bool,
    ) -> Result<PruneResult> {
        info!(
            "Validating {} tasks for pruning (dry_run: {})",
            task_ids.len(),
            dry_run
        );

        let mut deletable_tasks = Vec::new();
        let mut blocked_tasks = Vec::new();

        // PHASE 1: VALIDATION
        for task_id in task_ids {
            // 1. Verify task exists
            let _task = match self.repo.get(task_id).await {
                Ok(Some(t)) => t,
                Ok(None) => {
                    warn!("Task {} not found, skipping", task_id);
                    blocked_tasks.push(BlockedTask {
                        task_id,
                        reason: "Task not found".to_string(),
                        non_terminal_dependents: vec![],
                    });
                    continue;
                }
                Err(e) => {
                    warn!("Failed to fetch task {}: {}", task_id, e);
                    blocked_tasks.push(BlockedTask {
                        task_id,
                        reason: format!("Failed to fetch task: {}", e),
                        non_terminal_dependents: vec![],
                    });
                    continue;
                }
            };

            // 2. Get all dependent tasks
            let dependent_tasks = match self.repo.get_dependents(task_id).await {
                Ok(deps) => deps,
                Err(e) => {
                    warn!("Failed to get dependents for task {}: {}", task_id, e);
                    blocked_tasks.push(BlockedTask {
                        task_id,
                        reason: format!("Failed to fetch dependents: {}", e),
                        non_terminal_dependents: vec![],
                    });
                    continue;
                }
            };

            // 3. Check if ALL dependents are terminal
            let non_terminal_dependents: Vec<Uuid> = dependent_tasks
                .iter()
                .filter(|dep| !is_terminal_status(dep.status))
                .map(|dep| dep.id)
                .collect();

            if non_terminal_dependents.is_empty() {
                // 4. All dependents are terminal (or no dependents) - safe to delete
                info!(
                    "Task {} is deletable ({} terminal dependents)",
                    task_id,
                    dependent_tasks.len()
                );
                deletable_tasks.push(task_id);
            } else {
                // 5. Has non-terminal dependents - block deletion
                warn!(
                    "Task {} blocked from deletion: {} non-terminal dependents",
                    task_id,
                    non_terminal_dependents.len()
                );
                blocked_tasks.push(BlockedTask {
                    task_id,
                    reason: format!(
                        "Task has {} non-terminal dependent(s)",
                        non_terminal_dependents.len()
                    ),
                    non_terminal_dependents,
                });
            }
        }

        // PHASE 2: DELETION (if not dry run)
        let deleted_ids = if !dry_run && !deletable_tasks.is_empty() {
            info!("Deleting {} tasks", deletable_tasks.len());

            // Delete tasks one by one since we don't have delete_batch yet
            let mut successfully_deleted = Vec::new();
            for task_id in &deletable_tasks {
                // First, delete the task from the repository
                match self.repo.delete(*task_id).await {
                    Ok(_) => {
                        info!("Successfully deleted task {}", task_id);

                        // Then, delete associated memory
                        match self.delete_task_memory(*task_id).await {
                            Ok(count) => {
                                if count > 0 {
                                    info!("Deleted {} memory entries for task {}", count, task_id);
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "Task {} deleted but failed to delete memory: {}",
                                    task_id, e
                                );
                                // Don't fail the entire operation if memory deletion fails
                                // The task is already deleted, so we continue
                            }
                        }

                        successfully_deleted.push(*task_id);
                    }
                    Err(e) => {
                        warn!("Failed to delete task {}: {}", task_id, e);
                        // Move from deletable to blocked
                        blocked_tasks.push(BlockedTask {
                            task_id: *task_id,
                            reason: format!("Deletion failed: {}", e),
                            non_terminal_dependents: vec![],
                        });
                    }
                }
            }
            successfully_deleted
        } else {
            if dry_run {
                info!("Dry run: would delete {} tasks", deletable_tasks.len());
            }
            vec![]
        };

        let result = PruneResult {
            deleted_count: deleted_ids.len(),
            deleted_ids: deleted_ids.clone(),
            blocked_tasks,
            dry_run,
        };

        info!(
            "Pruning complete: deleted={}, blocked={}, dry_run={}",
            result.deleted_count,
            result.blocked_tasks.len(),
            result.dry_run
        );

        Ok(result)
    }
}

/// Check if a task status is terminal (completed, failed, or cancelled).
///
/// Terminal states indicate that a task will never execute or change state again,
/// making it safe to delete tasks that depend on them.
fn is_terminal_status(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
    )
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

    async fn update_task(&self, task: &Task) -> Result<()> {
        self.repo
            .update(task)
            .await
            .context("Failed to update task")
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

    async fn claim_next_ready_task(&self) -> Result<Option<Task>> {
        // Use the repository's atomic claim implementation
        self.repo
            .claim_next_ready_task()
            .await
            .context("Failed to atomically claim next ready task")
    }

    async fn submit_task(&self, task: Task) -> Result<Uuid> {
        self.submit(task).await
    }

    async fn get_stale_running_tasks(&self, stale_threshold_secs: u64) -> Result<Vec<Task>> {
        self.repo
            .get_stale_running_tasks(stale_threshold_secs)
            .await
            .context("Failed to get stale running tasks")
    }

    async fn task_exists_by_idempotency_key(&self, idempotency_key: &str) -> Result<bool> {
        self.repo
            .task_exists_by_idempotency_key(idempotency_key)
            .await
            .context("Failed to check if task exists by idempotency key")
    }

    async fn submit_task_idempotent(&self, mut task: Task) -> Result<crate::domain::ports::task_repository::IdempotentInsertResult> {
        use crate::domain::ports::task_repository::IdempotentInsertResult;

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
        } else {
            // No dependencies - depth is 0
            self.priority_calc.update_task_priority(&mut task, 0);
        }

        // 7. Insert atomically with idempotency check
        let result = self.repo
            .insert_task_idempotent(&task)
            .await
            .context("Failed to insert task idempotently")?;

        match &result {
            IdempotentInsertResult::Inserted(task_id) => {
                info!(
                    "Task {} submitted idempotently (new task created)",
                    task_id
                );
                // 8. Re-resolve dependencies in case this new task completes dependencies for other tasks
                self.resolve_dependencies().await
                    .context("Failed to resolve dependencies after task submission")?;
            }
            IdempotentInsertResult::AlreadyExists => {
                info!(
                    "Task with idempotency key {:?} already exists, skipping",
                    task.idempotency_key
                );
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::task_repository::IdempotentInsertResult;
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
            async fn claim_next_ready_task(&self) -> Result<Option<Task>, DatabaseError>;
            async fn get_stale_running_tasks(&self, stale_threshold_secs: u64) -> Result<Vec<Task>, DatabaseError>;
            async fn task_exists_by_idempotency_key(&self, idempotency_key: &str) -> Result<bool, DatabaseError>;
            async fn insert_task_idempotent(&self, task: &Task) -> Result<IdempotentInsertResult, DatabaseError>;
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

    // ==================== PRUNE TESTS ====================

    #[tokio::test]
    async fn test_validate_task_no_dependents_ok() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task with no dependents");
        let task_id = task.id;
        let task_clone = task.clone();

        // Task exists
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // No dependents
        mock_repo
            .expect_get_dependents()
            .with(eq(task_id))
            .returning(|_| Ok(vec![]));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task_id], true)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_ids.len(), 0); // Dry run
        assert_eq!(prune_result.blocked_tasks.len(), 0);
        assert!(prune_result.dry_run);
    }

    #[tokio::test]
    async fn test_validate_task_terminal_dependents_ok() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task to delete");
        let task_id = task.id;
        let task_clone = task.clone();

        // Create terminal dependent tasks
        let mut dep1 = create_test_task("Completed dependent");
        dep1.status = TaskStatus::Completed;
        dep1.dependencies = Some(vec![task_id]);

        let mut dep2 = create_test_task("Failed dependent");
        dep2.status = TaskStatus::Failed;
        dep2.dependencies = Some(vec![task_id]);

        let mut dep3 = create_test_task("Cancelled dependent");
        dep3.status = TaskStatus::Cancelled;
        dep3.dependencies = Some(vec![task_id]);

        // Task exists
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // All dependents are terminal
        let deps = vec![dep1.clone(), dep2.clone(), dep3.clone()];
        mock_repo
            .expect_get_dependents()
            .with(eq(task_id))
            .returning(move |_| Ok(deps.clone()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task_id], true)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_ids.len(), 0); // Dry run
        assert_eq!(prune_result.blocked_tasks.len(), 0);
        assert!(prune_result.dry_run);
    }

    #[tokio::test]
    async fn test_validate_task_non_terminal_dependents_blocked() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task to delete");
        let task_id = task.id;
        let task_clone = task.clone();

        // Create non-terminal dependent tasks
        let mut dep1 = create_test_task("Running dependent");
        dep1.status = TaskStatus::Running;
        dep1.dependencies = Some(vec![task_id]);

        let mut dep2 = create_test_task("Ready dependent");
        dep2.status = TaskStatus::Ready;
        dep2.dependencies = Some(vec![task_id]);

        // Task exists
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // Has non-terminal dependents
        let deps = vec![dep1.clone(), dep2.clone()];
        let dep1_id = dep1.id;
        let dep2_id = dep2.id;
        mock_repo
            .expect_get_dependents()
            .with(eq(task_id))
            .returning(move |_| Ok(deps.clone()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task_id], true)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_ids.len(), 0);
        assert_eq!(prune_result.blocked_tasks.len(), 1);
        assert!(prune_result.dry_run);

        let blocked = &prune_result.blocked_tasks[0];
        assert_eq!(blocked.task_id, task_id);
        assert_eq!(blocked.non_terminal_dependents.len(), 2);
        assert!(blocked.non_terminal_dependents.contains(&dep1_id));
        assert!(blocked.non_terminal_dependents.contains(&dep2_id));
    }

    #[tokio::test]
    async fn test_prune_dry_run_no_deletion() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task to delete");
        let task_id = task.id;
        let task_clone = task.clone();

        // Task exists
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // No dependents
        mock_repo
            .expect_get_dependents()
            .with(eq(task_id))
            .returning(|_| Ok(vec![]));

        // Should NOT call delete in dry run
        mock_repo.expect_delete().times(0);

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task_id], true)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_count, 0);
        assert_eq!(prune_result.deleted_ids.len(), 0);
        assert!(prune_result.dry_run);
    }

    #[tokio::test]
    async fn test_prune_actual_deletion_calls_repository() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task to delete");
        let task_id = task.id;
        let task_clone = task.clone();

        // Task exists
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // No dependents
        mock_repo
            .expect_get_dependents()
            .with(eq(task_id))
            .returning(|_| Ok(vec![]));

        // Should call delete when not dry run
        mock_repo
            .expect_delete()
            .with(eq(task_id))
            .times(1)
            .returning(|_| Ok(()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task_id], false)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_count, 1);
        assert_eq!(prune_result.deleted_ids.len(), 1);
        assert_eq!(prune_result.deleted_ids[0], task_id);
        assert!(!prune_result.dry_run);
    }

    #[tokio::test]
    async fn test_prune_empty_list() {
        let mock_repo = MockTaskRepo::new();

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service.validate_and_prune_tasks(vec![], false).await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_count, 0);
        assert_eq!(prune_result.deleted_ids.len(), 0);
        assert_eq!(prune_result.blocked_tasks.len(), 0);
    }

    #[tokio::test]
    async fn test_prune_all_blocked() {
        let mut mock_repo = MockTaskRepo::new();

        let task1 = create_test_task("Task 1");
        let task1_id = task1.id;
        let task1_clone = task1.clone();

        let task2 = create_test_task("Task 2");
        let task2_id = task2.id;
        let task2_clone = task2.clone();

        // Both tasks have non-terminal dependents
        let mut dep1 = create_test_task("Dep 1");
        dep1.status = TaskStatus::Running;

        let mut dep2 = create_test_task("Dep 2");
        dep2.status = TaskStatus::Ready;

        // Task 1
        mock_repo
            .expect_get()
            .with(eq(task1_id))
            .returning(move |_| Ok(Some(task1_clone.clone())));
        let dep1_for_task1 = dep1.clone();
        mock_repo
            .expect_get_dependents()
            .with(eq(task1_id))
            .returning(move |_| Ok(vec![dep1_for_task1.clone()]));

        // Task 2
        mock_repo
            .expect_get()
            .with(eq(task2_id))
            .returning(move |_| Ok(Some(task2_clone.clone())));
        let dep2_for_task2 = dep2.clone();
        mock_repo
            .expect_get_dependents()
            .with(eq(task2_id))
            .returning(move |_| Ok(vec![dep2_for_task2.clone()]));

        // No deletions should occur
        mock_repo.expect_delete().times(0);

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task1_id, task2_id], false)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_count, 0);
        assert_eq!(prune_result.blocked_tasks.len(), 2);
    }

    #[tokio::test]
    async fn test_prune_with_dependency_chain() {
        let mut mock_repo = MockTaskRepo::new();

        // Create chain: task1 <- task2 <- task3
        // task1 is a dependency of task2, task2 is a dependency of task3
        let task1 = create_test_task("Task 1");
        let task1_id = task1.id;
        let task1_clone = task1.clone();

        let mut task2 = create_test_task("Task 2");
        task2.dependencies = Some(vec![task1_id]);
        task2.status = TaskStatus::Completed; // Terminal
        let task2_id = task2.id;
        let _task2_clone = task2.clone();

        let mut task3 = create_test_task("Task 3");
        task3.dependencies = Some(vec![task2_id]);
        task3.status = TaskStatus::Completed; // Terminal
        let _task3_clone = task3.clone();

        // Try to delete task1 - should succeed since task2 and task3 are terminal
        mock_repo
            .expect_get()
            .with(eq(task1_id))
            .returning(move |_| Ok(Some(task1_clone.clone())));

        // task1 has task2 as dependent
        let task2_for_deps = task2.clone();
        mock_repo
            .expect_get_dependents()
            .with(eq(task1_id))
            .returning(move |_| Ok(vec![task2_for_deps.clone()]));

        // Actually delete task1
        mock_repo
            .expect_delete()
            .with(eq(task1_id))
            .times(1)
            .returning(|_| Ok(()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task1_id], false)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.deleted_count, 1);
        assert_eq!(prune_result.deleted_ids[0], task1_id);
        assert_eq!(prune_result.blocked_tasks.len(), 0);
    }

    #[tokio::test]
    async fn test_prune_respects_terminal_states() {
        let mut mock_repo = MockTaskRepo::new();

        let task = create_test_task("Task to delete");
        let task_id = task.id;
        let task_clone = task.clone();

        // Create one terminal and one non-terminal dependent
        let mut dep1 = create_test_task("Completed dependent");
        dep1.status = TaskStatus::Completed;

        let mut dep2 = create_test_task("Pending dependent");
        dep2.status = TaskStatus::Pending; // Non-terminal

        // Task exists
        mock_repo
            .expect_get()
            .with(eq(task_id))
            .returning(move |_| Ok(Some(task_clone.clone())));

        // Has mixed dependents - should be blocked
        let deps = vec![dep1.clone(), dep2.clone()];
        let dep2_id = dep2.id;
        mock_repo
            .expect_get_dependents()
            .with(eq(task_id))
            .returning(move |_| Ok(deps.clone()));

        let service = TaskQueueService::new(
            Arc::new(mock_repo),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        );

        let result = service
            .validate_and_prune_tasks(vec![task_id], true)
            .await;

        assert!(result.is_ok());
        let prune_result = result.unwrap();
        assert_eq!(prune_result.blocked_tasks.len(), 1);
        assert_eq!(prune_result.blocked_tasks[0].non_terminal_dependents.len(), 1);
        assert_eq!(
            prune_result.blocked_tasks[0].non_terminal_dependents[0],
            dep2_id
        );
    }
}

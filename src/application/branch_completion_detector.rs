///! Branch Completion Detector
///!
///! Detects when all tasks in a branch (task or feature) have reached terminal state
///! and triggers branch completion hooks.

use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::{BranchCompletionContext, BranchType, Task};
use crate::domain::ports::{TaskFilters, TaskRepository};
use anyhow::Result;
use std::sync::Arc;
use tracing::{debug, info, instrument};
use uuid::Uuid;

/// Detector for branch completion events
pub struct BranchCompletionDetector {
    #[allow(dead_code)]
    task_coordinator: Arc<TaskCoordinator>,
    repository: Arc<dyn TaskRepository>,
}

impl BranchCompletionDetector {
    /// Create a new branch completion detector
    pub fn new(task_coordinator: Arc<TaskCoordinator>, repository: Arc<dyn TaskRepository>) -> Self {
        Self { task_coordinator, repository }
    }

    /// Check if a branch is complete after a task status change
    ///
    /// This should be called whenever a task transitions to a terminal state.
    /// It checks if all tasks in the same branch are now terminal.
    ///
    /// # Arguments
    ///
    /// * `completed_task` - The task that just completed
    ///
    /// # Returns
    ///
    /// Some(BranchCompletionContext) if the branch is complete, None otherwise
    #[instrument(skip(self, completed_task), fields(task_id = %completed_task.id))]
    pub async fn check_branch_completion(
        &self,
        completed_task: &Task,
    ) -> Result<Option<BranchCompletionContext>> {
        // Only check when task reaches terminal state
        if !completed_task.is_terminal() {
            debug!("Task not terminal, skipping branch completion check");
            return Ok(None);
        }

        // Determine which branch to check
        let (branch_name, branch_type) = if let Some(ref branch) = completed_task.branch {
            // Determine branch type based on branch name prefix
            let branch_type = if branch.starts_with("task/") {
                BranchType::TaskBranch
            } else if branch.starts_with("feature/") {
                BranchType::FeatureBranch
            } else {
                debug!("Branch '{}' doesn't match known patterns (task/* or feature/*), skipping", branch);
                return Ok(None);
            };
            (branch.clone(), branch_type)
        } else {
            debug!("Task has no branch association, skipping check");
            return Ok(None);
        };

        info!(
            branch_name = %branch_name,
            branch_type = ?branch_type,
            "Checking branch completion"
        );

        // Get all tasks for this branch
        let branch_tasks = self.get_branch_tasks(&branch_name, branch_type).await?;

        if branch_tasks.is_empty() {
            debug!("No tasks found for branch");
            return Ok(None);
        }

        // Check if ALL tasks are terminal
        let all_terminal = branch_tasks.iter().all(|t| t.is_terminal());

        if !all_terminal {
            let pending_count = branch_tasks.iter().filter(|t| !t.is_terminal()).count();
            debug!(
                pending_tasks = pending_count,
                total_tasks = branch_tasks.len(),
                "Branch not yet complete"
            );
            return Ok(None);
        }

        // Branch is complete! Collect completion statistics
        let all_succeeded = branch_tasks.iter().all(|t| t.is_completed());
        let failed_tasks: Vec<Task> = branch_tasks
            .iter()
            .filter(|t| t.is_failed() || t.is_cancelled())
            .cloned()
            .collect();

        let failed_task_ids: Vec<Uuid> = failed_tasks.iter().map(|t| t.id).collect();
        let completed_task_ids: Vec<Uuid> = branch_tasks.iter().map(|t| t.id).collect();

        // Collect unique agent types
        let mut agent_types: Vec<String> = branch_tasks
            .iter()
            .map(|t| t.agent_type.clone())
            .collect();
        agent_types.sort();
        agent_types.dedup();

        let context = BranchCompletionContext {
            branch_name: branch_name.clone(),
            branch_type,
            completed_task_ids,
            feature_branch: completed_task.feature_branch.clone(),
            all_succeeded,
            failed_task_count: failed_tasks.len(),
            failed_task_ids,
            total_tasks: branch_tasks.len(),
            completed_agent_types: agent_types,
        };

        info!(
            branch_name = %branch_name,
            total_tasks = context.total_tasks,
            all_succeeded = context.all_succeeded,
            failed_count = context.failed_task_count,
            "Branch completion detected"
        );

        Ok(Some(context))
    }

    /// Get all tasks for a specific branch
    async fn get_branch_tasks(
        &self,
        branch_name: &str,
        branch_type: BranchType,
    ) -> Result<Vec<Task>> {
        match branch_type {
            BranchType::TaskBranch => {
                // Get tasks by branch field
                let filters = TaskFilters {
                    branch: Some(branch_name.to_string()),
                    ..Default::default()
                };
                self.repository
                    .list(&filters)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to get tasks by branch: {}", e))
            }
            BranchType::FeatureBranch => {
                // Get tasks by feature_branch field
                let filters = TaskFilters {
                    feature_branch: Some(branch_name.to_string()),
                    ..Default::default()
                };
                self.repository
                    .list(&filters)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to get tasks by feature branch: {}", e))
            }
        }
    }

    /// Check multiple branches for completion (batch operation)
    ///
    /// Useful for periodic checks or when system starts up.
    pub async fn check_all_branches(&self) -> Result<Vec<BranchCompletionContext>> {
        debug!("Checking all branches for completion");

        // Get all tasks
        let all_tasks = self
            .repository
            .list(&TaskFilters::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list all tasks: {}", e))?;

        // Group tasks by branch
        let mut task_branches: std::collections::HashMap<String, Vec<Task>> =
            std::collections::HashMap::new();
        let mut feature_branches: std::collections::HashMap<String, Vec<Task>> =
            std::collections::HashMap::new();

        for task in all_tasks {
            if let Some(branch) = &task.branch {
                if branch.starts_with("task/") {
                    task_branches
                        .entry(branch.clone())
                        .or_default()
                        .push(task.clone());
                }
            }
            if let Some(feature_branch) = &task.feature_branch {
                feature_branches
                    .entry(feature_branch.clone())
                    .or_default()
                    .push(task);
            }
        }

        let mut completed_branches = Vec::new();

        // Check task branches
        for (_branch_name, tasks) in task_branches {
            if tasks.iter().all(|t| t.is_terminal()) {
                if let Some(first_task) = tasks.first() {
                    if let Some(context) = self.check_branch_completion(first_task).await? {
                        completed_branches.push(context);
                    }
                }
            }
        }

        // Check feature branches
        for (branch_name, tasks) in feature_branches {
            if tasks.iter().all(|t| t.is_terminal()) {
                if let Some(first_task) = tasks.first() {
                    if let Some(context) = self.check_branch_completion(first_task).await? {
                        // Only add if not already in list (avoid duplicates)
                        if !completed_branches
                            .iter()
                            .any(|c| c.branch_name == branch_name)
                        {
                            completed_branches.push(context);
                        }
                    }
                }
            }
        }

        info!(
            completed_count = completed_branches.len(),
            "Branch completion check complete"
        );

        Ok(completed_branches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::TaskRepository;
    use crate::infrastructure::database::TaskRepositoryImpl;
    use crate::services::{DependencyResolver, PriorityCalculator, TaskQueueService};
    use tempfile::tempdir;

    async fn create_test_coordinator() -> Result<(Arc<TaskCoordinator>, Arc<dyn TaskRepository>)> {
        use sqlx::sqlite::SqlitePoolOptions;

        // Create an in-memory SQLite database for testing
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await?;

        let repo: Arc<dyn TaskRepository> = Arc::new(TaskRepositoryImpl::new(pool));
        let dependency_resolver = Arc::new(DependencyResolver::new());
        let priority_calc = Arc::new(PriorityCalculator::new());
        let queue_service = Arc::new(TaskQueueService::new(
            repo.clone(),
            DependencyResolver::new(),
            PriorityCalculator::new(),
        ));

        let coordinator = Arc::new(TaskCoordinator::new(
            queue_service,
            dependency_resolver,
            priority_calc,
        ));
        Ok((coordinator, repo))
    }

    async fn create_test_task_with_branch(
        branch: Option<String>,
        feature_branch: Option<String>,
        status: TaskStatus,
    ) -> Task {
        let mut task = Task::new("Test task".to_string(), "Test description".to_string());
        task.branch = branch;
        task.feature_branch = feature_branch;
        task.status = status;
        task
    }

    #[tokio::test]
    async fn test_no_completion_when_task_not_terminal() {
        let (coordinator, repo) = create_test_coordinator().await.unwrap();
        let detector = BranchCompletionDetector::new(coordinator, repo);

        let task = create_test_task_with_branch(
            Some("task/test-123".to_string()),
            None,
            TaskStatus::Running,
        )
        .await;

        let result = detector.check_branch_completion(&task).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_no_completion_when_no_branch() {
        let (coordinator, repo) = create_test_coordinator().await.unwrap();
        let detector = BranchCompletionDetector::new(coordinator, repo);

        let task = create_test_task_with_branch(None, None, TaskStatus::Completed).await;

        let result = detector.check_branch_completion(&task).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_branch_completion_with_single_task() {
        let (coordinator, repo) = create_test_coordinator().await.unwrap();
        let detector = BranchCompletionDetector::new(coordinator.clone(), repo);

        let mut task = create_test_task_with_branch(
            Some("task/test-123".to_string()),
            Some("feature/test".to_string()),
            TaskStatus::Completed,
        )
        .await;

        // Submit task to coordinator
        let task_id = coordinator.submit_task(task.clone()).await.unwrap();
        task.id = task_id;

        // Complete the task
        coordinator
            .handle_task_completion(task_id)
            .await
            .unwrap();

        let completed_task = coordinator.get_task(task_id).await.unwrap();

        let result = detector
            .check_branch_completion(&completed_task)
            .await
            .unwrap();

        assert!(result.is_some());
        let context = result.unwrap();
        assert_eq!(context.branch_name, "task/test-123");
        assert_eq!(context.branch_type, BranchType::TaskBranch);
        assert_eq!(context.total_tasks, 1);
        assert!(context.all_succeeded);
    }

    #[tokio::test]
    async fn test_no_completion_with_pending_sibling_task() {
        let (coordinator, repo) = create_test_coordinator().await.unwrap();
        let detector = BranchCompletionDetector::new(coordinator.clone(), repo);

        // Create two tasks in same branch
        let mut task1 = create_test_task_with_branch(
            Some("task/test-123".to_string()),
            None,
            TaskStatus::Completed,
        )
        .await;
        let mut task2 = create_test_task_with_branch(
            Some("task/test-123".to_string()),
            None,
            TaskStatus::Running,
        )
        .await;

        let task1_id = coordinator.submit_task(task1.clone()).await.unwrap();
        let _task2_id = coordinator.submit_task(task2.clone()).await.unwrap();

        // Complete task1, but task2 is still running
        coordinator
            .handle_task_completion(task1_id)
            .await
            .unwrap();

        task1.id = task1_id;
        task1.status = TaskStatus::Completed;

        let result = detector.check_branch_completion(&task1).await.unwrap();
        // Should be None because task2 is still running
        assert!(result.is_none());
    }
}

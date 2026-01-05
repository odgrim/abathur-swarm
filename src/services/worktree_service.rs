//! Worktree Service
//!
//! Provides git worktree management for task isolation.
//! Each implementation task gets its own worktree branched from the feature branch,
//! allowing parallel task execution without git conflicts.

use crate::domain::models::Task;
use crate::domain::ports::TaskQueueService as TaskQueueServiceTrait;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, instrument, warn};

/// Service for managing git worktrees for tasks.
///
/// Creates isolated worktrees for implementation tasks, enabling parallel
/// execution without git conflicts. Each task gets its own branch and
/// worktree directory.
///
/// # Worktree Naming Convention
///
/// - Branch: `task/<feature_name>/<short_task_id>` (e.g., `task/my-feature/a1b2c3d4`)
/// - Worktree path: `.abathur/worktrees/task-<full_task_id>`
///
/// # Example
///
/// ```no_run
/// use abathur::services::WorktreeService;
/// use std::sync::Arc;
///
/// # async fn example(task_queue: Arc<dyn abathur::domain::ports::TaskQueueService>, mut task: abathur::domain::models::Task) -> anyhow::Result<()> {
/// let worktree_service = WorktreeService::new(task_queue);
/// let updated_task = worktree_service.setup_task_worktree(&mut task).await?;
/// # Ok(())
/// # }
/// ```
pub struct WorktreeService {
    task_queue: Arc<dyn TaskQueueServiceTrait>,
}

impl WorktreeService {
    /// Create a new WorktreeService with the given task queue service.
    pub fn new(task_queue: Arc<dyn TaskQueueServiceTrait>) -> Self {
        Self { task_queue }
    }

    /// Set up a git worktree for a task.
    ///
    /// This function:
    /// 1. Generates branch and worktree_path if not already set (requires feature_branch)
    /// 2. Verifies the feature branch exists
    /// 3. Creates the git worktree
    /// 4. Updates the task in the database
    /// 5. Updates the in-memory task object
    ///
    /// # Arguments
    ///
    /// * `task` - Mutable reference to the task. Will be updated with worktree fields.
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Worktree was created successfully
    /// * `Ok(false)` - Task doesn't need a worktree (no feature_branch set)
    /// * `Err` - If worktree creation fails
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Feature branch doesn't exist
    /// - Git worktree creation fails
    /// - Database update fails
    #[instrument(skip(self, task), fields(task_id = %task.id, feature_branch = ?task.feature_branch))]
    pub async fn setup_task_worktree(&self, task: &mut Task) -> Result<bool> {
        // Check for inconsistent state: task has branch/worktree_path but no feature_branch
        // This indicates a bug in task creation - all worktree tasks must have feature_branch
        if task.feature_branch.is_none() {
            if task.branch.is_some() || task.worktree_path.is_some() {
                error!(
                    task_id = %task.id,
                    branch = ?task.branch,
                    worktree_path = ?task.worktree_path,
                    "Task has branch/worktree_path but no feature_branch - inconsistent state"
                );
                return Err(anyhow::anyhow!(
                    "Task {} has branch ({:?}) or worktree_path ({:?}) set but no feature_branch. \
                     This is an inconsistent state - all tasks with worktree configuration must have feature_branch set.",
                    task.id,
                    task.branch,
                    task.worktree_path
                ));
            }

            // Task doesn't need a worktree - this is fine
            debug!(
                task_id = %task.id,
                "Task has no feature_branch, skipping worktree setup"
            );
            return Ok(false);
        }

        // Check if task explicitly doesn't need a worktree
        // When needs_worktree == Some(false), skip task worktree creation
        // This allows planning agents (e.g., technical-architect) to work without their own worktree
        if task.needs_worktree == Some(false) {
            debug!(
                task_id = %task.id,
                feature_branch = ?task.feature_branch,
                "Task has needs_worktree=false, skipping task worktree setup"
            );
            return Ok(false);
        }

        let feature_branch = task.feature_branch.clone().unwrap();

        info!(
            task_id = %task.id,
            feature_branch = %feature_branch,
            "Setting up task worktree"
        );

        // Generate branch and worktree_path if not already set
        let (branch, worktree_path) = self.generate_branch_metadata(task, &feature_branch);

        // Verify feature branch exists
        self.verify_feature_branch_exists(&feature_branch).await?;

        // Check if worktree already exists and is valid
        if self.is_valid_worktree(&worktree_path).await? {
            info!(
                task_id = %task.id,
                worktree_path = %worktree_path,
                "Valid worktree already exists, reusing"
            );
            // Update task fields even if worktree exists (in case they were missing)
            self.update_task_worktree_fields(task, &branch, &feature_branch, &worktree_path)
                .await?;
            return Ok(true);
        }

        // Create worktree parent directory if needed
        self.ensure_worktree_parent_exists(&worktree_path).await?;

        // Create the worktree
        self.create_worktree(&branch, &feature_branch, &worktree_path)
            .await?;

        // Update task in database and in-memory
        self.update_task_worktree_fields(task, &branch, &feature_branch, &worktree_path)
            .await?;

        info!(
            task_id = %task.id,
            branch = %branch,
            worktree_path = %worktree_path,
            "Task worktree created successfully"
        );

        Ok(true)
    }

    /// Validate that a task's worktree exists and is valid.
    ///
    /// Call this after `setup_task_worktree` to verify the worktree was actually created.
    /// This is a safety check to ensure tasks don't run in the wrong directory.
    ///
    /// # Arguments
    ///
    /// * `task` - The task to validate
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Worktree exists and is valid, or task doesn't require a worktree
    /// * `Err` - Worktree should exist but doesn't
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    pub async fn validate_worktree_exists(&self, task: &Task) -> Result<()> {
        // If task has no worktree_path, nothing to validate
        let Some(ref worktree_path) = task.worktree_path else {
            return Ok(());
        };

        // Validate the worktree actually exists
        if !self.is_valid_worktree(worktree_path).await? {
            error!(
                task_id = %task.id,
                worktree_path = %worktree_path,
                branch = ?task.branch,
                feature_branch = ?task.feature_branch,
                "Task has worktree_path set but worktree does not exist or is invalid"
            );
            return Err(anyhow::anyhow!(
                "Worktree validation failed for task {}: path '{}' does not exist or is not a valid git worktree. \
                 The worktree may have been cleaned up prematurely or creation failed silently.",
                task.id,
                worktree_path
            ));
        }

        debug!(
            task_id = %task.id,
            worktree_path = %worktree_path,
            "Worktree validation passed"
        );

        Ok(())
    }

    /// Generate branch name and worktree path for a task.
    ///
    /// If the task already has these fields set, returns the existing values.
    /// Otherwise generates new values following the naming convention.
    fn generate_branch_metadata(&self, task: &Task, feature_branch: &str) -> (String, String) {
        let branch = task.branch.clone().unwrap_or_else(|| {
            // Extract feature name from branch (e.g., "feature/my-feature" -> "my-feature")
            let feature_name = feature_branch
                .strip_prefix("feature/")
                .or_else(|| feature_branch.strip_prefix("features/"))
                .unwrap_or(feature_branch);

            // Generate short task ID (first 8 characters of UUID)
            let short_task_id = &task.id.to_string()[..8];

            format!("task/{}/{}", feature_name, short_task_id)
        });

        let worktree_path = task
            .worktree_path
            .clone()
            .unwrap_or_else(|| format!(".abathur/worktrees/task-{}", task.id));

        (branch, worktree_path)
    }

    /// Verify that the feature branch exists in the git repository.
    async fn verify_feature_branch_exists(&self, feature_branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", feature_branch)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .context("Failed to check if feature branch exists")?;

        if !output.success() {
            return Err(anyhow::anyhow!(
                "Feature branch '{}' does not exist. \
                The feature branch must be created first (typically by technical-requirements-specialist).",
                feature_branch
            ));
        }

        Ok(())
    }

    /// Check if a valid git worktree exists at the given path.
    async fn is_valid_worktree(&self, worktree_path: &str) -> Result<bool> {
        let path = Path::new(worktree_path);

        if !path.exists() {
            return Ok(false);
        }

        // Check if it has a .git file (worktrees have a file, not a directory)
        let git_file = path.join(".git");
        if !git_file.exists() || !git_file.is_file() {
            warn!(
                worktree_path = %worktree_path,
                "Directory exists but is not a valid worktree (missing .git file)"
            );
            // Remove invalid directory
            tokio::fs::remove_dir_all(path)
                .await
                .context("Failed to remove invalid worktree directory")?;
            return Ok(false);
        }

        // Verify it's a valid git worktree
        let output = Command::new("git")
            .current_dir(worktree_path)
            .args(["rev-parse", "--git-dir"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .context("Failed to verify worktree validity")?;

        Ok(output.success())
    }

    /// Create the parent directory for the worktree if it doesn't exist.
    async fn ensure_worktree_parent_exists(&self, worktree_path: &str) -> Result<()> {
        let path = Path::new(worktree_path);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                debug!(parent = ?parent, "Creating worktree parent directory");
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("Failed to create worktree parent directory")?;
            }
        }
        Ok(())
    }

    /// Create the git worktree.
    async fn create_worktree(
        &self,
        branch: &str,
        feature_branch: &str,
        worktree_path: &str,
    ) -> Result<()> {
        // Check if branch already exists
        let branch_exists = Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", branch)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        let output = if branch_exists {
            info!(
                branch = %branch,
                "Branch already exists, creating worktree from existing branch"
            );
            Command::new("git")
                .args(["worktree", "add", worktree_path, branch])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("Failed to create worktree from existing branch")?
        } else {
            info!(
                branch = %branch,
                feature_branch = %feature_branch,
                "Creating new branch from feature branch"
            );
            Command::new("git")
                .args(["worktree", "add", "-b", branch, worktree_path, feature_branch])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("Failed to create worktree with new branch")?
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                branch = %branch,
                worktree_path = %worktree_path,
                stderr = %stderr,
                "Failed to create git worktree"
            );
            return Err(anyhow::anyhow!("Git worktree creation failed: {}", stderr));
        }

        Ok(())
    }

    /// Update task fields in both the database and the in-memory object.
    async fn update_task_worktree_fields(
        &self,
        task: &mut Task,
        branch: &str,
        feature_branch: &str,
        worktree_path: &str,
    ) -> Result<()> {
        // Update in-memory task
        task.branch = Some(branch.to_string());
        task.feature_branch = Some(feature_branch.to_string());
        task.worktree_path = Some(worktree_path.to_string());
        task.last_updated_at = chrono::Utc::now();

        // Update in database
        self.task_queue
            .update_task(task)
            .await
            .context("Failed to update task with worktree fields")?;

        // CRITICAL: Increment version in-memory to match database.
        // The database update incremented the version, but the task object
        // still has the old version. Without this, subsequent updates will
        // fail with OptimisticLockConflict.
        task.version += 1;

        debug!(
            task_id = %task.id,
            branch = %branch,
            worktree_path = %worktree_path,
            "Task worktree fields updated in database"
        );

        Ok(())
    }

    /// Remove a task's worktree and optionally delete the branch.
    ///
    /// This should be called when a task completes and the worktree is no longer needed.
    /// Typically the merge orchestrator handles cleanup after merging.
    ///
    /// # Arguments
    ///
    /// * `task` - The task whose worktree should be removed
    /// * `delete_branch` - Whether to also delete the task branch
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Cleanup successful (or nothing to clean)
    /// * `Err` - If cleanup fails
    #[instrument(skip(self, task), fields(task_id = %task.id))]
    pub async fn cleanup_task_worktree(&self, task: &Task, delete_branch: bool) -> Result<()> {
        // Remove worktree if it exists
        if let Some(ref worktree_path) = task.worktree_path {
            if Path::new(worktree_path).exists() {
                info!(worktree_path = %worktree_path, "Removing task worktree");

                let output = Command::new("git")
                    .args(["worktree", "remove", worktree_path])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .context("Failed to remove worktree")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!(
                        worktree_path = %worktree_path,
                        stderr = %stderr,
                        "Failed to remove worktree, trying with --force"
                    );

                    // Try with force
                    let force_output = Command::new("git")
                        .args(["worktree", "remove", "--force", worktree_path])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                        .await?;

                    if !force_output.status.success() {
                        let force_stderr = String::from_utf8_lossy(&force_output.stderr);
                        error!(
                            worktree_path = %worktree_path,
                            stderr = %force_stderr,
                            "Failed to force remove worktree"
                        );
                    }
                }
            }
        }

        // Delete branch if requested
        if delete_branch {
            if let Some(ref branch) = task.branch {
                info!(branch = %branch, "Deleting task branch");

                let output = Command::new("git")
                    .args(["branch", "-d", branch])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .context("Failed to delete branch")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!(
                        branch = %branch,
                        stderr = %stderr,
                        "Failed to delete branch (may not be merged yet)"
                    );
                }
            }
        }

        Ok(())
    }

    /// Set up a worktree for a feature branch.
    ///
    /// Feature branches need their own worktrees to serve as merge targets
    /// for task branches. This enables conflict resolution without affecting
    /// the main working directory.
    ///
    /// # Arguments
    ///
    /// * `feature_branch` - The feature branch name (e.g., "feature/my-feature")
    ///
    /// # Returns
    ///
    /// * `Ok(worktree_path)` - The path to the created worktree
    /// * `Err` - If worktree creation fails
    #[instrument(skip(self), fields(feature_branch = %feature_branch))]
    pub async fn setup_feature_branch_worktree(&self, feature_branch: &str) -> Result<String> {
        // Generate worktree path from feature branch name
        // e.g., "feature/my-feature" -> ".abathur/worktrees/feature-my-feature"
        let sanitized_name = feature_branch
            .trim_start_matches("feature/")
            .trim_start_matches("features/")
            .replace('/', "-");
        let worktree_path = format!(".abathur/worktrees/feature-{}", sanitized_name);

        // Check if worktree already exists and is valid
        if self.is_valid_worktree(&worktree_path).await? {
            info!(
                feature_branch = %feature_branch,
                worktree_path = %worktree_path,
                "Feature branch worktree already exists"
            );
            return Ok(worktree_path);
        }

        // Verify the feature branch exists
        self.verify_feature_branch_exists(feature_branch).await?;

        // Ensure parent directory exists
        self.ensure_worktree_parent_exists(&worktree_path).await?;

        // Create the worktree (branch already exists, so just attach it)
        info!(
            feature_branch = %feature_branch,
            worktree_path = %worktree_path,
            "Creating worktree for feature branch"
        );

        let output = Command::new("git")
            .args(["worktree", "add", &worktree_path, feature_branch])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to create feature branch worktree")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                feature_branch = %feature_branch,
                worktree_path = %worktree_path,
                stderr = %stderr,
                "Failed to create feature branch worktree"
            );
            return Err(anyhow::anyhow!("Git worktree creation failed: {}", stderr));
        }

        info!(
            feature_branch = %feature_branch,
            worktree_path = %worktree_path,
            "Feature branch worktree created successfully"
        );

        Ok(worktree_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::task::{DependencyType, TaskSource, TaskStatus, ValidationRequirement};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    // Mock TaskQueueService for testing
    struct MockTaskQueue {
        tasks: Arc<StdMutex<HashMap<uuid::Uuid, Task>>>,
    }

    impl MockTaskQueue {
        fn new() -> Self {
            Self {
                tasks: Arc::new(StdMutex::new(HashMap::new())),
            }
        }

        #[allow(dead_code)]
        fn add_task(&self, task: Task) {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task.id, task);
        }
    }

    #[async_trait]
    impl TaskQueueServiceTrait for MockTaskQueue {
        async fn get_task(&self, task_id: uuid::Uuid) -> Result<Task> {
            let tasks = self.tasks.lock().unwrap();
            tasks
                .get(&task_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Task not found"))
        }

        async fn get_tasks_by_status(&self, _status: TaskStatus) -> Result<Vec<Task>> {
            Ok(vec![])
        }

        async fn get_dependent_tasks(&self, _task_id: uuid::Uuid) -> Result<Vec<Task>> {
            Ok(vec![])
        }

        async fn get_children_by_parent(&self, _parent_id: uuid::Uuid) -> Result<Vec<Task>> {
            Ok(vec![])
        }

        async fn update_task_status(&self, task_id: uuid::Uuid, status: TaskStatus) -> Result<()> {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.status = status;
                Ok(())
            } else {
                Err(anyhow::anyhow!("Task not found"))
            }
        }

        async fn update_task_priority(&self, _task_id: uuid::Uuid, _priority: f64) -> Result<()> {
            Ok(())
        }

        async fn update_task(&self, task: &Task) -> Result<()> {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task.id, task.clone());
            Ok(())
        }

        async fn mark_task_failed(&self, _task_id: uuid::Uuid, _error_message: String) -> Result<()> {
            Ok(())
        }

        async fn get_next_ready_task(&self) -> Result<Option<Task>> {
            Ok(None)
        }

        async fn claim_next_ready_task(&self) -> Result<Option<Task>> {
            Ok(None)
        }

        async fn submit_task(&self, task: Task) -> Result<uuid::Uuid> {
            let task_id = task.id;
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task_id, task);
            Ok(task_id)
        }

        async fn get_stale_running_tasks(&self, _stale_threshold_secs: u64) -> Result<Vec<Task>> {
            Ok(vec![])
        }

        async fn task_exists_by_idempotency_key(&self, _idempotency_key: &str) -> Result<bool> {
            Ok(false)
        }

        async fn get_task_by_idempotency_key(&self, _idempotency_key: &str) -> Result<Option<Task>> {
            Ok(None)
        }

        async fn submit_task_idempotent(&self, task: Task) -> Result<crate::domain::ports::task_repository::IdempotentInsertResult> {
            use crate::domain::ports::task_repository::IdempotentInsertResult;
            let task_id = task.id;
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task_id, task);
            Ok(IdempotentInsertResult::Inserted(task_id))
        }

        async fn submit_tasks_transactional(&self, tasks_to_insert: Vec<Task>) -> Result<crate::domain::ports::task_repository::BatchInsertResult> {
            use crate::domain::ports::task_repository::BatchInsertResult;
            let mut result = BatchInsertResult::new();
            let mut tasks = self.tasks.lock().unwrap();
            for task in tasks_to_insert {
                let task_id = task.id;
                tasks.insert(task_id, task);
                result.inserted.push(task_id);
            }
            Ok(result)
        }

        async fn resolve_dependencies_for_completed_task(&self, _completed_task_id: uuid::Uuid) -> Result<usize> {
            Ok(0)
        }

        async fn update_parent_and_insert_children_atomic(
            &self,
            parent_task: &Task,
            child_tasks: Vec<Task>,
        ) -> Result<crate::domain::ports::task_repository::DecompositionResult> {
            use crate::domain::ports::task_repository::DecompositionResult;
            let mut tasks = self.tasks.lock().unwrap();

            // Update parent
            tasks.insert(parent_task.id, parent_task.clone());

            // Insert children
            let mut children_inserted = Vec::new();
            for child in child_tasks {
                children_inserted.push(child.id);
                tasks.insert(child.id, child);
            }

            Ok(DecompositionResult {
                parent_id: parent_task.id,
                parent_new_version: parent_task.version + 1,
                children_inserted,
                children_already_existed: vec![],
            })
        }
    }

    fn create_test_task() -> Task {
        Task {
            id: uuid::Uuid::new_v4(),
            summary: "Test task".to_string(),
            description: "Test description".to_string(),
            agent_type: "test-agent".to_string(),
            priority: 5,
            calculated_priority: 5.0,
            status: TaskStatus::Ready,
            dependencies: None,
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
            branch: None,
            worktree_path: None,
            needs_worktree: None,
            validation_requirement: ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
            chain_step_index: 0,
            chain_handoff_state: None,
            idempotency_key: None,
            version: 1,
            awaiting_children: None,
            spawned_by_task_id: None,
        }
    }

    #[test]
    fn test_generate_branch_metadata_new() {
        let mock_queue = Arc::new(MockTaskQueue::new());
        let service = WorktreeService::new(mock_queue);

        let task = create_test_task();
        let (branch, worktree_path) = service.generate_branch_metadata(&task, "feature/my-feature");

        assert!(branch.starts_with("task/my-feature/"));
        assert!(branch.len() > "task/my-feature/".len());
        assert!(worktree_path.starts_with(".abathur/worktrees/task-"));
    }

    #[test]
    fn test_generate_branch_metadata_existing() {
        let mock_queue = Arc::new(MockTaskQueue::new());
        let service = WorktreeService::new(mock_queue);

        let mut task = create_test_task();
        task.branch = Some("existing-branch".to_string());
        task.worktree_path = Some("existing-path".to_string());

        let (branch, worktree_path) = service.generate_branch_metadata(&task, "feature/my-feature");

        assert_eq!(branch, "existing-branch");
        assert_eq!(worktree_path, "existing-path");
    }

    #[test]
    fn test_generate_branch_metadata_feature_prefix_variants() {
        let mock_queue = Arc::new(MockTaskQueue::new());
        let service = WorktreeService::new(mock_queue);
        let task = create_test_task();

        // Test feature/ prefix
        let (branch, _) = service.generate_branch_metadata(&task, "feature/test");
        assert!(branch.starts_with("task/test/"));

        // Test features/ prefix
        let (branch, _) = service.generate_branch_metadata(&task, "features/test");
        assert!(branch.starts_with("task/test/"));

        // Test no prefix
        let (branch, _) = service.generate_branch_metadata(&task, "main");
        assert!(branch.starts_with("task/main/"));
    }

    #[tokio::test]
    async fn test_setup_task_worktree_no_feature_branch() {
        let mock_queue = Arc::new(MockTaskQueue::new());
        let service = WorktreeService::new(mock_queue);

        let mut task = create_test_task();
        // No feature_branch set

        let result = service.setup_task_worktree(&mut task).await.unwrap();

        // Should return false since no feature_branch
        assert!(!result);
        assert!(task.branch.is_none());
        assert!(task.worktree_path.is_none());
    }

    #[tokio::test]
    async fn test_setup_task_worktree_needs_worktree_false() {
        let mock_queue = Arc::new(MockTaskQueue::new());
        let service = WorktreeService::new(mock_queue);

        let mut task = create_test_task();
        task.feature_branch = Some("feature/test".to_string());
        task.needs_worktree = Some(false); // Explicitly set to false

        let result = service.setup_task_worktree(&mut task).await.unwrap();

        // Should return false since needs_worktree is explicitly false
        // (e.g., for planning agents like technical-architect)
        assert!(!result);
        assert!(task.branch.is_none());
        assert!(task.worktree_path.is_none());
    }
}

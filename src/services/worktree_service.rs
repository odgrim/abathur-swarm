//! Worktree service implementing git worktree management.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Worktree, WorktreeStatus};
use crate::domain::ports::WorktreeRepository;

/// Configuration for worktree management.
#[derive(Debug, Clone)]
pub struct WorktreeConfig {
    /// Base directory for worktrees.
    pub base_path: PathBuf,
    /// Main repository path.
    pub repo_path: PathBuf,
    /// Default base ref for new branches.
    pub default_base_ref: String,
    /// Whether to auto-cleanup merged worktrees.
    pub auto_cleanup: bool,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            base_path: PathBuf::from(".abathur/worktrees"),
            repo_path: PathBuf::from("."),
            default_base_ref: "main".to_string(),
            auto_cleanup: true,
        }
    }
}

/// Stats about worktree status.
#[derive(Debug, Clone, Default)]
pub struct WorktreeStats {
    pub creating: u64,
    pub active: u64,
    pub completed: u64,
    pub merging: u64,
    pub merged: u64,
    pub failed: u64,
    pub removed: u64,
}

impl WorktreeStats {
    pub fn total(&self) -> u64 {
        self.creating + self.active + self.completed + self.merging + self.merged + self.failed + self.removed
    }

    pub fn active_count(&self) -> u64 {
        self.creating + self.active + self.completed + self.merging
    }
}

impl From<HashMap<WorktreeStatus, u64>> for WorktreeStats {
    fn from(map: HashMap<WorktreeStatus, u64>) -> Self {
        Self {
            creating: *map.get(&WorktreeStatus::Creating).unwrap_or(&0),
            active: *map.get(&WorktreeStatus::Active).unwrap_or(&0),
            completed: *map.get(&WorktreeStatus::Completed).unwrap_or(&0),
            merging: *map.get(&WorktreeStatus::Merging).unwrap_or(&0),
            merged: *map.get(&WorktreeStatus::Merged).unwrap_or(&0),
            failed: *map.get(&WorktreeStatus::Failed).unwrap_or(&0),
            removed: *map.get(&WorktreeStatus::Removed).unwrap_or(&0),
        }
    }
}

pub struct WorktreeService<W: WorktreeRepository> {
    repo: Arc<W>,
    config: WorktreeConfig,
}

impl<W: WorktreeRepository> WorktreeService<W> {
    pub fn new(repo: Arc<W>, config: WorktreeConfig) -> Self {
        Self { repo, config }
    }

    /// Create a new worktree for a task.
    pub async fn create_worktree(
        &self,
        task_id: Uuid,
        base_ref: Option<&str>,
    ) -> DomainResult<Worktree> {
        // Check if worktree already exists for this task
        if let Some(existing) = self.repo.get_by_task(task_id).await? {
            if !existing.status.is_terminal() {
                return Err(DomainError::ValidationFailed(
                    format!("Worktree already exists for task {}", task_id)
                ));
            }
        }

        let base = base_ref.unwrap_or(&self.config.default_base_ref);
        let branch = Worktree::branch_name_for_task(task_id);
        let path = Worktree::path_for_task(
            self.config.base_path.to_str().unwrap_or(".abathur/worktrees"),
            task_id,
        );

        // Create worktree record in creating state
        let mut worktree = Worktree::new(task_id, &path, &branch, base);
        self.repo.create(&worktree).await?;

        // Actually create the git worktree
        match self.git_create_worktree(&path, &branch, base).await {
            Ok(()) => {
                worktree.activate();
                self.repo.update(&worktree).await?;
                Ok(worktree)
            }
            Err(e) => {
                worktree.fail(e.to_string());
                self.repo.update(&worktree).await?;
                Err(e)
            }
        }
    }

    /// Get a worktree by ID.
    pub async fn get_worktree(&self, id: Uuid) -> DomainResult<Option<Worktree>> {
        self.repo.get(id).await
    }

    /// Get worktree for a task.
    pub async fn get_worktree_for_task(&self, task_id: Uuid) -> DomainResult<Option<Worktree>> {
        self.repo.get_by_task(task_id).await
    }

    /// Get worktree by path.
    pub async fn get_worktree_by_path(&self, path: &str) -> DomainResult<Option<Worktree>> {
        self.repo.get_by_path(path).await
    }

    /// Mark worktree as completed (work finished, ready for merge).
    pub async fn complete_worktree(&self, task_id: Uuid) -> DomainResult<Worktree> {
        let mut worktree = self.repo.get_by_task(task_id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("No worktree found for task {}", task_id)
            ))?;

        if worktree.status != WorktreeStatus::Active {
            return Err(DomainError::InvalidStateTransition {
                from: worktree.status.as_str().to_string(),
                to: "completed".to_string(),
            });
        }

        worktree.complete();
        self.repo.update(&worktree).await?;
        Ok(worktree)
    }

    /// Merge a completed worktree back to the base branch.
    pub async fn merge_worktree(&self, task_id: Uuid) -> DomainResult<Worktree> {
        let mut worktree = self.repo.get_by_task(task_id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("No worktree found for task {}", task_id)
            ))?;

        if worktree.status != WorktreeStatus::Completed {
            return Err(DomainError::InvalidStateTransition {
                from: worktree.status.as_str().to_string(),
                to: "merging".to_string(),
            });
        }

        worktree.start_merge();
        self.repo.update(&worktree).await?;

        match self.git_merge_branch(&worktree.branch, &worktree.base_ref).await {
            Ok(commit_sha) => {
                worktree.merged(commit_sha);
                self.repo.update(&worktree).await?;

                // Auto-cleanup if configured
                if self.config.auto_cleanup {
                    let _ = self.cleanup_worktree(worktree.id).await;
                }

                Ok(worktree)
            }
            Err(e) => {
                worktree.fail(e.to_string());
                self.repo.update(&worktree).await?;
                Err(e)
            }
        }
    }

    /// Cleanup a worktree (remove from filesystem and delete branch).
    pub async fn cleanup_worktree(&self, id: Uuid) -> DomainResult<()> {
        let mut worktree = self.repo.get(id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("Worktree {} not found", id)
            ))?;

        if !worktree.can_cleanup() {
            return Err(DomainError::ValidationFailed(
                format!("Worktree {} cannot be cleaned up in state {}", id, worktree.status.as_str())
            ));
        }

        // Remove git worktree
        if let Err(e) = self.git_remove_worktree(&worktree.path).await {
            tracing::warn!("Failed to remove git worktree: {}", e);
        }

        // Delete branch if merged
        if worktree.status == WorktreeStatus::Merged {
            if let Err(e) = self.git_delete_branch(&worktree.branch).await {
                tracing::warn!("Failed to delete branch: {}", e);
            }
        }

        worktree.remove();
        self.repo.update(&worktree).await?;
        Ok(())
    }

    /// Cleanup all eligible worktrees.
    pub async fn cleanup_all(&self) -> DomainResult<u64> {
        let worktrees = self.repo.list_for_cleanup().await?;
        let mut cleaned = 0u64;

        for wt in worktrees {
            if let Ok(()) = self.cleanup_worktree(wt.id).await {
                cleaned += 1;
            }
        }

        Ok(cleaned)
    }

    /// List active worktrees.
    pub async fn list_active(&self) -> DomainResult<Vec<Worktree>> {
        self.repo.list_active().await
    }

    /// List worktrees by status.
    pub async fn list_by_status(&self, status: WorktreeStatus) -> DomainResult<Vec<Worktree>> {
        self.repo.list_by_status(status).await
    }

    /// Get worktree statistics.
    pub async fn get_stats(&self) -> DomainResult<WorktreeStats> {
        let counts = self.repo.count_by_status().await?;
        Ok(WorktreeStats::from(counts))
    }

    /// Mark a worktree as failed.
    pub async fn fail_worktree(&self, task_id: Uuid, error: &str) -> DomainResult<Worktree> {
        let mut worktree = self.repo.get_by_task(task_id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("No worktree found for task {}", task_id)
            ))?;

        worktree.fail(error);
        self.repo.update(&worktree).await?;
        Ok(worktree)
    }

    // Git operations

    async fn git_create_worktree(&self, path: &str, branch: &str, base_ref: &str) -> DomainResult<()> {
        // Ensure base directory exists
        let full_path = self.config.repo_path.join(path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to create directory: {}", e)))?;
        }

        // Create worktree with new branch
        let output = Command::new("git")
            .args(["worktree", "add", "-b", branch, path, base_ref])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DomainError::ValidationFailed(format!("Git worktree add failed: {}", stderr)));
        }

        Ok(())
    }

    async fn git_remove_worktree(&self, path: &str) -> DomainResult<()> {
        let output = Command::new("git")
            .args(["worktree", "remove", "--force", path])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DomainError::ValidationFailed(format!("Git worktree remove failed: {}", stderr)));
        }

        Ok(())
    }

    async fn git_merge_branch(&self, branch: &str, target: &str) -> DomainResult<String> {
        // Checkout target branch in main repo
        let checkout = Command::new("git")
            .args(["checkout", target])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            return Err(DomainError::ValidationFailed(format!("Git checkout failed: {}", stderr)));
        }

        // Merge the branch
        let merge = Command::new("git")
            .args(["merge", "--no-ff", branch, "-m", &format!("Merge {} into {}", branch, target)])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !merge.status.success() {
            let stderr = String::from_utf8_lossy(&merge.stderr);
            return Err(DomainError::ValidationFailed(format!("Git merge failed: {}", stderr)));
        }

        // Get the merge commit SHA
        let rev_parse = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        let commit_sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_string();
        Ok(commit_sha)
    }

    async fn git_delete_branch(&self, branch: &str) -> DomainResult<()> {
        let output = Command::new("git")
            .args(["branch", "-d", branch])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DomainError::ValidationFailed(format!("Git branch delete failed: {}", stderr)));
        }

        Ok(())
    }

    /// Check if a path is a valid git worktree.
    pub async fn is_valid_worktree(&self, path: &str) -> bool {
        let full_path = self.config.repo_path.join(path);
        Path::new(&full_path).join(".git").exists()
    }

    /// Sync database with actual git worktrees on disk.
    pub async fn sync_with_filesystem(&self) -> DomainResult<(u64, u64)> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DomainError::ValidationFailed(format!("Git worktree list failed: {}", stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut fs_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

        for line in stdout.lines() {
            if line.starts_with("worktree ") {
                let path = line.strip_prefix("worktree ").unwrap_or("");
                fs_paths.insert(path.to_string());
            }
        }

        // Check DB records against filesystem
        let active = self.repo.list_active().await?;
        let mut marked_removed = 0u64;
        let marked_active = 0u64;

        for mut wt in active {
            let full_path = self.config.repo_path.join(&wt.path).to_string_lossy().to_string();
            if !fs_paths.contains(&full_path) && !fs_paths.contains(&wt.path) {
                // Worktree in DB but not on filesystem
                wt.remove();
                self.repo.update(&wt).await?;
                marked_removed += 1;
            }
        }

        Ok((marked_active, marked_removed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, SqliteWorktreeRepository, Migrator, all_embedded_migrations};

    async fn setup_service() -> WorktreeService<SqliteWorktreeRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();

        let repo = Arc::new(SqliteWorktreeRepository::new(pool));
        let config = WorktreeConfig::default();
        WorktreeService::new(repo, config)
    }

    #[tokio::test]
    async fn test_stats() {
        let service = setup_service().await;
        let stats = service.get_stats().await.unwrap();
        assert_eq!(stats.total(), 0);
    }
}

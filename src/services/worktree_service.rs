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
    /// Whether to fetch from remote before creating worktrees.
    /// Default: true. Set to false for local-only / offline development.
    pub fetch_on_sync: bool,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            base_path: PathBuf::from(".abathur/worktrees"),
            repo_path: PathBuf::from("."),
            default_base_ref: "main".to_string(),
            auto_cleanup: true,
            fetch_on_sync: true,
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
    last_fetch: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}

impl<W: WorktreeRepository> WorktreeService<W> {
    pub fn new(repo: Arc<W>, config: WorktreeConfig) -> Self {
        Self {
            repo,
            config,
            last_fetch: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Create a new worktree for a task.
    pub async fn create_worktree(
        &self,
        task_id: Uuid,
        base_ref: Option<&str>,
    ) -> DomainResult<Worktree> {
        // Check if worktree already exists for this task
        if let Some(existing) = self.repo.get_by_task(task_id).await?
            && !existing.status.is_terminal() {
                return Err(DomainError::ValidationFailed(
                    format!("Worktree already exists for task {}", task_id)
                ));
            }

        let base = base_ref.unwrap_or(&self.config.default_base_ref);
        let branch = Worktree::branch_name_for_task(task_id);
        let path = Worktree::path_for_task(
            self.config.base_path.to_str().unwrap_or(".abathur/worktrees"),
            task_id,
        );

        // Validate the generated path is under the configured base_path to
        // prevent path traversal (defense-in-depth: path_for_task should
        // always produce safe paths, but we verify anyway).
        self.validate_worktree_path(&path)?;

        // Fetch latest base ref so worktree branches from current remote state.
        // Debounce: skip if we fetched within the last 10 seconds (batch creation).
        let fetch_succeeded = if self.config.fetch_on_sync {
            let should_fetch = {
                let mut last = self.last_fetch.lock().await;
                match *last {
                    Some(t) if t.elapsed().as_secs() < 10 => false,
                    _ => { *last = Some(std::time::Instant::now()); true }
                }
            };
            if should_fetch {
                match Command::new("git")
                    .args(["fetch", "origin", base])
                    .current_dir(&self.config.repo_path)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                {
                    Ok(o) if o.status.success() => {
                        tracing::debug!(base_ref = base, "fetched latest remote state before worktree creation");
                        true
                    }
                    Ok(o) => {
                        tracing::warn!(
                            base_ref = base,
                            stderr = %String::from_utf8_lossy(&o.stderr),
                            "fetch before worktree creation failed — using local ref"
                        );
                        false
                    }
                    Err(e) => {
                        tracing::warn!(base_ref = base, error = %e, "fetch command failed — using local ref");
                        false
                    }
                }
            } else {
                // Debounced — assume recent fetch is still valid
                true
            }
        } else {
            false
        };

        // Use origin/<base> when fetch succeeded so the worktree starts from the
        // latest remote state rather than the (potentially stale) local branch.
        let effective_base = if fetch_succeeded {
            format!("origin/{}", base)
        } else {
            base.to_string()
        };

        // Create worktree record in creating state
        let mut worktree = Worktree::new(task_id, &path, &branch, base);
        self.repo.create(&worktree).await?;

        // Actually create the git worktree
        match self.git_create_worktree(&path, &branch, &effective_base).await {
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
                reason: "worktree must be in Active state to be completed".to_string(),
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
                reason: "worktree must be in Completed state to merge".to_string(),
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
        if worktree.status == WorktreeStatus::Merged
            && let Err(e) = self.git_delete_branch(&worktree.branch).await {
                tracing::warn!("Failed to delete branch: {}", e);
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

    /// Validate that a worktree path is under the configured base_path.
    ///
    /// Prevents path traversal by ensuring the worktree directory will be
    /// created inside the expected base directory. Uses canonicalization
    /// when possible, with a textual fallback for not-yet-created paths.
    fn validate_worktree_path(&self, path: &str) -> DomainResult<()> {
        let full_path = self.config.repo_path.join(path);
        let base_path = self.config.repo_path.join(&self.config.base_path);

        // Reject absolute paths that clearly lie outside the base directory
        if Path::new(path).is_absolute() && !full_path.starts_with(&base_path) {
            return Err(DomainError::ValidationFailed(format!(
                "Path traversal detected: worktree path '{}' is an absolute path outside base '{}'",
                path,
                self.config.base_path.display()
            )));
        }

        // Attempt to canonicalize the base path
        let canonical_base = match base_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // Base path doesn't exist yet — fall back to textual check
                let normalized = full_path.to_string_lossy();
                if normalized.contains("..") {
                    return Err(DomainError::ValidationFailed(format!(
                        "Path traversal detected: worktree path '{}' contains '..' components",
                        path
                    )));
                }
                return Ok(());
            }
        };

        // Try to canonicalize the full path
        match full_path.canonicalize() {
            Ok(canonical) => {
                if !canonical.starts_with(&canonical_base) {
                    return Err(DomainError::ValidationFailed(format!(
                        "Path traversal detected: worktree path '{}' resolves outside base '{}'",
                        path,
                        self.config.base_path.display()
                    )));
                }
            }
            Err(_) => {
                // Path doesn't exist yet — check the parent
                if let Some(parent) = full_path.parent()
                    && let Ok(canonical_parent) = parent.canonicalize()
                {
                    let file_name = full_path.file_name().unwrap_or_default();
                    let reconstructed = canonical_parent.join(file_name);
                    if !reconstructed.starts_with(&canonical_base) {
                        return Err(DomainError::ValidationFailed(format!(
                            "Path traversal detected: worktree path '{}' would resolve outside base '{}'",
                            path,
                            self.config.base_path.display()
                        )));
                    }
                }

                // Textual fallback
                let normalized = full_path.to_string_lossy();
                if normalized.contains("..") {
                    return Err(DomainError::ValidationFailed(format!(
                        "Path traversal detected: worktree path '{}' contains '..' components",
                        path
                    )));
                }
            }
        }

        Ok(())
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
        // Use git plumbing commands to merge without touching the working tree.
        // This avoids checkout + merge in the main worktree which can leave
        // uncommitted changes on failure or race conditions.

        // merge-tree --write-tree creates the merged tree without modifying any worktree
        let merge_tree = Command::new("git")
            .args(["merge-tree", "--write-tree", target, branch])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("git merge-tree failed to run: {}", e)))?;

        if !merge_tree.status.success() {
            let stdout = String::from_utf8_lossy(&merge_tree.stdout);
            let stderr = String::from_utf8_lossy(&merge_tree.stderr);
            return Err(DomainError::ValidationFailed(
                format!("Git merge failed (conflicts or error): {}{}", stdout.trim(), stderr.trim())
            ));
        }

        let tree_sha = String::from_utf8_lossy(&merge_tree.stdout)
            .lines().next().unwrap_or("").trim().to_string();

        // Resolve parent commit SHAs
        let target_sha = {
            let out = Command::new("git")
                .args(["rev-parse", target])
                .current_dir(&self.config.repo_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to rev-parse {}: {}", target, e)))?;
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        let source_sha = {
            let out = Command::new("git")
                .args(["rev-parse", branch])
                .current_dir(&self.config.repo_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to rev-parse {}: {}", branch, e)))?;
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        // Create merge commit from the tree
        let merge_msg = format!("Merge {} into {}", branch, target);
        let commit = Command::new("git")
            .args(["commit-tree", &tree_sha, "-p", &target_sha, "-p", &source_sha, "-m", &merge_msg])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("git commit-tree failed: {}", e)))?;

        if !commit.status.success() {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            return Err(DomainError::ValidationFailed(format!("git commit-tree: {}", stderr)));
        }

        let commit_sha = String::from_utf8_lossy(&commit.stdout).trim().to_string();

        // Atomically update the branch ref (CAS prevents races)
        let update = Command::new("git")
            .args(["update-ref", &format!("refs/heads/{}", target), &commit_sha, &target_sha])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("git update-ref failed: {}", e)))?;

        if !update.status.success() {
            let stderr = String::from_utf8_lossy(&update.stderr);
            return Err(DomainError::ValidationFailed(format!("git update-ref: {}", stderr)));
        }

        // Sync main worktree to match the updated branch tip
        let _ = Command::new("git")
            .args(["reset", "--hard", "HEAD"])
            .current_dir(&self.config.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

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
    use crate::adapters::sqlite::{create_migrated_test_pool, SqliteWorktreeRepository};

    async fn setup_service() -> WorktreeService<SqliteWorktreeRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
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

    #[test]
    fn test_validate_worktree_path_rejects_traversal() {
        let config = WorktreeConfig {
            base_path: PathBuf::from(".abathur/worktrees"),
            repo_path: std::env::current_dir().unwrap(),
            ..Default::default()
        };
        let repo = Arc::new(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async {
                    let pool = crate::adapters::sqlite::create_migrated_test_pool().await.unwrap();
                    crate::adapters::sqlite::SqliteWorktreeRepository::new(pool)
                }),
        );
        let service = WorktreeService::new(repo, config);

        // Path with ".." should be rejected
        let result = service.validate_worktree_path("../../etc/passwd");
        assert!(result.is_err(), "Path traversal with '..' should be rejected");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Path traversal"), "Error should mention path traversal: {err_msg}");

        // Absolute path outside base should be rejected
        let result = service.validate_worktree_path("/tmp/evil");
        assert!(result.is_err(), "Absolute path outside base should be rejected");

        // Valid relative path under base should be accepted
        let result = service.validate_worktree_path(".abathur/worktrees/task-abc123");
        assert!(result.is_ok(), "Valid path under base should be accepted: {:?}", result.err());
    }
}

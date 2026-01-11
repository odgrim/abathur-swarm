//! Worktree domain model.
//!
//! Git worktrees provide isolated workspaces for tasks,
//! enabling parallel execution without conflicts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeStatus {
    /// Being created
    Creating,
    /// Active and in use
    Active,
    /// Work completed, ready for merge
    Completed,
    /// Being merged
    Merging,
    /// Successfully merged
    Merged,
    /// Failed, needs cleanup
    Failed,
    /// Cleaned up and removed
    Removed,
}

impl Default for WorktreeStatus {
    fn default() -> Self {
        Self::Creating
    }
}

impl WorktreeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Merging => "merging",
            Self::Merged => "merged",
            Self::Failed => "failed",
            Self::Removed => "removed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "creating" => Some(Self::Creating),
            "active" => Some(Self::Active),
            "completed" => Some(Self::Completed),
            "merging" => Some(Self::Merging),
            "merged" => Some(Self::Merged),
            "failed" => Some(Self::Failed),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Merged | Self::Failed | Self::Removed)
    }
}

/// A git worktree for task isolation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    /// Unique identifier
    pub id: Uuid,
    /// Associated task ID
    pub task_id: Uuid,
    /// Worktree filesystem path
    pub path: String,
    /// Branch name
    pub branch: String,
    /// Base ref (where branch was created from)
    pub base_ref: String,
    /// Current status
    pub status: WorktreeStatus,
    /// Merge commit SHA (if merged)
    pub merge_commit: Option<String>,
    /// Error message (if failed)
    pub error_message: Option<String>,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When last updated
    pub updated_at: DateTime<Utc>,
    /// When completed (if applicable)
    pub completed_at: Option<DateTime<Utc>>,
}

impl Worktree {
    /// Create a new worktree.
    pub fn new(task_id: Uuid, path: impl Into<String>, branch: impl Into<String>, base_ref: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            task_id,
            path: path.into(),
            branch: branch.into(),
            base_ref: base_ref.into(),
            status: WorktreeStatus::Creating,
            merge_commit: None,
            error_message: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    /// Generate a branch name from task ID.
    pub fn branch_name_for_task(task_id: Uuid) -> String {
        format!("abathur/task-{}", &task_id.to_string()[..8])
    }

    /// Generate a worktree path from task ID.
    pub fn path_for_task(base_path: &str, task_id: Uuid) -> String {
        format!("{}/task-{}", base_path, &task_id.to_string()[..8])
    }

    /// Mark as active (creation complete).
    pub fn activate(&mut self) {
        self.status = WorktreeStatus::Active;
        self.updated_at = Utc::now();
    }

    /// Mark as completed (work done, ready for merge).
    pub fn complete(&mut self) {
        self.status = WorktreeStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Start merging.
    pub fn start_merge(&mut self) {
        self.status = WorktreeStatus::Merging;
        self.updated_at = Utc::now();
    }

    /// Mark as merged successfully.
    pub fn merged(&mut self, commit: impl Into<String>) {
        self.status = WorktreeStatus::Merged;
        self.merge_commit = Some(commit.into());
        self.updated_at = Utc::now();
    }

    /// Mark as failed.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.status = WorktreeStatus::Failed;
        self.error_message = Some(error.into());
        self.updated_at = Utc::now();
    }

    /// Mark as removed.
    pub fn remove(&mut self) {
        self.status = WorktreeStatus::Removed;
        self.updated_at = Utc::now();
    }

    /// Check if worktree can be cleaned up.
    pub fn can_cleanup(&self) -> bool {
        matches!(self.status, WorktreeStatus::Merged | WorktreeStatus::Failed | WorktreeStatus::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_creation() {
        let task_id = Uuid::new_v4();
        let wt = Worktree::new(
            task_id,
            "/tmp/worktrees/task-123",
            "abathur/task-123",
            "main",
        );

        assert_eq!(wt.task_id, task_id);
        assert_eq!(wt.status, WorktreeStatus::Creating);
    }

    #[test]
    fn test_worktree_lifecycle() {
        let task_id = Uuid::new_v4();
        let mut wt = Worktree::new(task_id, "/path", "branch", "main");

        wt.activate();
        assert_eq!(wt.status, WorktreeStatus::Active);

        wt.complete();
        assert_eq!(wt.status, WorktreeStatus::Completed);
        assert!(wt.completed_at.is_some());

        wt.start_merge();
        assert_eq!(wt.status, WorktreeStatus::Merging);

        wt.merged("abc123");
        assert_eq!(wt.status, WorktreeStatus::Merged);
        assert_eq!(wt.merge_commit, Some("abc123".to_string()));
    }

    #[test]
    fn test_branch_name_generation() {
        let task_id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let branch = Worktree::branch_name_for_task(task_id);
        assert!(branch.starts_with("abathur/task-"));
    }
}

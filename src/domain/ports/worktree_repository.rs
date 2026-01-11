//! Worktree repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Worktree, WorktreeStatus};

/// Repository interface for Worktree persistence.
#[async_trait]
pub trait WorktreeRepository: Send + Sync {
    /// Create a new worktree record.
    async fn create(&self, worktree: &Worktree) -> DomainResult<()>;

    /// Get a worktree by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<Worktree>>;

    /// Get worktree by task ID.
    async fn get_by_task(&self, task_id: Uuid) -> DomainResult<Option<Worktree>>;

    /// Get worktree by path.
    async fn get_by_path(&self, path: &str) -> DomainResult<Option<Worktree>>;

    /// Update a worktree.
    async fn update(&self, worktree: &Worktree) -> DomainResult<()>;

    /// Delete a worktree record.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// List worktrees by status.
    async fn list_by_status(&self, status: WorktreeStatus) -> DomainResult<Vec<Worktree>>;

    /// List active worktrees.
    async fn list_active(&self) -> DomainResult<Vec<Worktree>>;

    /// List worktrees ready for cleanup.
    async fn list_for_cleanup(&self) -> DomainResult<Vec<Worktree>>;

    /// Count worktrees by status.
    async fn count_by_status(&self) -> DomainResult<std::collections::HashMap<WorktreeStatus, u64>>;
}

//! Merge request repository port.
//!
//! Persists merge queue state so that conflict detection, specialist spawning,
//! and retry-after-resolution survive across ephemeral MergeQueue instances
//! and process restarts.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::services::merge_queue::{MergeRequest, MergeStatus};

/// Repository interface for MergeRequest persistence.
#[async_trait]
pub trait MergeRequestRepository: Send + Sync {
    /// Insert a new merge request.
    async fn create(&self, request: &MergeRequest) -> DomainResult<()>;

    /// Get a merge request by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<MergeRequest>>;

    /// Update a merge request (status, error, commit_sha, conflict_files, attempts, etc.).
    async fn update(&self, request: &MergeRequest) -> DomainResult<()>;

    /// List merge requests by status.
    async fn list_by_status(&self, status: MergeStatus) -> DomainResult<Vec<MergeRequest>>;

    /// List merge requests by task ID.
    async fn list_by_task(&self, task_id: Uuid) -> DomainResult<Vec<MergeRequest>>;

    /// Get all conflict records needing resolution (status=Conflict, attempts < max).
    async fn list_unresolved_conflicts(&self, max_attempts: u32) -> DomainResult<Vec<MergeRequest>>;

    /// Delete merge requests with terminal status older than the given duration.
    async fn prune_terminal(&self, older_than: chrono::Duration) -> DomainResult<u64>;
}

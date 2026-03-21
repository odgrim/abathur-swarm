//! Two-Stage Merge Queue Service.
//!
//! Implements a two-stage merge process:
//! - Stage 1: Agent worktree branches → Task feature branch
//! - Stage 2: Task feature branch → Main branch (with verification)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{TaskStatus, WorktreeStatus};
use crate::domain::ports::{GoalRepository, MergeRequestRepository, TaskRepository, WorktreeRepository};
use crate::services::integration_verifier::{IntegrationVerifierService, VerificationResult};

/// Stage of the merge process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStage {
    /// Stage 1: Merging agent work into task branch.
    AgentToTask,
    /// Stage 2: Merging task branch into main.
    TaskToMain,
}

impl MergeStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentToTask => "AgentToTask",
            Self::TaskToMain => "TaskToMain",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "AgentToTask" => Some(Self::AgentToTask),
            "TaskToMain" => Some(Self::TaskToMain),
            _ => None,
        }
    }
}

/// Status of a merge request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    /// Queued for merge.
    Queued,
    /// Currently being processed.
    InProgress,
    /// Merge completed successfully.
    Completed,
    /// Merge failed.
    Failed,
    /// Blocked by conflicts.
    Conflict,
    /// Verification failed (Stage 2 only).
    VerificationFailed,
}

impl MergeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::InProgress => "InProgress",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Conflict => "Conflict",
            Self::VerificationFailed => "VerificationFailed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Queued" => Some(Self::Queued),
            "InProgress" => Some(Self::InProgress),
            "Completed" => Some(Self::Completed),
            "Failed" => Some(Self::Failed),
            "Conflict" => Some(Self::Conflict),
            "VerificationFailed" => Some(Self::VerificationFailed),
            _ => None,
        }
    }

    /// Whether this status is terminal (no further transitions expected).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::VerificationFailed)
    }
}

/// A merge request in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRequest {
    /// Unique ID for this merge request.
    pub id: Uuid,
    /// Stage of the merge.
    pub stage: MergeStage,
    /// Task ID associated with this merge.
    pub task_id: Uuid,
    /// Source branch to merge from.
    pub source_branch: String,
    /// Target branch to merge into.
    pub target_branch: String,
    /// Working directory for git operations.
    pub workdir: String,
    /// Current status.
    pub status: MergeStatus,
    /// Error message if failed.
    pub error: Option<String>,
    /// Merge commit SHA if successful.
    pub commit_sha: Option<String>,
    /// Verification result (Stage 2 only).
    pub verification: Option<VerificationResult>,
    /// Files with merge conflicts (populated when status is Conflict).
    #[serde(default)]
    pub conflict_files: Vec<String>,
    /// Number of resolution attempts so far.
    #[serde(default)]
    pub attempts: u32,
    /// When the request was created.
    pub created_at: DateTime<Utc>,
    /// When the request was last updated.
    pub updated_at: DateTime<Utc>,
}

impl MergeRequest {
    pub fn new_stage1(task_id: Uuid, source_branch: String, target_branch: String, workdir: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            stage: MergeStage::AgentToTask,
            task_id,
            source_branch,
            target_branch,
            workdir,
            status: MergeStatus::Queued,
            error: None,
            commit_sha: None,
            verification: None,
            conflict_files: Vec::new(),
            attempts: 0,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn new_stage2(task_id: Uuid, source_branch: String, target_branch: String, workdir: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            stage: MergeStage::TaskToMain,
            task_id,
            source_branch,
            target_branch,
            workdir,
            status: MergeStatus::Queued,
            error: None,
            commit_sha: None,
            verification: None,
            conflict_files: Vec::new(),
            attempts: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Result of a merge operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// The merge request ID.
    pub request_id: Uuid,
    /// Whether the merge succeeded.
    pub success: bool,
    /// Commit SHA if successful.
    pub commit_sha: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Whether there were conflicts.
    pub had_conflicts: bool,
    /// Conflicting files if any.
    pub conflict_files: Vec<String>,
}

/// Configuration for the merge queue.
#[derive(Debug, Clone)]
pub struct MergeQueueConfig {
    /// Path to the main repository.
    pub repo_path: String,
    /// Target branch for Stage 2 merges (typically "main").
    pub main_branch: String,
    /// Whether to require verification before Stage 2 merge.
    pub require_verification: bool,
    /// Whether to auto-retry on transient failures.
    pub auto_retry: bool,
    /// Maximum retry attempts.
    pub max_retries: u32,
    /// Whether to route conflicts to specialist agents.
    pub route_conflicts_to_specialist: bool,
}

impl Default for MergeQueueConfig {
    fn default() -> Self {
        Self {
            repo_path: ".".to_string(),
            main_branch: "main".to_string(),
            require_verification: true,
            auto_retry: true,
            max_retries: 3,
            route_conflicts_to_specialist: true,
        }
    }
}

/// Information about a merge conflict that needs specialist resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionRequest {
    /// The merge request that has conflicts.
    pub merge_request_id: Uuid,
    /// Associated task ID.
    pub task_id: Uuid,
    /// Source branch.
    pub source_branch: String,
    /// Target branch.
    pub target_branch: String,
    /// Working directory.
    pub workdir: String,
    /// Files with conflicts.
    pub conflict_files: Vec<String>,
    /// When the conflict was detected.
    pub detected_at: DateTime<Utc>,
    /// Number of resolution attempts so far.
    pub attempts: u32,
}

/// Stats about the merge queue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeQueueStats {
    pub queued: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub conflicts: usize,
    pub stage1_completed: usize,
    pub stage2_completed: usize,
}

/// Two-Stage Merge Queue Service.
///
/// Persists merge requests to a `MergeRequestRepository` so that conflict
/// records survive across ephemeral instances and process restarts.
pub struct MergeQueue<T, G, W>
where
    T: TaskRepository + 'static,
    G: GoalRepository + 'static,
    W: WorktreeRepository + 'static,
{
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
    verifier: Arc<IntegrationVerifierService<T, G, W>>,
    config: MergeQueueConfig,
    merge_repo: Arc<dyn MergeRequestRepository>,
}

impl<T, G, W> MergeQueue<T, G, W>
where
    T: TaskRepository + 'static,
    G: GoalRepository + 'static,
    W: WorktreeRepository + 'static,
{
    pub fn new(
        task_repo: Arc<T>,
        worktree_repo: Arc<W>,
        verifier: Arc<IntegrationVerifierService<T, G, W>>,
        config: MergeQueueConfig,
        merge_repo: Arc<dyn MergeRequestRepository>,
    ) -> Self {
        Self {
            task_repo,
            worktree_repo,
            verifier,
            config,
            merge_repo,
        }
    }

    /// Queue a Stage 1 merge (agent → task branch).
    pub async fn queue_stage1(
        &self,
        task_id: Uuid,
        agent_branch: &str,
        task_branch: &str,
    ) -> DomainResult<Uuid> {
        tracing::info!(%task_id, source_branch = %agent_branch, target_branch = %task_branch, "queueing stage 1 merge");
        // Get worktree for this task
        let worktree = self.worktree_repo.get_by_task(task_id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("No worktree found for task {}", task_id)
            ))?;

        let request = MergeRequest::new_stage1(
            task_id,
            agent_branch.to_string(),
            task_branch.to_string(),
            worktree.path.clone(),
        );

        let id = request.id;
        tracing::info!(%task_id, merge_request_id = %id, "stage 1 merge request queued");
        self.merge_repo.create(&request).await?;
        Ok(id)
    }

    /// Queue a merge-back: subtask branch → feature branch, in the feature branch's worktree.
    pub async fn queue_merge_back(
        &self,
        task_id: Uuid,
        source_branch: &str,
        target_branch: &str,
        target_workdir: &str,
    ) -> DomainResult<Uuid> {
        tracing::info!(%task_id, %source_branch, %target_branch, "queueing merge-back: subtask → feature branch");
        let request = MergeRequest::new_stage1(
            task_id,
            source_branch.to_string(),
            target_branch.to_string(),
            target_workdir.to_string(),
        );
        let id = request.id;
        tracing::info!(%task_id, merge_request_id = %id, "merge-back request queued");
        self.merge_repo.create(&request).await?;
        Ok(id)
    }

    /// Queue a Stage 2 merge (task → main).
    pub async fn queue_stage2(&self, task_id: Uuid) -> DomainResult<Uuid> {
        tracing::info!(%task_id, "queueing stage 2 merge: task → main");
        // Get worktree for this task
        let worktree = self.worktree_repo.get_by_task(task_id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("No worktree found for task {}", task_id)
            ))?;

        let request = MergeRequest::new_stage2(
            task_id,
            worktree.branch.clone(),
            self.config.main_branch.clone(),
            worktree.path.clone(),
        );

        let id = request.id;
        tracing::info!(%task_id, merge_request_id = %id, "stage 2 merge request queued");
        self.merge_repo.create(&request).await?;
        Ok(id)
    }

    /// Process the next merge in the queue.
    #[instrument(skip(self), fields(stage))]
    pub async fn process_next(&self) -> DomainResult<Option<MergeResult>> {
        // Get next queued request from persistent storage
        let queued = self.merge_repo.list_by_status(MergeStatus::Queued).await?;
        let mut request = match queued.into_iter().next() {
            Some(req) => req,
            None => {
                tracing::debug!("no queued merge requests to process");
                return Ok(None);
            }
        };

        // Skip if another merge to the same target branch is already in progress
        let in_progress = self.merge_repo.list_by_status(MergeStatus::InProgress).await?;
        if in_progress.iter().any(|r| r.target_branch == request.target_branch) {
            tracing::debug!(
                target_branch = %request.target_branch,
                "skipping merge — another merge to same target is in progress"
            );
            return Ok(None);
        }

        tracing::info!(merge_request_id = %request.id, stage = ?request.stage, task_id = %request.task_id, "processing merge request");
        request.status = MergeStatus::InProgress;
        request.updated_at = Utc::now();
        self.merge_repo.update(&request).await?;

        // Process based on stage
        let result = match request.stage {
            MergeStage::AgentToTask => self.process_stage1(&mut request).await,
            MergeStage::TaskToMain => self.process_stage2(&mut request).await,
        };

        // Persist final state
        self.merge_repo.update(&request).await?;

        result.map(Some)
    }

    /// Process a Stage 1 merge (agent → task branch).
    #[instrument(skip(self, request), fields(task_id = %request.task_id, source = %request.source_branch, target = %request.target_branch))]
    async fn process_stage1(&self, request: &mut MergeRequest) -> DomainResult<MergeResult> {
        // Verify workdir still exists
        if !std::path::Path::new(&request.workdir).exists() {
            tracing::error!(task_id = %request.task_id, workdir = %request.workdir, "workdir does not exist");
            request.status = MergeStatus::Failed;
            request.error = Some(format!("Working directory no longer exists: {}", request.workdir));
            request.updated_at = Utc::now();
            return Ok(MergeResult {
                request_id: request.id,
                success: false,
                commit_sha: None,
                error: request.error.clone(),
                had_conflicts: false,
                conflict_files: vec![],
            });
        }

        // Check for conflicts first
        let conflict_check = self.check_merge_conflicts(
            &request.workdir,
            &request.source_branch,
            &request.target_branch,
        ).await?;

        if !conflict_check.0.is_empty() {
            tracing::warn!(task_id = %request.task_id, conflict_count = conflict_check.0.len(), "stage 1 merge: conflicts detected");
            request.status = MergeStatus::Conflict;
            request.conflict_files = conflict_check.0.clone();
            request.error = Some(format!("Merge conflicts in: {}", conflict_check.0.join(", ")));
            request.updated_at = Utc::now();

            return Ok(MergeResult {
                request_id: request.id,
                success: false,
                commit_sha: None,
                error: request.error.clone(),
                had_conflicts: true,
                conflict_files: conflict_check.0,
            });
        }

        // Perform the merge
        match self.git_merge(&request.workdir, &request.source_branch, &request.target_branch).await {
            Ok(commit_sha) => {
                tracing::info!(task_id = %request.task_id, %commit_sha, "stage 1 merge completed successfully");
                request.status = MergeStatus::Completed;
                request.commit_sha = Some(commit_sha.clone());
                request.updated_at = Utc::now();

                Ok(MergeResult {
                    request_id: request.id,
                    success: true,
                    commit_sha: Some(commit_sha),
                    error: None,
                    had_conflicts: false,
                    conflict_files: vec![],
                })
            }
            Err(e) => {
                tracing::error!(task_id = %request.task_id, error = %e, "stage 1 merge failed");
                request.status = MergeStatus::Failed;
                request.error = Some(e.to_string());
                request.updated_at = Utc::now();

                Ok(MergeResult {
                    request_id: request.id,
                    success: false,
                    commit_sha: None,
                    error: request.error.clone(),
                    had_conflicts: false,
                    conflict_files: vec![],
                })
            }
        }
    }

    /// Process a Stage 2 merge (task → main with verification).
    #[instrument(skip(self, request), fields(task_id = %request.task_id, source = %request.source_branch, target = %request.target_branch))]
    async fn process_stage2(&self, request: &mut MergeRequest) -> DomainResult<MergeResult> {
        // Run verification first if required
        if self.config.require_verification {
            let verification = self.verifier.verify_task(request.task_id).await?;
            request.verification = Some(verification.clone());

            if !verification.passed {
                tracing::warn!(task_id = %request.task_id, "stage 2 merge blocked: verification failed");
                request.status = MergeStatus::VerificationFailed;
                request.error = verification.failures_summary.clone();
                request.updated_at = Utc::now();

                return Ok(MergeResult {
                    request_id: request.id,
                    success: false,
                    commit_sha: None,
                    error: request.error.clone(),
                    had_conflicts: false,
                    conflict_files: vec![],
                });
            }
        }

        // Check for conflicts
        let conflict_check = self.check_merge_conflicts(
            &self.config.repo_path,
            &request.source_branch,
            &request.target_branch,
        ).await?;

        if !conflict_check.0.is_empty() {
            tracing::warn!(task_id = %request.task_id, conflict_count = conflict_check.0.len(), "stage 2 merge: conflicts detected");
            request.status = MergeStatus::Conflict;
            request.conflict_files = conflict_check.0.clone();
            request.error = Some(format!("Merge conflicts in: {}", conflict_check.0.join(", ")));
            request.updated_at = Utc::now();

            return Ok(MergeResult {
                request_id: request.id,
                success: false,
                commit_sha: None,
                error: request.error.clone(),
                had_conflicts: true,
                conflict_files: conflict_check.0,
            });
        }

        // Perform the merge to main
        match self.git_merge(&self.config.repo_path, &request.source_branch, &request.target_branch).await {
            Ok(commit_sha) => {
                tracing::info!(task_id = %request.task_id, %commit_sha, "stage 2 merge to main completed successfully");
                request.status = MergeStatus::Completed;
                request.commit_sha = Some(commit_sha.clone());
                request.updated_at = Utc::now();

                // Update worktree status
                if let Some(mut worktree) = self.worktree_repo.get_by_task(request.task_id).await? {
                    worktree.merged(commit_sha.clone());
                    let _ = self.worktree_repo.update(&worktree).await;
                }

                Ok(MergeResult {
                    request_id: request.id,
                    success: true,
                    commit_sha: Some(commit_sha),
                    error: None,
                    had_conflicts: false,
                    conflict_files: vec![],
                })
            }
            Err(e) => {
                tracing::error!(task_id = %request.task_id, error = %e, "stage 2 merge to main failed");
                request.status = MergeStatus::Failed;
                request.error = Some(e.to_string());
                request.updated_at = Utc::now();

                Ok(MergeResult {
                    request_id: request.id,
                    success: false,
                    commit_sha: None,
                    error: request.error.clone(),
                    had_conflicts: false,
                    conflict_files: vec![],
                })
            }
        }
    }

    /// Check for merge conflicts without actually merging.
    #[instrument(skip(self), fields(%workdir, %source, %target))]
    async fn check_merge_conflicts(
        &self,
        workdir: &str,
        source: &str,
        target: &str,
    ) -> DomainResult<(Vec<String>, bool)> {
        let workdir = workdir.to_string();
        let source = source.to_string();
        let target = target.to_string();

        tokio::task::spawn_blocking(move || {
            let _span = tracing::info_span!("git_merge_tree", %workdir).entered();

            // Use git merge-tree to check for conflicts without modifying worktree
            let output = Command::new("git")
                .args(["merge-tree", &target, &source])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

            let stdout = String::from_utf8_lossy(&output.stdout);

            // Look for conflict markers
            let has_conflicts = stdout.contains("<<<<<<<") || stdout.contains(">>>>>>>");

            if has_conflicts {
                // Extract conflicting file names
                let mut conflicts = Vec::new();
                for line in stdout.lines() {
                    // merge-tree output format includes file paths
                    if (line.starts_with("+++") || line.starts_with("---"))
                        && let Some(path) = line.split_whitespace().nth(1)
                            && !path.starts_with("a/") && !path.starts_with("b/")
                                && !conflicts.contains(&path.to_string()) {
                                    conflicts.push(path.to_string());
                                }
                }
                Ok((conflicts, true))
            } else {
                Ok((vec![], false))
            }
        })
        .await
        .map_err(|e| DomainError::ValidationFailed(format!("spawn_blocking panicked: {}", e)))?
    }

    /// Perform a git merge.
    #[instrument(skip(self), fields(%workdir, %source, %target))]
    async fn git_merge(
        &self,
        workdir: &str,
        source: &str,
        target: &str,
    ) -> DomainResult<String> {
        let workdir = workdir.to_string();
        let source = source.to_string();
        let target = target.to_string();

        tokio::task::spawn_blocking(move || {
            let _span = tracing::info_span!("git_merge_ops", %workdir, %source, %target).entered();

            // Checkout target branch
            let checkout = Command::new("git")
                .args(["checkout", &target])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

            if !checkout.status.success() {
                let stderr = String::from_utf8_lossy(&checkout.stderr);
                return Err(DomainError::ValidationFailed(format!("Git checkout failed: {}", stderr)));
            }

            // Merge source into target
            let merge_msg = format!("Merge {} into {}", source, target);
            let merge = Command::new("git")
                .args(["merge", "--no-ff", &source, "-m", &merge_msg])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

            if !merge.status.success() {
                // Abort the merge if it failed
                let _ = Command::new("git")
                    .args(["merge", "--abort"])
                    .current_dir(&workdir)
                    .output();

                let stderr = String::from_utf8_lossy(&merge.stderr);
                return Err(DomainError::ValidationFailed(format!("Git merge failed: {}", stderr)));
            }

            // Get the merge commit SHA
            let rev_parse = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

            let commit_sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_string();
            Ok(commit_sha)
        })
        .await
        .map_err(|e| DomainError::ValidationFailed(format!("spawn_blocking panicked: {}", e)))?
    }

    /// Get the current queue (non-terminal requests).
    pub async fn get_queue(&self) -> Vec<MergeRequest> {
        let mut result = Vec::new();
        for status in [MergeStatus::Queued, MergeStatus::InProgress, MergeStatus::Conflict] {
            if let Ok(reqs) = self.merge_repo.list_by_status(status).await {
                result.extend(reqs);
            }
        }
        result
    }

    /// Get merge history (terminal requests).
    pub async fn get_history(&self, _limit: usize) -> Vec<MergeRequest> {
        let mut result = Vec::new();
        for status in [MergeStatus::Completed, MergeStatus::Failed, MergeStatus::VerificationFailed] {
            if let Ok(reqs) = self.merge_repo.list_by_status(status).await {
                result.extend(reqs);
            }
        }
        result
    }

    /// Get statistics about the merge queue.
    pub async fn get_stats(&self) -> MergeQueueStats {
        let mut stats = MergeQueueStats::default();

        for status in [MergeStatus::Queued, MergeStatus::InProgress, MergeStatus::Completed,
                       MergeStatus::Failed, MergeStatus::Conflict, MergeStatus::VerificationFailed] {
            let count = self.merge_repo.list_by_status(status).await
                .map(|v| v.len())
                .unwrap_or(0);
            match status {
                MergeStatus::Queued => stats.queued = count,
                MergeStatus::InProgress => stats.in_progress = count,
                MergeStatus::Completed => {
                    stats.completed = count;
                    // Count stage breakdowns from the completed list
                    if let Ok(reqs) = self.merge_repo.list_by_status(MergeStatus::Completed).await {
                        for req in &reqs {
                            match req.stage {
                                MergeStage::AgentToTask => stats.stage1_completed += 1,
                                MergeStage::TaskToMain => stats.stage2_completed += 1,
                            }
                        }
                    }
                }
                MergeStatus::Failed | MergeStatus::VerificationFailed => stats.failed += count,
                MergeStatus::Conflict => stats.conflicts = count,
            }
        }

        stats
    }

    /// Get a specific merge request by ID.
    pub async fn get_request(&self, id: Uuid) -> Option<MergeRequest> {
        self.merge_repo.get(id).await.ok().flatten()
    }

    /// Cancel a queued merge request.
    pub async fn cancel(&self, id: Uuid) -> DomainResult<bool> {
        tracing::info!(merge_request_id = %id, "cancelling merge request");
        if let Some(mut req) = self.merge_repo.get(id).await? {
            if req.status == MergeStatus::Queued {
                req.status = MergeStatus::Failed;
                req.error = Some("Cancelled".to_string());
                req.updated_at = Utc::now();
                self.merge_repo.update(&req).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Process all queued merges.
    pub async fn process_all(&self) -> DomainResult<Vec<MergeResult>> {
        tracing::info!("processing all queued merge requests");
        let mut results = Vec::new();
        while let Some(result) = self.process_next().await? {
            results.push(result);
        }
        tracing::info!(processed_count = results.len(), "finished processing all merge requests");
        Ok(results)
    }

    /// Queue Stage 2 merge for a task if all its subtasks are complete.
    pub async fn queue_stage2_if_ready(&self, task_id: Uuid) -> DomainResult<Option<Uuid>> {
        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        // Check if task is complete
        if task.status != TaskStatus::Complete {
            tracing::debug!(%task_id, status = ?task.status, "task not ready for stage 2: not complete");
            return Ok(None);
        }

        // Check if all dependencies are complete
        let deps = self.task_repo.get_dependencies(task_id).await?;
        let all_deps_complete = deps.iter().all(|t| t.status == TaskStatus::Complete);

        if !all_deps_complete {
            tracing::debug!(%task_id, "task not ready for stage 2: dependencies incomplete");
            return Ok(None);
        }

        // Check worktree exists and is completed
        if let Some(worktree) = self.worktree_repo.get_by_task(task_id).await?
            && worktree.status == WorktreeStatus::Completed {
                tracing::info!(%task_id, "task ready for stage 2 — merge queued");
                let id = self.queue_stage2(task_id).await?;
                return Ok(Some(id));
            }

        Ok(None)
    }

    /// Get all merge requests that have conflicts and need specialist resolution.
    ///
    /// Queries persistent storage so conflicts survive across MergeQueue instances.
    pub async fn get_conflicts_needing_resolution(&self) -> Vec<ConflictResolutionRequest> {
        if !self.config.route_conflicts_to_specialist {
            return vec![];
        }

        let conflicts = match self.merge_repo.list_unresolved_conflicts(self.config.max_retries).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to query unresolved conflicts: {}", e);
                return vec![];
            }
        };

        conflicts.into_iter().map(|req| {
            ConflictResolutionRequest {
                merge_request_id: req.id,
                task_id: req.task_id,
                source_branch: req.source_branch,
                target_branch: req.target_branch,
                workdir: req.workdir,
                conflict_files: req.conflict_files,
                detected_at: req.updated_at,
                attempts: req.attempts,
            }
        }).collect()
    }

    /// Mark a conflict as resolved and retry the merge.
    ///
    /// Should be called after a specialist agent has resolved the conflicts
    /// in the working directory. Looks up the merge request from persistent
    /// storage, so it works even across ephemeral MergeQueue instances.
    pub async fn retry_after_conflict_resolution(&self, merge_request_id: Uuid) -> DomainResult<bool> {
        tracing::info!(merge_request_id = %merge_request_id, "re-queuing merge request after conflict resolution");

        let mut request = match self.merge_repo.get(merge_request_id).await? {
            Some(req) => req,
            None => {
                tracing::warn!(merge_request_id = %merge_request_id, "merge request not found for conflict resolution retry");
                return Ok(false);
            }
        };

        if request.status != MergeStatus::Conflict {
            tracing::warn!(
                merge_request_id = %merge_request_id,
                status = request.status.as_str(),
                "merge request is not in Conflict status, skipping retry"
            );
            return Ok(false);
        }

        request.status = MergeStatus::Queued;
        request.error = None;
        request.conflict_files.clear();
        request.attempts += 1;
        request.updated_at = Utc::now();
        self.merge_repo.update(&request).await?;

        tracing::info!(
            merge_request_id = %merge_request_id,
            attempts = request.attempts,
            "merge request re-queued after conflict resolution"
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_request_stage1() {
        let task_id = Uuid::new_v4();
        let req = MergeRequest::new_stage1(
            task_id,
            "agent-branch".to_string(),
            "task-branch".to_string(),
            "/work".to_string(),
        );

        assert_eq!(req.stage, MergeStage::AgentToTask);
        assert_eq!(req.status, MergeStatus::Queued);
        assert_eq!(req.task_id, task_id);
    }

    #[test]
    fn test_merge_request_stage2() {
        let task_id = Uuid::new_v4();
        let req = MergeRequest::new_stage2(
            task_id,
            "task-branch".to_string(),
            "main".to_string(),
            "/work".to_string(),
        );

        assert_eq!(req.stage, MergeStage::TaskToMain);
        assert_eq!(req.status, MergeStatus::Queued);
    }

    #[test]
    fn test_merge_queue_config_default() {
        let config = MergeQueueConfig::default();
        assert_eq!(config.main_branch, "main");
        assert!(config.require_verification);
        assert!(config.auto_retry);
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_merge_result_serialization() {
        let result = MergeResult {
            request_id: Uuid::new_v4(),
            success: true,
            commit_sha: Some("abc123".to_string()),
            error: None,
            had_conflicts: false,
            conflict_files: vec![],
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"commit_sha\":\"abc123\""));
    }

    #[test]
    fn test_merge_stage_str_roundtrip() {
        for stage in [MergeStage::AgentToTask, MergeStage::TaskToMain] {
            let s = stage.as_str();
            assert_eq!(MergeStage::from_str(s), Some(stage));
        }
        assert_eq!(MergeStage::from_str("invalid"), None);
    }

    #[test]
    fn test_merge_status_str_roundtrip() {
        for status in [
            MergeStatus::Queued, MergeStatus::InProgress, MergeStatus::Completed,
            MergeStatus::Failed, MergeStatus::Conflict, MergeStatus::VerificationFailed,
        ] {
            let s = status.as_str();
            assert_eq!(MergeStatus::from_str(s), Some(status));
        }
        assert_eq!(MergeStatus::from_str("invalid"), None);
    }

    #[test]
    fn test_merge_status_is_terminal() {
        assert!(MergeStatus::Completed.is_terminal());
        assert!(MergeStatus::Failed.is_terminal());
        assert!(MergeStatus::VerificationFailed.is_terminal());
        assert!(!MergeStatus::Queued.is_terminal());
        assert!(!MergeStatus::InProgress.is_terminal());
        assert!(!MergeStatus::Conflict.is_terminal());
    }

    #[test]
    fn test_new_fields_initialized() {
        let req = MergeRequest::new_stage1(
            Uuid::new_v4(),
            "src".to_string(),
            "dst".to_string(),
            "/work".to_string(),
        );
        assert!(req.conflict_files.is_empty());
        assert_eq!(req.attempts, 0);
    }
}

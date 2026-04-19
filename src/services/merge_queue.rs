//! Two-Stage Merge Queue Service.
//!
//! Implements a two-stage merge process:
//! - Stage 1: Agent worktree branches → Task feature branch
//! - Stage 2: Task feature branch → Main branch (with verification)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{TaskStatus, WorktreeStatus};
use crate::domain::ports::{
    GoalRepository, MergeRequestRepository, TaskRepository, WorktreeRepository,
};
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
        matches!(
            self,
            Self::Completed | Self::Failed | Self::VerificationFailed
        )
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
    pub fn new_stage1(
        task_id: Uuid,
        source_branch: String,
        target_branch: String,
        workdir: String,
    ) -> Self {
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

    pub fn new_stage2(
        task_id: Uuid,
        source_branch: String,
        target_branch: String,
        workdir: String,
    ) -> Self {
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
    /// Allowed base directory for worktree working directories.
    /// All workdir paths must resolve to a subdirectory of this path
    /// (joined with `repo_path`) to prevent path traversal attacks.
    pub allowed_workdir_base: String,
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
            allowed_workdir_base: ".abathur/worktrees".to_string(),
        }
    }
}

/// Validate that a working directory path is under the allowed base directory.
///
/// This prevents path traversal attacks where a malicious or corrupted database
/// entry could cause git commands to execute in arbitrary directories (e.g.,
/// `/etc`, `/home/user/.ssh`).
///
/// The function canonicalizes both the workdir and the allowed base path, then
/// checks that the workdir is a subdirectory of the base. If the workdir does
/// not yet exist on disk, it falls back to canonicalizing the parent directory
/// and checking that the constructed path would be under the base.
pub fn validate_workdir(workdir: &str, repo_path: &str, allowed_base: &str) -> DomainResult<()> {
    let repo = Path::new(repo_path);
    let base = repo.join(allowed_base);

    // Canonicalize the base path — it must exist.
    let canonical_base = base.canonicalize().map_err(|e| {
        DomainError::ValidationFailed(format!(
            "Cannot canonicalize allowed workdir base '{}': {}",
            base.display(),
            e
        ))
    })?;

    // Try to canonicalize the workdir directly (works if it exists).
    let workdir_path = Path::new(workdir);
    let canonical_workdir = if workdir_path.is_absolute() {
        workdir_path.canonicalize()
    } else {
        repo.join(workdir).canonicalize()
    };

    match canonical_workdir {
        Ok(cwd) => {
            if !cwd.starts_with(&canonical_base) {
                return Err(DomainError::ValidationFailed(format!(
                    "Path traversal detected: workdir '{}' resolves to '{}' which is outside allowed base '{}'",
                    workdir,
                    cwd.display(),
                    canonical_base.display()
                )));
            }
            Ok(())
        }
        Err(_) => {
            // Path doesn't exist yet — canonicalize the parent and check.
            let full_path = if workdir_path.is_absolute() {
                workdir_path.to_path_buf()
            } else {
                repo.join(workdir)
            };

            if let Some(parent) = full_path.parent()
                && let Ok(canonical_parent) = parent.canonicalize()
            {
                let file_name = full_path.file_name().unwrap_or_default();
                let reconstructed = canonical_parent.join(file_name);
                if !reconstructed.starts_with(&canonical_base) {
                    return Err(DomainError::ValidationFailed(format!(
                        "Path traversal detected: workdir '{}' would resolve outside allowed base '{}'",
                        workdir,
                        canonical_base.display()
                    )));
                }
                return Ok(());
            }

            // If we can't resolve the parent either, do a textual check as a
            // last-resort defense. Reject anything with `..` components.
            let normalized = full_path.to_string_lossy();
            if normalized.contains("..") {
                return Err(DomainError::ValidationFailed(format!(
                    "Path traversal detected: workdir '{}' contains '..' components",
                    workdir
                )));
            }

            Ok(())
        }
    }
}

/// Validate a branch name to prevent git flag injection.
///
/// Branch names must:
/// - Not be empty
/// - Start with an ASCII alphanumeric character (prevents `-` prefix flag injection)
/// - Contain only `[a-zA-Z0-9/_.\-]` characters
///
/// This is intentionally conservative — it rejects names that git itself might
/// allow (e.g. names with `~`, `^`, `:`), but the restricted set is sufficient
/// for all branches created by the Abathur system.
pub fn validate_branch_name(name: &str) -> DomainResult<()> {
    if name.is_empty() {
        return Err(DomainError::ValidationFailed(
            "Branch name must not be empty".to_string(),
        ));
    }

    let first = name.as_bytes()[0];
    if !(first.is_ascii_alphanumeric()) {
        return Err(DomainError::ValidationFailed(format!(
            "Branch name must start with an alphanumeric character, got: '{}'",
            name
        )));
    }

    for ch in name.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '/' || ch == '_' || ch == '.' || ch == '-') {
            return Err(DomainError::ValidationFailed(format!(
                "Branch name contains invalid character '{}': '{}'",
                ch, name
            )));
        }
    }

    Ok(())
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

/// Extract conflicting file paths from `git merge-tree --write-tree` output.
///
/// When `git merge-tree --write-tree` exits with code 1, its stdout contains
/// informational messages in various formats:
///   - `CONFLICT (content): Merge conflict in <path>`
///   - `CONFLICT (rename/delete): <path> renamed to <path> in ..., deleted in ...`
///   - `CONFLICT (modify/delete): <path> deleted in ... and modified in ...`
///   - `CONFLICT (add/add): Merge conflict in <path>`
///
/// This function parses all CONFLICT lines and returns unique file paths.
fn extract_conflict_files(merge_tree_stdout: &str) -> Vec<String> {
    let mut conflicts = Vec::new();
    for line in merge_tree_stdout.lines() {
        let Some(rest) = line.strip_prefix("CONFLICT ") else {
            continue;
        };
        // Skip past the conflict type in parentheses, e.g. "(content): "
        let after_type = match rest.find("): ") {
            Some(idx) => &rest[idx + 3..],
            None => continue,
        };
        // Try the common "Merge conflict in <path>" pattern first
        if let Some(path) = after_type
            .strip_prefix("Merge conflict in ")
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let path = path.to_string();
            if !conflicts.contains(&path) {
                conflicts.push(path);
            }
        } else {
            // For rename/delete, modify/delete, etc., extract the first file path
            // which appears as the first whitespace-delimited token after "): "
            let token = after_type.split_whitespace().next().unwrap_or("");
            let path = token.trim().to_string();
            if !path.is_empty() && !conflicts.contains(&path) {
                conflicts.push(path);
            }
        }
    }
    conflicts
}

/// Two-Stage Merge Queue Service.
///
/// Persists merge requests to a `MergeRequestRepository` so that conflict
/// records survive across ephemeral instances and process restarts.
pub struct MergeQueue<T: ?Sized, G: ?Sized, W: ?Sized>
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

impl<T: ?Sized, G: ?Sized, W: ?Sized> MergeQueue<T, G, W>
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
        let worktree = self
            .worktree_repo
            .get_by_task(task_id)
            .await?
            .ok_or_else(|| {
                DomainError::ValidationFailed(format!("No worktree found for task {}", task_id))
            })?;

        // Validate workdir is under the allowed base to prevent path traversal
        validate_workdir(
            &worktree.path,
            &self.config.repo_path,
            &self.config.allowed_workdir_base,
        )?;

        // Validate branch names to prevent git flag injection
        validate_branch_name(agent_branch)?;
        validate_branch_name(task_branch)?;

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

        // Validate workdir is under the allowed base to prevent path traversal
        validate_workdir(
            target_workdir,
            &self.config.repo_path,
            &self.config.allowed_workdir_base,
        )?;

        // Validate branch names to prevent git flag injection
        validate_branch_name(source_branch)?;
        validate_branch_name(target_branch)?;

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
        let worktree = self
            .worktree_repo
            .get_by_task(task_id)
            .await?
            .ok_or_else(|| {
                DomainError::ValidationFailed(format!("No worktree found for task {}", task_id))
            })?;

        // Validate workdir is under the allowed base to prevent path traversal
        validate_workdir(
            &worktree.path,
            &self.config.repo_path,
            &self.config.allowed_workdir_base,
        )?;

        // Validate branch names to prevent git flag injection
        validate_branch_name(&worktree.branch)?;
        validate_branch_name(&self.config.main_branch)?;

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
        let in_progress = self
            .merge_repo
            .list_by_status(MergeStatus::InProgress)
            .await?;
        if in_progress
            .iter()
            .any(|r| r.target_branch == request.target_branch)
        {
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
        // Validate workdir is under the allowed base to prevent path traversal
        validate_workdir(
            &request.workdir,
            &self.config.repo_path,
            &self.config.allowed_workdir_base,
        )?;

        // Verify workdir still exists
        if !std::path::Path::new(&request.workdir).exists() {
            tracing::error!(task_id = %request.task_id, workdir = %request.workdir, "workdir does not exist");
            request.status = MergeStatus::Failed;
            request.error = Some(format!(
                "Working directory no longer exists: {}",
                request.workdir
            ));
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
        let conflict_check = self
            .check_merge_conflicts(
                &request.workdir,
                &request.source_branch,
                &request.target_branch,
            )
            .await?;

        if conflict_check.1 {
            tracing::warn!(task_id = %request.task_id, conflict_count = conflict_check.0.len(), "stage 1 merge: conflicts detected");
            request.status = MergeStatus::Conflict;
            request.conflict_files = conflict_check.0.clone();
            request.error = Some(if conflict_check.0.is_empty() {
                "Merge conflicts detected".to_string()
            } else {
                format!("Merge conflicts in: {}", conflict_check.0.join(", "))
            });
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
        match self
            .git_merge(
                &request.workdir,
                &request.source_branch,
                &request.target_branch,
            )
            .await
        {
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

        // Use git plumbing commands to merge without touching the working tree.
        // This avoids checkout + merge in the main worktree which can leave
        // uncommitted changes on failure or race conditions.
        let repo_path = self.config.repo_path.clone();
        let source = request.source_branch.clone();
        let target = request.target_branch.clone();

        let plumbing_result = tokio::task::spawn_blocking(move || {
            // merge-tree --write-tree creates the merged tree without modifying any worktree.
            // Exit 0 = clean merge (stdout = tree SHA), exit 1 = conflicts.
            let merge_tree = Command::new("git")
                .args(["merge-tree", "--write-tree", &target, &source])
                .current_dir(&repo_path)
                .output()
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("git merge-tree failed to run: {}", e),
                })?;

            let stdout = String::from_utf8_lossy(&merge_tree.stdout).to_string();

            if merge_tree.status.code() == Some(1) {
                // Conflicts detected — extract file names from CONFLICT lines
                let conflict_files = extract_conflict_files(&stdout);
                return Ok((None, conflict_files));
            }

            if !merge_tree.status.success() {
                let stderr = String::from_utf8_lossy(&merge_tree.stderr);
                return Err(DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("git merge-tree failed: {}", stderr),
                });
            }

            let tree_sha = stdout.lines().next().unwrap_or("").trim().to_string();

            // Resolve parent commit SHAs
            let target_sha = {
                let out = Command::new("git")
                    .args(["rev-parse", &target])
                    .current_dir(&repo_path)
                    .output()
                    .map_err(|e| DomainError::ExternalServiceError {
                        service: "git".to_string(),
                        reason: format!("rev-parse target: {}", e),
                    })?;
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            };
            let source_sha = {
                let out = Command::new("git")
                    .args(["rev-parse", &source])
                    .current_dir(&repo_path)
                    .output()
                    .map_err(|e| DomainError::ExternalServiceError {
                        service: "git".to_string(),
                        reason: format!("rev-parse source: {}", e),
                    })?;
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            };

            // Create merge commit from the tree
            let merge_msg = format!("Merge {} into {}", source, target);
            let commit = Command::new("git")
                .args([
                    "commit-tree",
                    &tree_sha,
                    "-p",
                    &target_sha,
                    "-p",
                    &source_sha,
                    "-m",
                    &merge_msg,
                ])
                .current_dir(&repo_path)
                .output()
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("git commit-tree failed: {}", e),
                })?;

            if !commit.status.success() {
                let stderr = String::from_utf8_lossy(&commit.stderr);
                return Err(DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("git commit-tree: {}", stderr),
                });
            }

            let commit_sha = String::from_utf8_lossy(&commit.stdout).trim().to_string();

            // Atomically update the branch ref (CAS prevents races)
            let update = Command::new("git")
                .args([
                    "update-ref",
                    &format!("refs/heads/{}", target),
                    &commit_sha,
                    &target_sha,
                ])
                .current_dir(&repo_path)
                .output()
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("git update-ref failed: {}", e),
                })?;

            if !update.status.success() {
                let stderr = String::from_utf8_lossy(&update.stderr);
                return Err(DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("git update-ref: {}", stderr),
                });
            }

            // Sync main worktree to match the updated branch tip
            let _ = Command::new("git")
                .args(["reset", "--hard", "HEAD"])
                .current_dir(&repo_path)
                .output();

            Ok((Some(commit_sha), vec![]))
        })
        .await
        .map_err(|e| DomainError::ExecutionFailed(format!("spawn_blocking panicked: {}", e)))??;

        match plumbing_result {
            (None, conflict_files) => {
                tracing::warn!(task_id = %request.task_id, conflict_count = conflict_files.len(), "stage 2 merge: conflicts detected");
                request.status = MergeStatus::Conflict;
                request.conflict_files = conflict_files.clone();
                request.error = Some(format!("Merge conflicts in: {}", conflict_files.join(", ")));
                request.updated_at = Utc::now();

                Ok(MergeResult {
                    request_id: request.id,
                    success: false,
                    commit_sha: None,
                    error: request.error.clone(),
                    had_conflicts: true,
                    conflict_files,
                })
            }
            (Some(commit_sha), _) => {
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

            // Use git merge-tree --write-tree (Git 2.38+) to check for conflicts
            // without modifying the worktree. Exit code 1 indicates conflicts.
            let output = Command::new("git")
                .args(["merge-tree", "--write-tree", &target, &source])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("Failed to run git: {}", e),
                })?;

            // Exit code 0 = clean merge, 1 = conflicts, ≥2 = git error.
            // Requires Git 2.38+ for --write-tree support.
            match output.status.code() {
                Some(0) => Ok((vec![], false)),
                Some(1) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let conflicts = extract_conflict_files(&stdout);
                    Ok((conflicts, true))
                }
                _ => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(DomainError::ExternalServiceError {
                        service: "git".to_string(),
                        reason: format!(
                            "git merge-tree failed (exit {:?}): {}",
                            output.status.code(),
                            stderr
                        ),
                    })
                }
            }
        })
        .await
        .map_err(|e| DomainError::ExecutionFailed(format!("spawn_blocking panicked: {}", e)))?
    }

    /// Perform a git merge.
    #[instrument(skip(self), fields(%workdir, %source, %target))]
    async fn git_merge(&self, workdir: &str, source: &str, target: &str) -> DomainResult<String> {
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
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("Failed to run git: {}", e),
                })?;

            if !checkout.status.success() {
                let stderr = String::from_utf8_lossy(&checkout.stderr);
                return Err(DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("Git checkout failed: {}", stderr),
                });
            }

            // Merge source into target
            let merge_msg = format!("Merge {} into {}", source, target);
            let merge = Command::new("git")
                .args(["merge", "--no-ff", &source, "-m", &merge_msg])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("Failed to run git: {}", e),
                })?;

            if !merge.status.success() {
                // Abort the merge if it failed
                let _ = Command::new("git")
                    .args(["merge", "--abort"])
                    .current_dir(&workdir)
                    .output();

                let stderr = String::from_utf8_lossy(&merge.stderr);
                return Err(DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("Git merge failed: {}", stderr),
                });
            }

            // Get the merge commit SHA
            let rev_parse = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&workdir)
                .output()
                .map_err(|e| DomainError::ExternalServiceError {
                    service: "git".to_string(),
                    reason: format!("Failed to run git: {}", e),
                })?;

            let commit_sha = String::from_utf8_lossy(&rev_parse.stdout)
                .trim()
                .to_string();
            Ok(commit_sha)
        })
        .await
        .map_err(|e| DomainError::ExecutionFailed(format!("spawn_blocking panicked: {}", e)))?
    }

    /// Get the current queue (non-terminal requests).
    pub async fn get_queue(&self) -> Vec<MergeRequest> {
        let mut result = Vec::new();
        for status in [
            MergeStatus::Queued,
            MergeStatus::InProgress,
            MergeStatus::Conflict,
        ] {
            if let Ok(reqs) = self.merge_repo.list_by_status(status).await {
                result.extend(reqs);
            }
        }
        result
    }

    /// Get merge history (terminal requests).
    pub async fn get_history(&self, _limit: usize) -> Vec<MergeRequest> {
        let mut result = Vec::new();
        for status in [
            MergeStatus::Completed,
            MergeStatus::Failed,
            MergeStatus::VerificationFailed,
        ] {
            if let Ok(reqs) = self.merge_repo.list_by_status(status).await {
                result.extend(reqs);
            }
        }
        result
    }

    /// Get statistics about the merge queue.
    pub async fn get_stats(&self) -> MergeQueueStats {
        let mut stats = MergeQueueStats::default();

        for status in [
            MergeStatus::Queued,
            MergeStatus::InProgress,
            MergeStatus::Completed,
            MergeStatus::Failed,
            MergeStatus::Conflict,
            MergeStatus::VerificationFailed,
        ] {
            let count = self
                .merge_repo
                .list_by_status(status)
                .await
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
        if let Some(mut req) = self
            .merge_repo
            .get(id)
            .await?
            .filter(|r| r.status == MergeStatus::Queued)
        {
            req.status = MergeStatus::Failed;
            req.error = Some("Cancelled".to_string());
            req.updated_at = Utc::now();
            self.merge_repo.update(&req).await?;
            return Ok(true);
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
        tracing::info!(
            processed_count = results.len(),
            "finished processing all merge requests"
        );
        Ok(results)
    }

    /// Queue Stage 2 merge for a task if all its subtasks are complete.
    pub async fn queue_stage2_if_ready(&self, task_id: Uuid) -> DomainResult<Option<Uuid>> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
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
            && worktree.status == WorktreeStatus::Completed
        {
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

        let conflicts = match self
            .merge_repo
            .list_unresolved_conflicts(self.config.max_retries)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to query unresolved conflicts: {}", e);
                return vec![];
            }
        };

        conflicts
            .into_iter()
            .map(|req| ConflictResolutionRequest {
                merge_request_id: req.id,
                task_id: req.task_id,
                source_branch: req.source_branch,
                target_branch: req.target_branch,
                workdir: req.workdir,
                conflict_files: req.conflict_files,
                detected_at: req.updated_at,
                attempts: req.attempts,
            })
            .collect()
    }

    /// Mark a conflict as resolved and retry the merge.
    ///
    /// Should be called after a specialist agent has resolved the conflicts
    /// in the working directory. Looks up the merge request from persistent
    /// storage, so it works even across ephemeral MergeQueue instances.
    pub async fn retry_after_conflict_resolution(
        &self,
        merge_request_id: Uuid,
    ) -> DomainResult<bool> {
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
            MergeStatus::Queued,
            MergeStatus::InProgress,
            MergeStatus::Completed,
            MergeStatus::Failed,
            MergeStatus::Conflict,
            MergeStatus::VerificationFailed,
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
    fn test_extract_conflict_files_single() {
        let output = "\
abc123def456\n\
CONFLICT (content): Merge conflict in src/main.rs\n";
        let files = extract_conflict_files(output);
        assert_eq!(files, vec!["src/main.rs"]);
    }

    #[test]
    fn test_extract_conflict_files_multiple() {
        let output = "\
abc123def456\n\
CONFLICT (content): Merge conflict in src/main.rs\n\
CONFLICT (content): Merge conflict in src/lib.rs\n\
CONFLICT (modify/delete): Merge conflict in docs/README.md\n";
        let files = extract_conflict_files(output);
        assert_eq!(files, vec!["src/main.rs", "src/lib.rs", "docs/README.md"]);
    }

    #[test]
    fn test_extract_conflict_files_no_conflicts() {
        let output = "abc123def456\n";
        let files = extract_conflict_files(output);
        assert!(files.is_empty());
    }

    #[test]
    fn test_extract_conflict_files_empty_input() {
        let files = extract_conflict_files("");
        assert!(files.is_empty());
    }

    #[test]
    fn test_extract_conflict_files_deduplicates() {
        let output = "\
CONFLICT (content): Merge conflict in src/main.rs\n\
CONFLICT (content): Merge conflict in src/main.rs\n";
        let files = extract_conflict_files(output);
        assert_eq!(files, vec!["src/main.rs"]);
    }

    #[test]
    fn test_extract_conflict_files_rename_delete() {
        let output = "\
CONFLICT (rename/delete): old_name.rs renamed to new_name.rs in HEAD, deleted in feature\n";
        let files = extract_conflict_files(output);
        assert_eq!(files, vec!["old_name.rs"]);
    }

    #[test]
    fn test_extract_conflict_files_modify_delete_alternate() {
        let output = "\
CONFLICT (modify/delete): src/config.rs deleted in HEAD and modified in feature\n";
        let files = extract_conflict_files(output);
        assert_eq!(files, vec!["src/config.rs"]);
    }

    #[test]
    fn test_extract_conflict_files_mixed_types() {
        let output = "\
abc123def456\n\
CONFLICT (content): Merge conflict in src/main.rs\n\
CONFLICT (rename/delete): old.rs renamed to new.rs in HEAD, deleted in feature\n\
CONFLICT (modify/delete): src/config.rs deleted in HEAD and modified in feature\n\
CONFLICT (add/add): Merge conflict in src/both_added.rs\n";
        let files = extract_conflict_files(output);
        assert_eq!(
            files,
            vec![
                "src/main.rs",
                "old.rs",
                "src/config.rs",
                "src/both_added.rs"
            ]
        );
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

    #[test]
    fn test_validate_workdir_rejects_dotdot_traversal() {
        // Create a temporary directory structure so the base path exists
        let tmp = std::env::temp_dir().join("abathur_test_validate_dotdot");
        let base = tmp.join(".abathur/worktrees");
        std::fs::create_dir_all(&base).ok();

        let result = validate_workdir(
            ".abathur/worktrees/../../etc/passwd",
            tmp.to_str().unwrap(),
            ".abathur/worktrees",
        );

        // Cleanup
        std::fs::remove_dir_all(&tmp).ok();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Path traversal detected"),
            "Expected path traversal error, got: {}",
            err
        );
    }

    #[test]
    fn test_validate_workdir_rejects_absolute_outside_base() {
        let result = validate_workdir("/etc/passwd", ".", ".abathur/worktrees");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Path traversal detected") || err.contains("Cannot canonicalize"),
            "Expected path traversal or canonicalize error, got: {}",
            err
        );
    }

    #[test]
    fn test_validate_workdir_accepts_valid_relative_path() {
        // Create a temporary directory structure to test with
        let tmp = std::env::temp_dir().join("abathur_test_validate_workdir");
        let base = tmp.join(".abathur/worktrees");
        std::fs::create_dir_all(&base).ok();
        let task_dir = base.join("task-abc12345");
        std::fs::create_dir_all(&task_dir).ok();

        let result = validate_workdir(
            ".abathur/worktrees/task-abc12345",
            tmp.to_str().unwrap(),
            ".abathur/worktrees",
        );

        // Cleanup
        std::fs::remove_dir_all(&tmp).ok();

        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_validate_workdir_accepts_nonexistent_safe_path() {
        // Test with a path that doesn't exist yet but whose parent is valid
        let tmp = std::env::temp_dir().join("abathur_test_validate_nonexist");
        let base = tmp.join(".abathur/worktrees");
        std::fs::create_dir_all(&base).ok();

        let result = validate_workdir(
            ".abathur/worktrees/task-new12345",
            tmp.to_str().unwrap(),
            ".abathur/worktrees",
        );

        // Cleanup
        std::fs::remove_dir_all(&tmp).ok();

        assert!(
            result.is_ok(),
            "Expected Ok for safe non-existent path, got: {:?}",
            result
        );
    }

    #[test]
    fn test_merge_queue_config_default_has_allowed_base() {
        let config = MergeQueueConfig::default();
        assert_eq!(config.allowed_workdir_base, ".abathur/worktrees");
    }

    // --- validate_branch_name tests ---

    #[test]
    fn test_validate_branch_name_accepts_simple() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature-branch").is_ok());
        assert!(validate_branch_name("release/v1.2.3").is_ok());
    }

    #[test]
    fn test_validate_branch_name_accepts_complex_valid() {
        assert!(validate_branch_name("task/abc_123.fix").is_ok());
        assert!(validate_branch_name("a").is_ok());
        assert!(validate_branch_name("A0-b1_c2/d3.e4").is_ok());
    }

    #[test]
    fn test_validate_branch_name_rejects_empty() {
        let result = validate_branch_name("");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_leading_hyphen() {
        let result = validate_branch_name("-Xours");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with an alphanumeric")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_double_hyphen_flag() {
        let result = validate_branch_name("--strategy=recursive");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with an alphanumeric")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_leading_dot() {
        let result = validate_branch_name(".hidden");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with an alphanumeric")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_leading_slash() {
        let result = validate_branch_name("/absolute");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with an alphanumeric")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_special_chars() {
        let result = validate_branch_name("branch~1");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid character")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_spaces() {
        let result = validate_branch_name("my branch");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid character")
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_colon() {
        let result = validate_branch_name("refs:heads");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid character")
        );
    }
}

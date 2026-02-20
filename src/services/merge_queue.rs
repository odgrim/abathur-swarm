//! Two-Stage Merge Queue Service.
//!
//! Implements a two-stage merge process:
//! - Stage 1: Agent worktree branches → Task feature branch
//! - Stage 2: Task feature branch → Main branch (with verification)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{TaskStatus, WorktreeStatus};
use crate::domain::ports::{TaskRepository, WorktreeRepository};
use crate::services::integration_verifier::{IntegrationVerifierService, VerificationResult};
use crate::domain::ports::GoalRepository;

/// Stage of the merge process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStage {
    /// Stage 1: Merging agent work into task branch.
    AgentToTask,
    /// Stage 2: Merging task branch into main.
    TaskToMain,
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
    queue: Arc<RwLock<VecDeque<MergeRequest>>>,
    history: Arc<RwLock<Vec<MergeRequest>>>,
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
    ) -> Self {
        Self {
            task_repo,
            worktree_repo,
            verifier,
            config,
            queue: Arc::new(RwLock::new(VecDeque::new())),
            history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Queue a Stage 1 merge (agent → task branch).
    pub async fn queue_stage1(
        &self,
        task_id: Uuid,
        agent_branch: &str,
        task_branch: &str,
    ) -> DomainResult<Uuid> {
        validate_branch_name(agent_branch)?;
        validate_branch_name(task_branch)?;

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
        self.queue.write().await.push_back(request);
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
        validate_branch_name(source_branch)?;
        validate_branch_name(target_branch)?;

        let request = MergeRequest::new_stage1(
            task_id,
            source_branch.to_string(),
            target_branch.to_string(),
            target_workdir.to_string(),
        );
        let id = request.id;
        self.queue.write().await.push_back(request);
        Ok(id)
    }

    /// Queue a Stage 2 merge (task → main).
    pub async fn queue_stage2(&self, task_id: Uuid) -> DomainResult<Uuid> {
        // Get worktree for this task
        let worktree = self.worktree_repo.get_by_task(task_id).await?
            .ok_or_else(|| DomainError::ValidationFailed(
                format!("No worktree found for task {}", task_id)
            ))?;

        validate_branch_name(&worktree.branch)?;
        validate_branch_name(&self.config.main_branch)?;

        let request = MergeRequest::new_stage2(
            task_id,
            worktree.branch.clone(),
            self.config.main_branch.clone(),
            worktree.path.clone(),
        );

        let id = request.id;
        self.queue.write().await.push_back(request);
        Ok(id)
    }

    /// Process the next merge in the queue.
    pub async fn process_next(&self) -> DomainResult<Option<MergeResult>> {
        // Get next queued request
        let mut request = {
            let mut queue = self.queue.write().await;
            match queue.iter().position(|r| r.status == MergeStatus::Queued) {
                Some(idx) => {
                    let mut req = queue.remove(idx).unwrap();
                    req.status = MergeStatus::InProgress;
                    req.updated_at = Utc::now();
                    queue.push_front(req.clone());
                    req
                }
                None => return Ok(None),
            }
        };

        // Process based on stage
        let result = match request.stage {
            MergeStage::AgentToTask => self.process_stage1(&mut request).await,
            MergeStage::TaskToMain => self.process_stage2(&mut request).await,
        };

        // Update request with result
        {
            let mut queue = self.queue.write().await;
            if let Some(idx) = queue.iter().position(|r| r.id == request.id) {
                queue[idx] = request.clone();
            }
        }

        // Move completed/failed to history
        if request.status != MergeStatus::InProgress {
            let mut queue = self.queue.write().await;
            if let Some(idx) = queue.iter().position(|r| r.id == request.id) {
                let completed = queue.remove(idx).unwrap();
                self.history.write().await.push(completed);
            }
        }

        result.map(Some)
    }

    /// Process a Stage 1 merge (agent → task branch).
    async fn process_stage1(&self, request: &mut MergeRequest) -> DomainResult<MergeResult> {
        // Check for conflicts first
        let conflict_check = self.check_merge_conflicts(
            &request.workdir,
            &request.source_branch,
            &request.target_branch,
        ).await?;

        if !conflict_check.0.is_empty() {
            request.status = MergeStatus::Conflict;
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
    async fn process_stage2(&self, request: &mut MergeRequest) -> DomainResult<MergeResult> {
        // Run verification first if required
        if self.config.require_verification {
            let verification = self.verifier.verify_task(request.task_id).await?;
            request.verification = Some(verification.clone());

            if !verification.passed {
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
            request.status = MergeStatus::Conflict;
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
    async fn check_merge_conflicts(
        &self,
        workdir: &str,
        source: &str,
        target: &str,
    ) -> DomainResult<(Vec<String>, bool)> {
        // Use git merge-tree to check for conflicts without modifying worktree
        let output = Command::new("git")
            .args(["merge-tree", target, source])
            .current_dir(workdir)
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Look for conflict markers
        let has_conflicts = stdout.contains("<<<<<<<") || stdout.contains(">>>>>>>");

        if has_conflicts {
            // Extract conflicting file names
            let mut conflicts = Vec::new();
            for line in stdout.lines() {
                // merge-tree output format includes file paths
                if line.starts_with("+++") || line.starts_with("---") {
                    if let Some(path) = line.split_whitespace().nth(1) {
                        if !path.starts_with("a/") && !path.starts_with("b/") {
                            if !conflicts.contains(&path.to_string()) {
                                conflicts.push(path.to_string());
                            }
                        }
                    }
                }
            }
            Ok((conflicts, true))
        } else {
            Ok((vec![], false))
        }
    }

    /// Perform a git merge.
    async fn git_merge(
        &self,
        workdir: &str,
        source: &str,
        target: &str,
    ) -> DomainResult<String> {
        // Checkout target branch
        let checkout = Command::new("git")
            .args(["checkout", target])
            .current_dir(workdir)
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            return Err(DomainError::ValidationFailed(format!("Git checkout failed: {}", stderr)));
        }

        // Merge source into target
        let merge_msg = format!("Merge {} into {}", source, target);
        let merge = Command::new("git")
            .args(["merge", "--no-ff", "-m", &merge_msg, "--", source])
            .current_dir(workdir)
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        if !merge.status.success() {
            // Abort the merge if it failed
            let _ = Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(workdir)
                .output()
                .await;

            let stderr = String::from_utf8_lossy(&merge.stderr);
            return Err(DomainError::ValidationFailed(format!("Git merge failed: {}", stderr)));
        }

        // Get the merge commit SHA
        let rev_parse = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(workdir)
            .output()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to run git: {}", e)))?;

        let commit_sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_string();
        Ok(commit_sha)
    }

    /// Get the current queue status.
    pub async fn get_queue(&self) -> Vec<MergeRequest> {
        self.queue.read().await.iter().cloned().collect()
    }

    /// Get merge history.
    pub async fn get_history(&self, limit: usize) -> Vec<MergeRequest> {
        let history = self.history.read().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Get statistics about the merge queue.
    pub async fn get_stats(&self) -> MergeQueueStats {
        let queue = self.queue.read().await;
        let history = self.history.read().await;

        let mut stats = MergeQueueStats::default();

        for req in queue.iter() {
            match req.status {
                MergeStatus::Queued => stats.queued += 1,
                MergeStatus::InProgress => stats.in_progress += 1,
                _ => {}
            }
        }

        for req in history.iter() {
            match req.status {
                MergeStatus::Completed => {
                    stats.completed += 1;
                    match req.stage {
                        MergeStage::AgentToTask => stats.stage1_completed += 1,
                        MergeStage::TaskToMain => stats.stage2_completed += 1,
                    }
                }
                MergeStatus::Failed | MergeStatus::VerificationFailed => stats.failed += 1,
                MergeStatus::Conflict => stats.conflicts += 1,
                _ => {}
            }
        }

        stats
    }

    /// Get a specific merge request by ID.
    pub async fn get_request(&self, id: Uuid) -> Option<MergeRequest> {
        // Check queue first
        let queue = self.queue.read().await;
        if let Some(req) = queue.iter().find(|r| r.id == id) {
            return Some(req.clone());
        }

        // Then check history
        let history = self.history.read().await;
        history.iter().find(|r| r.id == id).cloned()
    }

    /// Cancel a queued merge request.
    pub async fn cancel(&self, id: Uuid) -> DomainResult<bool> {
        let mut queue = self.queue.write().await;
        if let Some(idx) = queue.iter().position(|r| r.id == id && r.status == MergeStatus::Queued) {
            queue.remove(idx);
            return Ok(true);
        }
        Ok(false)
    }

    /// Process all queued merges.
    pub async fn process_all(&self) -> DomainResult<Vec<MergeResult>> {
        let mut results = Vec::new();
        while let Some(result) = self.process_next().await? {
            results.push(result);
        }
        Ok(results)
    }

    /// Queue Stage 2 merge for a task if all its subtasks are complete.
    pub async fn queue_stage2_if_ready(&self, task_id: Uuid) -> DomainResult<Option<Uuid>> {
        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        // Check if task is complete
        if task.status != TaskStatus::Complete {
            return Ok(None);
        }

        // Check if all dependencies are complete
        let deps = self.task_repo.get_dependencies(task_id).await?;
        let all_deps_complete = deps.iter().all(|t| t.status == TaskStatus::Complete);

        if !all_deps_complete {
            return Ok(None);
        }

        // Check worktree exists and is completed
        if let Some(worktree) = self.worktree_repo.get_by_task(task_id).await? {
            if worktree.status == WorktreeStatus::Completed {
                let id = self.queue_stage2(task_id).await?;
                return Ok(Some(id));
            }
        }

        Ok(None)
    }

    /// Get all merge requests that have conflicts and need specialist resolution.
    ///
    /// Returns requests that:
    /// - Have status = Conflict
    /// - Are configured to route to specialists
    /// - Haven't exceeded max retry attempts
    pub async fn get_conflicts_needing_resolution(&self) -> Vec<ConflictResolutionRequest> {
        if !self.config.route_conflicts_to_specialist {
            return vec![];
        }

        let queue = self.queue.read().await;
        let history = self.history.read().await;

        let mut conflicts = Vec::new();

        // Check queue for conflicts
        for req in queue.iter() {
            if req.status == MergeStatus::Conflict {
                if let Some(ref error) = req.error {
                    // Parse conflict files from error message
                    let conflict_files = if error.contains("Merge conflicts in:") {
                        error
                            .replace("Merge conflicts in:", "")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .collect()
                    } else {
                        vec![]
                    };

                    conflicts.push(ConflictResolutionRequest {
                        merge_request_id: req.id,
                        task_id: req.task_id,
                        source_branch: req.source_branch.clone(),
                        target_branch: req.target_branch.clone(),
                        workdir: req.workdir.clone(),
                        conflict_files,
                        detected_at: req.updated_at,
                        attempts: 0,
                    });
                }
            }
        }

        // Also check history for recent conflicts (might be retryable)
        for req in history.iter().rev().take(10) {
            if req.status == MergeStatus::Conflict {
                if let Some(ref error) = req.error {
                    let conflict_files = if error.contains("Merge conflicts in:") {
                        error
                            .replace("Merge conflicts in:", "")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .collect()
                    } else {
                        vec![]
                    };

                    conflicts.push(ConflictResolutionRequest {
                        merge_request_id: req.id,
                        task_id: req.task_id,
                        source_branch: req.source_branch.clone(),
                        target_branch: req.target_branch.clone(),
                        workdir: req.workdir.clone(),
                        conflict_files,
                        detected_at: req.updated_at,
                        attempts: 0,
                    });
                }
            }
        }

        conflicts
    }

    /// Mark a conflict as resolved and retry the merge.
    ///
    /// Should be called after a specialist agent has resolved the conflicts
    /// in the working directory.
    pub async fn retry_after_conflict_resolution(&self, merge_request_id: Uuid) -> DomainResult<bool> {
        // Check queue for the request
        {
            let mut queue = self.queue.write().await;
            if let Some(req) = queue.iter_mut().find(|r| r.id == merge_request_id) {
                if req.status == MergeStatus::Conflict {
                    req.status = MergeStatus::Queued;
                    req.error = None;
                    req.updated_at = Utc::now();
                    return Ok(true);
                }
            }
        }

        // Check history and re-queue if found
        {
            let history = self.history.read().await;
            if let Some(req) = history.iter().find(|r| r.id == merge_request_id) {
                if req.status == MergeStatus::Conflict {
                    let mut new_req = req.clone();
                    new_req.status = MergeStatus::Queued;
                    new_req.error = None;
                    new_req.updated_at = Utc::now();

                    drop(history);
                    self.queue.write().await.push_back(new_req);
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}

/// Validates a git branch name to prevent command injection.
///
/// Rejects names that could be interpreted as git flags or otherwise subvert
/// git command execution. Follows `git check-ref-format` rules.
fn validate_branch_name(name: &str) -> DomainResult<()> {
    if name.is_empty() {
        return Err(DomainError::ValidationFailed(
            "Branch name cannot be empty".to_string(),
        ));
    }
    if name.starts_with('-') {
        return Err(DomainError::ValidationFailed(format!(
            "Invalid branch name '{}': must not start with '-'",
            name
        )));
    }
    if name.contains("..") {
        return Err(DomainError::ValidationFailed(format!(
            "Invalid branch name '{}': must not contain '..'",
            name
        )));
    }
    for ch in name.chars() {
        if ch.is_ascii_control()
            || matches!(ch, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        {
            return Err(DomainError::ValidationFailed(format!(
                "Invalid branch name '{}': contains disallowed character '{}'",
                name, ch
            )));
        }
    }
    if name.ends_with(".lock") {
        return Err(DomainError::ValidationFailed(format!(
            "Invalid branch name '{}': must not end with '.lock'",
            name
        )));
    }
    Ok(())
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

    // --- validate_branch_name tests ---

    #[test]
    fn test_validate_branch_name_rejects_empty() {
        let err = validate_branch_name("").unwrap_err();
        assert!(
            err.to_string().contains("empty"),
            "Expected 'empty' in error: {}",
            err
        );
    }

    #[test]
    fn test_validate_branch_name_rejects_leading_dash() {
        assert!(validate_branch_name("-Xours").is_err());
        assert!(validate_branch_name("--strategy=recursive").is_err());
        assert!(validate_branch_name("-").is_err());
        assert!(validate_branch_name("--allow-unrelated-histories").is_err());
    }

    #[test]
    fn test_validate_branch_name_rejects_double_dot() {
        assert!(validate_branch_name("main..evil").is_err());
        assert!(validate_branch_name("feature..main").is_err());
        assert!(validate_branch_name("a..b").is_err());
    }

    #[test]
    fn test_validate_branch_name_rejects_invalid_chars() {
        assert!(validate_branch_name("branch~1").is_err());
        assert!(validate_branch_name("branch^evil").is_err());
        assert!(validate_branch_name("branch:evil").is_err());
        assert!(validate_branch_name("branch?evil").is_err());
        assert!(validate_branch_name("branch*evil").is_err());
        assert!(validate_branch_name("branch[evil").is_err());
        assert!(validate_branch_name("branch\\evil").is_err());
        assert!(validate_branch_name("branch name").is_err());
    }

    #[test]
    fn test_validate_branch_name_rejects_lock_suffix() {
        assert!(validate_branch_name("feature.lock").is_err());
        assert!(validate_branch_name("main.lock").is_err());
    }

    #[test]
    fn test_validate_branch_name_accepts_valid_names() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature/my-feature").is_ok());
        assert!(validate_branch_name("abathur/task-12345678").is_ok());
        assert!(validate_branch_name("task/abc123").is_ok());
        assert!(validate_branch_name("fix/issue-38").is_ok());
        assert!(validate_branch_name("release/1.0.0").is_ok());
        assert!(validate_branch_name("v2.0.0").is_ok());
    }
}

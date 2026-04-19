//! Helper functions for the swarm orchestrator.
//!
//! Top-level utility functions used by spawned tasks that don't have access
//! to the orchestrator instance (e.g., auto-commit, post-completion workflow).

use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::workflow_template::OutputDelivery;
use crate::domain::models::{TaskStatus, WorktreeStatus};
use crate::domain::ports::{
    GoalRepository, MergeRequestRepository, TaskRepository, WorktreeRepository,
};
use crate::services::{
    AuditAction, AuditCategory, AuditLogService, IntegrationVerifierService, MergeQueue,
    MergeQueueConfig, VerifierConfig,
};

use crate::services::event_bus::EventBus;

use super::types::SwarmEvent;

/// Known transient markdown filenames that agents may generate during execution.
/// These must never be committed or left in the working tree.
pub const TRANSIENT_ARTIFACT_FILENAMES: &[&str] = &[
    "REVIEW.md",
    "PLAN.md",
    "NOTES.md",
    "SUMMARY.md",
    "RESEARCH.md",
    "TODO.md",
    "SCRATCH.md",
];

/// Remove any known transient workflow artifacts from the given worktree path.
/// Returns the list of filenames that were actually deleted.
pub fn remove_transient_artifacts(worktree_path: &str) -> Vec<String> {
    let base = Path::new(worktree_path);
    let mut removed = Vec::new();
    for &name in TRANSIENT_ARTIFACT_FILENAMES {
        let path = base.join(name);
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(file = %name, error = %e, "failed to remove transient artifact");
            } else {
                tracing::info!(file = %name, worktree = %worktree_path, "removed transient artifact");
                removed.push(name.to_string());
            }
        }
    }
    removed
}

/// Fetch the latest state of a ref from origin.
///
/// Returns `true` on success, `false` on failure (network, no remote, etc.).
/// On failure, logs a warning — callers should proceed with stale local state
/// rather than blocking the swarm.
async fn sync_with_remote(repo_path: &Path, base_ref: &str) -> bool {
    use tokio::process::Command;
    match Command::new("git")
        .args(["fetch", "origin", base_ref])
        .current_dir(repo_path)
        .output()
        .await
    {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            tracing::warn!(
                base_ref,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "git fetch origin failed — proceeding with stale local state"
            );
            false
        }
        Err(e) => {
            tracing::warn!(base_ref, error = %e, "git fetch command failed — proceeding with stale local state");
            false
        }
    }
}

/// Resolve the actual `.git` directory for a working tree path.
///
/// For regular repos this returns `<path>/.git`. For git worktrees it follows the
/// indirection file to the real git directory. Uses `git rev-parse --git-dir` so it
/// handles both cases uniformly.
async fn git_dir_for(path: &Path) -> Option<std::path::PathBuf> {
    use tokio::process::Command;
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(path)
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let p = std::path::PathBuf::from(p);
            if p.is_absolute() { p } else { path.join(p) }
        })
}

/// Ensure the git working directory at `repo_path` is in a clean state.
///
/// Aborts any in-progress merge, rebase, or cherry-pick, then discards all staged
/// and unstaged changes. Returns `true` if the working directory is confirmed clean
/// afterwards, `false` if recovery failed (caller should abort).
///
/// This is safe to call on the shared repo working directory because it is only used
/// by the auto-ship pipeline — no active development happens there.
async fn ensure_clean_working_dir(repo_path: &Path) -> bool {
    use tokio::process::Command;

    let git_dir = match git_dir_for(repo_path).await {
        Some(d) => d,
        None => {
            tracing::error!(path = %repo_path.display(), "ensure_clean_working_dir: not a git repo");
            return false;
        }
    };

    // 1. Abort in-progress merge. A failure here means we CANNOT guarantee a
    // clean state, so return false immediately rather than fall through to the
    // final status probe (which would misreport "clean" whenever the abort
    // actually failed but git status happened to look empty).
    if git_dir.join("MERGE_HEAD").exists() {
        tracing::warn!(path = %repo_path.display(), "ensure_clean_working_dir: aborting in-progress merge");
        let result = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(repo_path)
            .output()
            .await;
        match &result {
            Ok(o) if !o.status.success() => {
                tracing::error!(
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "ensure_clean_working_dir: merge --abort failed"
                );
                return false;
            }
            Err(e) => {
                tracing::error!(error = %e, "ensure_clean_working_dir: failed to run merge --abort");
                return false;
            }
            _ => {}
        }
    }

    // 2. Abort in-progress rebase
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        tracing::warn!(path = %repo_path.display(), "ensure_clean_working_dir: aborting in-progress rebase");
        let result = Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(repo_path)
            .output()
            .await;
        match &result {
            Ok(o) if !o.status.success() => {
                tracing::error!(
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "ensure_clean_working_dir: rebase --abort failed"
                );
                return false;
            }
            Err(e) => {
                tracing::error!(error = %e, "ensure_clean_working_dir: failed to run rebase --abort");
                return false;
            }
            _ => {}
        }
    }

    // 3. Abort in-progress cherry-pick
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        tracing::warn!(path = %repo_path.display(), "ensure_clean_working_dir: aborting in-progress cherry-pick");
        let result = Command::new("git")
            .args(["cherry-pick", "--abort"])
            .current_dir(repo_path)
            .output()
            .await;
        match &result {
            Ok(o) if !o.status.success() => {
                tracing::error!(
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "ensure_clean_working_dir: cherry-pick --abort failed"
                );
                return false;
            }
            Err(e) => {
                tracing::error!(error = %e, "ensure_clean_working_dir: failed to run cherry-pick --abort");
                return false;
            }
            _ => {}
        }
    }

    // 4. Check for dirty state
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .await;

    let is_dirty = match &status {
        Ok(o) if o.status.success() => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
        _ => true, // assume dirty if we can't check
    };

    if is_dirty {
        tracing::warn!(path = %repo_path.display(), "ensure_clean_working_dir: working directory is dirty — resetting");

        // Reset staged and unstaged changes. A failed reset means we have no
        // confidence the working dir is clean — surface that instead of
        // relying on the subsequent status probe.
        let reset_result = Command::new("git")
            .args(["reset", "--hard", "HEAD"])
            .current_dir(repo_path)
            .output()
            .await;
        match &reset_result {
            Ok(o) if !o.status.success() => {
                tracing::error!(
                    path = %repo_path.display(),
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "ensure_clean_working_dir: git reset --hard HEAD failed"
                );
                return false;
            }
            Err(e) => {
                tracing::error!(
                    path = %repo_path.display(),
                    error = %e,
                    "ensure_clean_working_dir: failed to run git reset --hard HEAD"
                );
                return false;
            }
            _ => {}
        }
    }

    // 5. Verify clean state
    let verify = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .await;

    match verify {
        Ok(o) if o.status.success() => {
            let output = String::from_utf8_lossy(&o.stdout);
            if output.trim().is_empty() {
                true
            } else {
                tracing::error!(
                    path = %repo_path.display(),
                    remaining = %output.trim(),
                    "ensure_clean_working_dir: working directory still dirty after cleanup"
                );
                false
            }
        }
        Ok(o) => {
            tracing::error!(
                path = %repo_path.display(),
                stderr = %String::from_utf8_lossy(&o.stderr),
                "ensure_clean_working_dir: git status failed after cleanup"
            );
            false
        }
        Err(e) => {
            tracing::error!(
                path = %repo_path.display(),
                error = %e,
                "ensure_clean_working_dir: failed to run git status after cleanup"
            );
            false
        }
    }
}

/// Return `true` if the given git push stderr indicates a recoverable rejection
/// (remote advanced / non-fast-forward) that should be retried with fetch+rebase.
///
/// Non-rejection failures (auth, network, repo config) return `false` and should
/// not trigger a fetch+rebase retry because the underlying error will persist.
fn is_rejection_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("rejected")
        || lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("failed to push some refs")
}

/// Push the local base ref to remote. On rejection (remote advanced),
/// fetches and rebases once, then retries. Returns `true` if push succeeded.
async fn push_with_retry(repo_path: &Path, base_ref: &str, max_retries: u32) -> bool {
    use tokio::process::Command;

    // Early exit: check if 'origin' remote exists before entering the retry loop.
    match Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .await
    {
        Ok(output) if !output.status.success() => {
            tracing::warn!("No 'origin' remote configured — skipping push");
            return false;
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to check for 'origin' remote — skipping push");
            return false;
        }
        _ => {} // origin exists, proceed
    }

    for attempt in 0..=max_retries {
        let push = Command::new("git")
            .args(["push", "origin", base_ref])
            .current_dir(repo_path)
            .output()
            .await;
        match push {
            Ok(output) if output.status.success() => return true,
            Ok(output) if attempt < max_retries => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Only fetch+rebase when the failure is a rejection (remote advanced).
                // Auth, network, or repo-config errors won't be fixed by a rebase, so
                // retrying is wasted work — bail out immediately instead.
                if !is_rejection_error(&stderr) {
                    tracing::error!(
                        attempt,
                        base_ref,
                        %stderr,
                        "git push failed with non-retryable error (not a rejection) — aborting"
                    );
                    return false;
                }
                tracing::warn!(attempt, base_ref, %stderr, "git push rejected, fetching and rebasing");
                // Fetch latest. If the fetch itself fails, rebasing would be done
                // onto stale local state — which is exactly the bug we're trying
                // to prevent. Bail out instead of silently retrying on stale refs.
                let fetch = Command::new("git")
                    .args(["fetch", "origin", base_ref])
                    .current_dir(repo_path)
                    .output()
                    .await;
                match &fetch {
                    Ok(o) if !o.status.success() => {
                        tracing::warn!(
                            base_ref,
                            stderr = %String::from_utf8_lossy(&o.stderr),
                            "push_with_retry: git fetch failed — aborting retry to avoid rebasing on stale state"
                        );
                        return false;
                    }
                    Err(e) => {
                        tracing::warn!(
                            base_ref,
                            error = %e,
                            "push_with_retry: git fetch command failed — aborting retry to avoid rebasing on stale state"
                        );
                        return false;
                    }
                    _ => {}
                }
                // Rebase local commits on top of remote
                let rebase = Command::new("git")
                    .args(["rebase", &format!("origin/{}", base_ref)])
                    .current_dir(repo_path)
                    .output()
                    .await;
                if let Ok(r) = &rebase
                    && !r.status.success()
                {
                    tracing::error!(
                        base_ref,
                        stderr = %String::from_utf8_lossy(&r.stderr),
                        "rebase onto origin/{} failed — aborting push retry", base_ref
                    );
                    let abort_result = Command::new("git")
                        .args(["rebase", "--abort"])
                        .current_dir(repo_path)
                        .output()
                        .await;
                    match &abort_result {
                        Ok(o) if !o.status.success() => {
                            tracing::error!(
                                base_ref,
                                stderr = %String::from_utf8_lossy(&o.stderr),
                                "push_with_retry: rebase --abort itself failed"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                base_ref,
                                error = %e,
                                "push_with_retry: failed to run rebase --abort"
                            );
                        }
                        _ => {}
                    }
                    return false;
                }
            }
            Ok(output) => {
                tracing::error!(
                    base_ref,
                    stderr = %String::from_utf8_lossy(&output.stderr),
                    "git push failed after retries"
                );
                return false;
            }
            Err(e) => {
                tracing::error!(base_ref, error = %e, "git push command failed");
                return false;
            }
        }
    }
    false
}

/// Auto-commit any uncommitted changes in a worktree as a safety net.
/// Returns true if a commit was made, false if the worktree was clean.
pub async fn auto_commit_worktree(worktree_path: &str, task_id: Uuid) -> bool {
    use tokio::process::Command;

    // Remove transient workflow artifacts before committing
    let removed = remove_transient_artifacts(worktree_path);
    if !removed.is_empty() {
        tracing::info!(
            task_id = %task_id,
            files = ?removed,
            "cleaned transient artifacts before auto-commit"
        );
    }

    // Check if there are any uncommitted changes
    let status = match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output()
        .await
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
        Err(_) => return false,
    };

    if status.trim().is_empty() {
        return false;
    }

    // Stage all changes. The previous `add.is_err() || !add.unwrap().status.success()`
    // form panicked: `||` short-circuits only on `true`, so on `Err` the RHS
    // still evaluated and `unwrap()` on that `Err` panicked. Explicit match
    // avoids the panic.
    match Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree_path)
        .output()
        .await
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            tracing::warn!(
                task_id = %task_id,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "auto_commit_worktree: git add -A failed"
            );
            return false;
        }
        Err(e) => {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "auto_commit_worktree: failed to run git add -A"
            );
            return false;
        }
    }

    // Commit with a descriptive message
    let msg = format!(
        "auto-commit: captured uncommitted work from task {}\n\n\
         The agent did not commit before ending its session.\n\
         This auto-commit preserves the work for review and merge.",
        &task_id.to_string()[..8]
    );

    let commit = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(worktree_path)
        .output()
        .await;

    match commit {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Try to create a pull request for a completed task's branch.
///
/// Returns the PR URL on success, or `None` if `gh` is unavailable, auth fails,
/// or the push/PR creation fails for any reason.
pub async fn try_create_pull_request(
    worktree_path: &str,
    branch: &str,
    task_title: &str,
    task_description: &str,
    default_base_ref: &str,
) -> Option<String> {
    use tokio::process::Command;

    // Push the branch to origin
    let push = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(worktree_path)
        .output()
        .await;

    match push {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            tracing::warn!(
                "git push failed for branch '{}': {}",
                branch,
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }
        Err(e) => {
            tracing::warn!("git push failed for branch '{}': {}", branch, e);
            return None;
        }
    }

    // Create PR via gh CLI
    let pr_result = Command::new("gh")
        .args([
            "pr",
            "create",
            "--title",
            task_title,
            "--body",
            task_description,
            "--base",
            default_base_ref,
            "--head",
            branch,
        ])
        .current_dir(worktree_path)
        .output()
        .await;

    match pr_result {
        Ok(output) if output.status.success() => {
            let pr_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if pr_url.is_empty() {
                None
            } else {
                Some(pr_url)
            }
        }
        Ok(output) => {
            tracing::warn!(
                "gh pr create failed for branch '{}': {}",
                branch,
                String::from_utf8_lossy(&output.stderr)
            );
            None
        }
        Err(e) => {
            tracing::warn!("gh not available or failed: {}", e);
            None
        }
    }
}

/// Collect a human-readable summary of changes on a branch relative to a base ref.
///
/// Runs `git log --oneline` and `git diff --stat` to produce a concise overview
/// suitable for PR descriptions.
pub(crate) async fn summarize_branch_changes(
    worktree_path: &str,
    branch: &str,
    base_ref: &str,
) -> String {
    use tokio::process::Command;
    let mut summary = String::new();

    // Commit messages
    if let Ok(output) = Command::new("git")
        .args(["log", "--oneline", &format!("{}..{}", base_ref, branch)])
        .current_dir(worktree_path)
        .output()
        .await
    {
        let log = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !log.is_empty() {
            summary.push_str("### Commits\n\n```\n");
            summary.push_str(&log);
            summary.push_str("\n```\n\n");
        }
    }

    // File change stats
    if let Ok(output) = Command::new("git")
        .args(["diff", "--stat", &format!("{}..{}", base_ref, branch)])
        .current_dir(worktree_path)
        .output()
        .await
    {
        let stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stat.is_empty() {
            summary.push_str("### Files Changed\n\n```\n");
            summary.push_str(&stat);
            summary.push_str("\n```\n");
        }
    }

    summary
}

/// Extract a concise summary from a task description.
///
/// For short descriptions (under `max_chars`), returns as-is.
/// For long descriptions (like convergence check prompts), extracts the
/// meaningful first portion and truncates with an ellipsis.
pub(crate) fn truncate_description(description: &str, max_chars: usize) -> String {
    if description.len() <= max_chars {
        return description.to_string();
    }

    // Try to find the first paragraph (before the first blank line or markdown separator)
    let first_para = description.split("\n\n").next().unwrap_or(description);

    // Also try splitting on markdown horizontal rules
    let first_section = first_para.split("\n---\n").next().unwrap_or(first_para);

    // Strip leading markdown headers (# lines) to get to the actual content
    let cleaned: String = first_section
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    let result = if cleaned.is_empty() {
        // All lines were headers — use first non-header line from full desc
        description
            .lines()
            .find(|line| !line.starts_with('#') && !line.trim().is_empty())
            .unwrap_or(description)
            .to_string()
    } else {
        cleaned
    };

    if result.len() <= max_chars {
        result
    } else {
        // Find a char boundary at or before max_chars to avoid panicking on multi-byte UTF-8
        let mut truncate_at = max_chars.min(result.len());
        while truncate_at > 0 && !result.is_char_boundary(truncate_at) {
            truncate_at -= 1;
        }
        format!("{}...", &result[..truncate_at])
    }
}

/// Outcome of merging a subtask branch into the feature branch.
pub(crate) enum MergeBackOutcome {
    /// Subtask branch successfully merged into feature branch.
    Merged,
    /// Subtask had no commits ahead of feature branch.
    NoCommits,
    /// Merge conflict detected; queued in MergeQueue for specialist resolution.
    ConflictQueued,
}

/// Find the root ancestor of a task using a single recursive CTE query.
///
/// Standalone helper for use by both infrastructure.rs and helpers.rs.
/// Falls back to returning `task_id` itself if the query fails.
pub async fn find_root_ancestor_id<T: TaskRepository + ?Sized>(
    task_id: Uuid,
    task_repo: &T,
) -> Uuid {
    task_repo
        .find_root_task_id(task_id)
        .await
        .unwrap_or(task_id)
}

/// Helper function to run post-completion workflow (verification and merging).
/// This is called from spawned tasks after successful task completion.
///
/// For subtasks (tasks with parent_id), this merges the subtask branch back
/// into the root ancestor's feature branch and checks if the entire task tree
/// is ready for a single PR. For standalone tasks (no parent, no children),
/// the existing per-task PR/merge flow is used.
///
/// The `output_delivery` parameter controls how artifacts are delivered:
/// - `OutputDelivery::MemoryOnly` → skip all git operations immediately (agents already
///   persisted findings to memory).
/// - `OutputDelivery::PullRequest` → existing PR-first flow (default).
/// - `OutputDelivery::DirectMerge` → merge without creating a PR.
///
/// Since the [middleware refactor](super::middleware), this function builds
/// a [`PostCompletionContext`](super::middleware::PostCompletionContext) and
/// runs the orchestrator's post-completion middleware chain. The built-in
/// middleware preserve the previous semantics exactly; callers can register
/// additional middleware via
/// [`SwarmOrchestrator::with_post_completion_middleware`].
#[allow(clippy::too_many_arguments)]
pub async fn run_post_completion_workflow(
    task_id: Uuid,
    task_repo: Arc<dyn TaskRepository>,
    goal_repo: Arc<dyn GoalRepository>,
    worktree_repo: Arc<dyn WorktreeRepository>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    event_bus: &Arc<EventBus>,
    audit_log: &Arc<AuditLogService>,
    verify_on_completion: bool,
    use_merge_queue: bool,
    prefer_pull_requests: bool,
    repo_path: &std::path::Path,
    default_base_ref: &str,
    require_commits: bool,
    intent_satisfied: bool,
    output_delivery: OutputDelivery,
    merge_request_repo: Option<Arc<dyn MergeRequestRepository>>,
    fetch_on_sync: bool,
    post_completion_chain: Arc<tokio::sync::RwLock<super::middleware::PostCompletionChain>>,
) -> DomainResult<()> {
    use super::middleware::PostCompletionContext;

    let mut ctx = PostCompletionContext {
        task_id,
        task_repo,
        goal_repo,
        worktree_repo,
        merge_request_repo,
        audit_log: audit_log.clone(),
        event_bus: event_bus.clone(),
        event_tx: event_tx.clone(),
        verify_on_completion,
        use_merge_queue,
        prefer_pull_requests,
        require_commits,
        intent_satisfied,
        output_delivery,
        repo_path: repo_path.to_path_buf(),
        default_base_ref: default_base_ref.to_string(),
        fetch_on_sync,
        verification_passed: false,
        tree_handled: false,
    };

    let chain = post_completion_chain.read().await;
    chain.run(&mut ctx).await
}

/// Merge a subtask's branch into the root ancestor's feature branch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn merge_subtask_into_feature_branch(
    task_id: Uuid,
    task_repo: Arc<dyn TaskRepository>,
    goal_repo: Arc<dyn GoalRepository>,
    worktree_repo: Arc<dyn WorktreeRepository>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    audit_log: &Arc<AuditLogService>,
    repo_path: &std::path::Path,
    default_base_ref: &str,
    merge_request_repo: Option<Arc<dyn MergeRequestRepository>>,
) -> DomainResult<MergeBackOutcome> {
    use tokio::process::Command;

    let subtask_wt = match worktree_repo.get_by_task(task_id).await? {
        Some(wt) => wt,
        None => return Ok(MergeBackOutcome::NoCommits),
    };

    let task = match task_repo.get(task_id).await? {
        Some(t) => t,
        None => return Ok(MergeBackOutcome::NoCommits),
    };

    // If the task has no parent, it IS the root — nothing to merge "back into"
    // a feature branch. Previous code unwrapped and panicked on orphan tasks.
    let root_id = match task.parent_id {
        Some(pid) => find_root_ancestor_id(pid, &*task_repo).await,
        None => {
            tracing::debug!(
                task_id = %task_id,
                "merge_subtask_into_feature_branch: task has no parent (is root) — skipping merge-back"
            );
            return Ok(MergeBackOutcome::NoCommits);
        }
    };
    let root_wt = match worktree_repo.get_by_task(root_id).await? {
        Some(wt) => wt,
        None => return Ok(MergeBackOutcome::NoCommits),
    };

    // Check if subtask has commits ahead of feature branch
    let log_output = Command::new("git")
        .args([
            "log",
            &format!("{}..{}", root_wt.branch, subtask_wt.branch),
            "--oneline",
        ])
        .current_dir(repo_path)
        .output()
        .await;

    let has_commits = match log_output {
        Ok(out) => !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        Err(_) => false,
    };

    if !has_commits {
        return Ok(MergeBackOutcome::NoCommits);
    }

    // Use MergeQueue for the merge (gets conflict handling + specialist integration)
    let mr_repo = match merge_request_repo {
        Some(repo) => repo,
        None => {
            tracing::warn!(task_id = %task_id, "merge_request_repo not available, cannot merge subtask");
            return Err(DomainError::ValidationFailed(
                "MergeRequestRepository not configured".to_string(),
            ));
        }
    };
    let verifier = IntegrationVerifierService::new(
        task_repo.clone(),
        goal_repo.clone(),
        worktree_repo.clone(),
        VerifierConfig::default(),
    );
    let merge_config = MergeQueueConfig {
        repo_path: repo_path.to_str().unwrap_or(".").to_string(),
        main_branch: default_base_ref.to_string(),
        require_verification: false, // verification already ran above
        route_conflicts_to_specialist: true,
        ..Default::default()
    };
    let merge_queue = MergeQueue::new(
        task_repo.clone(),
        worktree_repo.clone(),
        Arc::new(verifier),
        merge_config,
        mr_repo,
    );

    // Queue merge: subtask branch → feature branch, in root's worktree
    merge_queue
        .queue_merge_back(task_id, &subtask_wt.branch, &root_wt.branch, &root_wt.path)
        .await?;

    // Process immediately
    match merge_queue.process_next().await? {
        Some(result) if result.success => {
            let _ = event_tx
                .send(SwarmEvent::SubtaskMergedToFeature {
                    task_id,
                    feature_branch: root_wt.branch.clone(),
                })
                .await;

            audit_log
                .info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!(
                        "Subtask {} merged into feature branch '{}'",
                        task_id, root_wt.branch
                    ),
                )
                .await;

            Ok(MergeBackOutcome::Merged)
        }
        Some(result) if result.had_conflicts => {
            // Conflict detected. The MergeQueue recorded it with status=Conflict.
            // The existing process_merge_conflict_specialists (specialist_triggers.rs)
            // will pick this up on next tick and spawn a specialist.
            audit_log
                .info(
                    AuditCategory::Task,
                    AuditAction::TaskFailed,
                    format!(
                        "Merge conflict merging subtask {} into feature branch: {:?}",
                        task_id, result.conflict_files
                    ),
                )
                .await;

            Ok(MergeBackOutcome::ConflictQueued)
        }
        Some(result) => {
            // Non-conflict merge failure
            Err(DomainError::ExecutionFailed(
                result
                    .error
                    .unwrap_or_else(|| "Unknown merge failure".to_string()),
            ))
        }
        None => Ok(MergeBackOutcome::NoCommits),
    }
}

/// Per-repo-path auto-ship locks.
///
/// Keyed by the canonical repository path so that auto-ship operations
/// targeting the same working tree serialize (required — they share a git
/// index), while ships targeting different repos can run in parallel.
static SHIP_LOCKS: tokio::sync::OnceCell<
    tokio::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Arc<tokio::sync::Mutex<()>>>>,
> = tokio::sync::OnceCell::const_new();

/// Get (or lazily create) the auto-ship mutex for a specific repo path.
///
/// The returned `Arc<Mutex<()>>` is cached in the global map, so subsequent
/// calls with the same (canonicalized) path return the same mutex instance.
async fn ship_lock_for(repo_path: &Path) -> Arc<tokio::sync::Mutex<()>> {
    let map = SHIP_LOCKS
        .get_or_init(|| async { tokio::sync::Mutex::new(std::collections::HashMap::new()) })
        .await;
    let canonical = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut guard = map.lock().await;
    guard
        .entry(canonical)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Check if all tasks in a tree are terminal and, if so, create a single PR.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn try_auto_ship(
    triggering_task_id: Uuid,
    task_repo: Arc<dyn TaskRepository>,
    worktree_repo: Arc<dyn WorktreeRepository>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    audit_log: &Arc<AuditLogService>,
    repo_path: &std::path::Path,
    default_base_ref: &str,
    output_delivery: OutputDelivery,
    fetch_on_sync: bool,
) -> Option<String> {
    // Serialize auto-ship operations *per repo path*. The git checkout → merge --squash → commit
    // sequence operates on the shared repo working directory and must not run concurrently for
    // the same repo; but ships targeting different repos are independent and may run in parallel.
    let lock = ship_lock_for(repo_path).await;
    let _guard = lock.lock().await;

    // Pre-flight: ensure the shared working directory is clean before starting.
    // A previous auto-ship may have left dirty state if its cleanup was incomplete.
    if !ensure_clean_working_dir(repo_path).await {
        tracing::error!(
            triggering_task_id = %triggering_task_id,
            "try_auto_ship: unable to recover clean working directory — aborting"
        );
        return None;
    }

    if output_delivery == OutputDelivery::MemoryOnly {
        tracing::debug!(
            triggering_task_id = %triggering_task_id,
            "try_auto_ship skipped — OutputDelivery::MemoryOnly"
        );
        return None;
    }

    let root_id = find_root_ancestor_id(triggering_task_id, &*task_repo).await;
    let mut root_task = task_repo.get(root_id).await.ok()??;

    // Fix 3: Recover root tasks stuck in Validating with inconsistent workflow state
    if root_task.status == TaskStatus::Validating {
        use crate::domain::models::workflow_state::WorkflowState;

        let workflow_state = root_task.workflow_state();

        match &workflow_state {
            Some(WorkflowState::Completed { .. }) => {
                tracing::warn!(
                    root_task_id = %root_id,
                    "try_auto_ship: recovering deadlock — Validating+Completed, force-completing root task"
                );
                let _ = root_task.transition_to(TaskStatus::Complete);
                let _ = task_repo.update(&root_task).await;
                // Reload after update
                root_task = task_repo.get(root_id).await.ok()??;
            }
            Some(WorkflowState::Failed { .. }) | Some(WorkflowState::Rejected { .. }) => {
                tracing::warn!(
                    root_task_id = %root_id,
                    workflow_state = ?workflow_state,
                    "try_auto_ship: recovering deadlock — Validating+Failed/Rejected, force-failing root task"
                );
                let _ = root_task.transition_to(TaskStatus::Failed);
                let _ = task_repo.update(&root_task).await;
                root_task = task_repo.get(root_id).await.ok()??;
            }
            Some(WorkflowState::PhaseReady { .. }) => {
                tracing::warn!(
                    root_task_id = %root_id,
                    "try_auto_ship: recovering deadlock — Validating+PhaseReady, force-failing root task"
                );
                let _ = root_task.transition_to(TaskStatus::Failed);
                let _ = task_repo.update(&root_task).await;
                root_task = task_repo.get(root_id).await.ok()??;
            }
            Some(WorkflowState::Verifying { .. }) => {
                // Verification genuinely in progress — do not interfere
                return None;
            }
            _ => {
                // No workflow state or other active state — let normal validation proceed
                return None;
            }
        }
    }

    // Root must be terminal
    if !root_task.is_terminal() {
        tracing::warn!(
            root_task_id = %root_id,
            triggering_task_id = %triggering_task_id,
            status = ?root_task.status,
            "try_auto_ship called but root task is not terminal"
        );
        return None;
    }

    // Fix 3b: Recover descendant tasks stuck in Validating with inconsistent workflow state
    {
        use crate::domain::models::workflow_state::WorkflowState;

        let mut queue = vec![root_id];
        while let Some(parent_id) = queue.pop() {
            if let Ok(subtasks) = task_repo.get_subtasks(parent_id).await {
                for mut st in subtasks {
                    queue.push(st.id);
                    if st.status != TaskStatus::Validating {
                        continue;
                    }

                    let workflow_state = st.workflow_state();

                    match &workflow_state {
                        Some(WorkflowState::PhaseReady { .. })
                        | Some(WorkflowState::PhaseGate { .. }) => {
                            tracing::warn!(
                                descendant_task_id = %st.id,
                                workflow_state = ?workflow_state,
                                "try_auto_ship: recovering descendant deadlock — Validating+{:?}, force-failing",
                                workflow_state.as_ref().unwrap(),
                            );
                            let _ = st.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&st).await;
                        }
                        Some(WorkflowState::Completed { .. }) => {
                            tracing::warn!(
                                descendant_task_id = %st.id,
                                "try_auto_ship: recovering descendant deadlock — Validating+Completed, force-completing"
                            );
                            let _ = st.transition_to(TaskStatus::Complete);
                            let _ = task_repo.update(&st).await;
                        }
                        Some(WorkflowState::Failed { .. })
                        | Some(WorkflowState::Rejected { .. }) => {
                            tracing::warn!(
                                descendant_task_id = %st.id,
                                workflow_state = ?workflow_state,
                                "try_auto_ship: recovering descendant deadlock — Validating+Failed/Rejected, force-failing"
                            );
                            let _ = st.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&st).await;
                        }
                        Some(WorkflowState::Verifying { .. }) => {
                            // Verification genuinely in progress — leave alone
                        }
                        _ => {
                            // No workflow state or other active state — leave alone
                        }
                    }
                }
            }
        }
    }

    // All descendants must be terminal
    if !all_descendants_terminal(root_id, &*task_repo).await {
        tracing::warn!(
            root_task_id = %root_id,
            triggering_task_id = %triggering_task_id,
            "try_auto_ship called but not all descendants are terminal"
        );
        return None;
    }

    // At least one descendant must have succeeded
    if root_task.status != TaskStatus::Complete
        && !has_any_successful_descendant(root_id, &*task_repo).await
    {
        audit_log
            .info(
                AuditCategory::Task,
                AuditAction::TaskCompleted,
                format!(
                    "All tasks in tree {} terminal but none succeeded - no PR",
                    root_id
                ),
            )
            .await;
        return None;
    }

    // Get root's worktree
    let root_wt = worktree_repo.get_by_task(root_id).await.ok()??;

    // Safety net: collect any descendant work not yet merged into the root branch.
    // This handles cases where individual merge-back was skipped (e.g., convergent
    // tasks) or incomplete due to race conditions.
    collect_descendant_work(
        root_id,
        &root_wt,
        &*task_repo,
        &*worktree_repo,
        audit_log,
        repo_path,
    )
    .await;

    // Sync local base ref with remote before merging so the squash merge
    // targets the latest remote state and the subsequent push is clean.
    if fetch_on_sync {
        use tokio::process::Command as TokioCommand;
        if sync_with_remote(repo_path, default_base_ref).await {
            // Checkout the base branch and fast-forward to match remote
            let checkout_sync = TokioCommand::new("git")
                .args(["checkout", default_base_ref])
                .current_dir(repo_path)
                .output()
                .await;
            match &checkout_sync {
                Ok(o) if !o.status.success() => {
                    tracing::warn!(
                        root_task_id = %root_id,
                        stderr = %String::from_utf8_lossy(&o.stderr),
                        "git checkout {} during sync failed", default_base_ref
                    );
                    ensure_clean_working_dir(repo_path).await;
                    return None;
                }
                Err(e) => {
                    tracing::warn!(root_task_id = %root_id, "git checkout {} during sync failed: {}", default_base_ref, e);
                    ensure_clean_working_dir(repo_path).await;
                    return None;
                }
                _ => {}
            }
            let ff = TokioCommand::new("git")
                .args([
                    "merge",
                    "--ff-only",
                    &format!("origin/{}", default_base_ref),
                ])
                .current_dir(repo_path)
                .output()
                .await;
            match &ff {
                Err(e) => {
                    tracing::warn!(
                        root_task_id = %root_id,
                        error = %e,
                        "git merge --ff-only command failed during sync"
                    );
                }
                Ok(output) if !output.status.success() => {
                    // Local main has diverged from remote (previous auto-ship push failed).
                    // Rebase local commits onto remote to recover.
                    tracing::warn!(
                        root_task_id = %root_id,
                        "local {} diverged from remote — rebasing to recover", default_base_ref
                    );
                    let rebase = TokioCommand::new("git")
                        .args(["rebase", &format!("origin/{}", default_base_ref)])
                        .current_dir(repo_path)
                        .output()
                        .await;
                    match &rebase {
                        Ok(r) if !r.status.success() => {
                            tracing::error!(
                                root_task_id = %root_id,
                                stderr = %String::from_utf8_lossy(&r.stderr),
                                "rebase onto origin/{} failed — auto-ship aborted", default_base_ref
                            );
                            ensure_clean_working_dir(repo_path).await;
                            return None;
                        }
                        Err(e) => {
                            tracing::error!(
                                root_task_id = %root_id,
                                error = %e,
                                "git rebase command failed — auto-ship aborted"
                            );
                            ensure_clean_working_dir(repo_path).await;
                            return None;
                        }
                        _ => {}
                    }
                }
                _ => {} // ff-only succeeded, nothing to do
            }
        }
    }

    // Check feature branch has commits ahead of base
    let has_commits = {
        use tokio::process::Command;
        Command::new("git")
            .args([
                "log",
                &format!("{}..{}", default_base_ref, root_wt.branch),
                "--oneline",
            ])
            .current_dir(repo_path)
            .output()
            .await
            .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
            .unwrap_or(false)
    };

    if !has_commits {
        tracing::warn!(
            root_task_id = %root_id,
            branch = %root_wt.branch,
            base_ref = %default_base_ref,
            "Completed task tree has no commits to ship — feature branch has no commits ahead of base"
        );
        audit_log
            .info(
                AuditCategory::Task,
                AuditAction::TaskCompleted,
                format!(
                    "Feature branch {} has no commits ahead of {} - no PR",
                    root_wt.branch, default_base_ref
                ),
            )
            .await;
        let _ = event_tx.send(SwarmEvent::TaskFailed {
            task_id: root_id,
            error: format!(
                "Task tree completed but produced no commits (branch {} has nothing ahead of {})",
                root_wt.branch, default_base_ref,
            ),
            retry_count: 0,
        }).await;
        return None;
    }

    // Build squash commit message from task metadata
    use tokio::process::Command as TokioCommand;

    let squash_msg = {
        let title = root_task.title.trim();
        let desc = root_task.description.trim();
        if desc.is_empty() || desc == title {
            format!("{}\n\nTask-Id: {}", title, root_id)
        } else {
            // Truncate long descriptions to keep commit messages reasonable
            let short_desc = if desc.len() > 500 {
                let truncated: String = desc.chars().take(497).collect();
                format!("{}...", truncated)
            } else {
                desc.to_string()
            };
            format!("{}\n\n{}\n\nTask-Id: {}", title, short_desc, root_id)
        }
    };

    // Ensure we're on the base branch before merging
    let checkout_output = TokioCommand::new("git")
        .args(["checkout", default_base_ref])
        .current_dir(repo_path)
        .output()
        .await;

    match &checkout_output {
        Err(e) => {
            tracing::warn!(root_task_id = %root_id, "git checkout {} failed: {}", default_base_ref, e);
            ensure_clean_working_dir(repo_path).await;
            return None;
        }
        Ok(o) if !o.status.success() => {
            tracing::warn!(
                root_task_id = %root_id,
                stderr = %String::from_utf8_lossy(&o.stderr),
                "git checkout {} failed", default_base_ref
            );
            ensure_clean_working_dir(repo_path).await;
            return None;
        }
        _ => {}
    }

    // Squash merge: stages all changes from the feature branch without committing
    let squash_output = TokioCommand::new("git")
        .args(["merge", "--squash", &root_wt.branch])
        .current_dir(repo_path)
        .output()
        .await;

    match squash_output {
        Ok(output) if output.status.success() => {
            // --squash stages changes but does not commit; we commit below
        }
        Ok(output) => {
            tracing::warn!(
                root_task_id = %root_id,
                branch = %root_wt.branch,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "git merge --squash failed for auto-ship"
            );
            ensure_clean_working_dir(repo_path).await;
            return None;
        }
        Err(e) => {
            tracing::warn!(root_task_id = %root_id, "git merge --squash command failed: {}", e);
            ensure_clean_working_dir(repo_path).await;
            return None;
        }
    }

    // Commit the squashed changes with our crafted message
    let commit_output = TokioCommand::new("git")
        .args(["commit", "-m", &squash_msg])
        .current_dir(repo_path)
        .output()
        .await;

    let commit_sha = match commit_output {
        Ok(output) if output.status.success() => TokioCommand::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default(),
        Ok(output) => {
            tracing::warn!(
                root_task_id = %root_id,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "git commit after squash merge failed"
            );
            ensure_clean_working_dir(repo_path).await;
            return None;
        }
        Err(e) => {
            tracing::warn!(root_task_id = %root_id, "git commit command failed: {}", e);
            ensure_clean_working_dir(repo_path).await;
            return None;
        }
    };

    // Push merged base ref to remote so subsequent auto-ships and worktree
    // creations see the latest state.
    if fetch_on_sync && !push_with_retry(repo_path, default_base_ref, 2).await {
        tracing::error!(
            root_task_id = %root_id,
            commit_sha = %commit_sha,
            "auto-ship committed locally but push to remote failed — \
             next auto-ship will attempt to rebase and push"
        );
        // Don't return None — the local commit is valid and the push
        // will be retried when the next auto-ship syncs.
    }

    let _ = event_tx
        .send(SwarmEvent::TaskMerged {
            task_id: root_id,
            commit_sha: commit_sha.clone(),
        })
        .await;

    audit_log
        .info(
            AuditCategory::Task,
            AuditAction::TaskCompleted,
            format!(
                "Task tree {} merged to {}: {}",
                root_id, default_base_ref, commit_sha
            ),
        )
        .await;

    Some(commit_sha)
}

/// BFS-walk the task tree from root and merge any unmerged descendant branches
/// into the root's worktree. This is a safety net to collect child work that
/// wasn't merged back during individual subtask completion (e.g., convergent
/// tasks where intent_satisfied skipped merge-back, or race conditions).
async fn collect_descendant_work<T: TaskRepository + ?Sized, W: WorktreeRepository + ?Sized>(
    root_id: Uuid,
    root_wt: &crate::domain::models::Worktree,
    task_repo: &T,
    worktree_repo: &W,
    audit_log: &Arc<AuditLogService>,
    repo_path: &std::path::Path,
) {
    use tokio::process::Command;

    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        let subtasks = match task_repo.get_subtasks(id).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        for st in &subtasks {
            queue.push(st.id);

            // Skip if subtask has no worktree
            let desc_wt = match worktree_repo.get_by_task(st.id).await {
                Ok(Some(wt)) => wt,
                _ => continue,
            };

            // Skip if already merged
            if desc_wt.status == WorktreeStatus::Merged {
                continue;
            }

            // Check if descendant has commits ahead of root's branch
            let has_commits = Command::new("git")
                .args([
                    "log",
                    &format!("{}..{}", root_wt.branch, desc_wt.branch),
                    "--oneline",
                ])
                .current_dir(repo_path)
                .output()
                .await
                .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
                .unwrap_or(false);

            if !has_commits {
                continue;
            }

            // Merge descendant into root's worktree
            let merge_result = Command::new("git")
                .args([
                    "merge",
                    "--no-ff",
                    &desc_wt.branch,
                    "-m",
                    &format!(
                        "Merge descendant {} ({}) into feature branch",
                        &st.id.to_string()[..8],
                        st.title
                    ),
                ])
                .current_dir(&root_wt.path)
                .output()
                .await;

            match merge_result {
                Ok(output) if output.status.success() => {
                    audit_log
                        .info(
                            AuditCategory::Task,
                            AuditAction::TaskCompleted,
                            format!(
                                "collect_descendant_work: merged {} ({}) into feature branch '{}'",
                                st.id, desc_wt.branch, root_wt.branch
                            ),
                        )
                        .await;

                    // Mark worktree as merged
                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(st.id).await {
                        wt.merged("collected-by-auto-ship".to_string());
                        let _ = worktree_repo.update(&wt).await;
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(
                        "collect_descendant_work: merge of {} into '{}' failed: {}",
                        st.id,
                        root_wt.branch,
                        stderr
                    );
                    if !ensure_clean_working_dir(Path::new(&root_wt.path)).await {
                        tracing::error!(
                            task_id = %st.id,
                            worktree = %root_wt.path,
                            "collect_descendant_work: failed to clean worktree after merge failure — stopping"
                        );
                        return;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "collect_descendant_work: failed to run merge for {}: {}",
                        st.id,
                        e
                    );
                    if !ensure_clean_working_dir(Path::new(&root_wt.path)).await {
                        tracing::error!(
                            task_id = %st.id,
                            worktree = %root_wt.path,
                            "collect_descendant_work: failed to clean worktree after command failure — stopping"
                        );
                        return;
                    }
                }
            }
        }
    }
}

/// BFS check: all descendants in terminal state.
async fn all_descendants_terminal<T: TaskRepository + ?Sized>(
    root_id: Uuid,
    task_repo: &T,
) -> bool {
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        let subtasks = match task_repo.get_subtasks(id).await {
            Ok(s) => s,
            Err(_) => return false,
        };
        for st in subtasks {
            if !st.is_terminal() {
                return false;
            }
            queue.push(st.id);
        }
    }
    true
}

/// BFS check: any descendant completed successfully.
async fn has_any_successful_descendant<T: TaskRepository + ?Sized>(
    root_id: Uuid,
    task_repo: &T,
) -> bool {
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        if let Ok(subtasks) = task_repo.get_subtasks(id).await {
            for st in &subtasks {
                if st.status == TaskStatus::Complete {
                    return true;
                }
                queue.push(st.id);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_description_unchanged() {
        let desc = "Fix the login button color";
        assert_eq!(truncate_description(desc, 300), desc);
    }

    #[test]
    fn truncate_long_description_to_first_paragraph() {
        let desc = "Fix the login button color to match the design system.\n\n\
                     This is a longer explanation that goes into detail about \
                     the specific CSS changes needed, the design tokens involved, \
                     and the testing strategy for visual regression. It also covers \
                     edge cases for dark mode and high contrast accessibility themes.";
        let result = truncate_description(desc, 300);
        assert!(result.len() <= 300);
        assert!(!result.contains("longer explanation"));
        assert!(result.contains("Fix the login button color"));
    }

    #[test]
    fn truncate_description_with_markdown_headers() {
        let desc = "# Convergence Check\n## System Prompt\nActual task content here.\n\n\
                     A huge block of system prompt text that goes on and on \
                     and should never appear in a PR description because it is \
                     the internal plumbing of the convergence engine and not \
                     relevant to reviewers at all. "
            .repeat(5);
        let result = truncate_description(&desc, 300);
        assert!(result.len() <= 303); // 300 + "..."
        assert!(!result.contains("# Convergence Check"));
        assert!(!result.contains("## System Prompt"));
        assert!(result.contains("Actual task content here"));
    }

    #[test]
    fn truncate_description_with_separator() {
        let desc = "Implement the new caching layer\n---\nThis is below the separator \
                     and contains a very long detailed specification that should not \
                     appear in the PR description.";
        let result = truncate_description(desc, 300);
        assert!(result.contains("Implement the new caching layer"));
        // Note: the `---` split only triggers on `\n---\n`, so this single-`\n`
        // separated case keeps both parts (but truncates if over max_chars).
    }

    #[test]
    fn truncate_description_with_blank_line_separator() {
        let long_second = "x".repeat(500);
        let desc = format!("Short intent summary\n\n{}", long_second);
        let result = truncate_description(&desc, 300);
        assert_eq!(result, "Short intent summary");
    }

    #[test]
    fn truncate_empty_description() {
        assert_eq!(truncate_description("", 300), "");
    }

    #[test]
    fn truncate_all_headers_falls_back_to_first_content_line() {
        // Must exceed max_chars to trigger truncation logic
        let long_body = "x".repeat(400);
        let desc = format!(
            "# Header One\n## Header Two\n### Header Three\n\n{}",
            long_body
        );
        let result = truncate_description(&desc, 300);
        // First paragraph is all headers; after filtering, it's empty.
        // Falls back to first non-header, non-empty line from the full description.
        assert!(!result.starts_with('#'));
    }

    #[test]
    fn remove_transient_artifacts_deletes_known_files_preserves_readme() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // Create some transient artifacts
        std::fs::write(base.join("PLAN.md"), "scratch").unwrap();
        std::fs::write(base.join("REVIEW.md"), "scratch").unwrap();
        std::fs::write(base.join("NOTES.md"), "scratch").unwrap();
        // Create a legitimate file that must NOT be removed
        std::fs::write(base.join("README.md"), "keep me").unwrap();

        let removed = remove_transient_artifacts(base.to_str().unwrap());

        assert!(removed.contains(&"PLAN.md".to_string()));
        assert!(removed.contains(&"REVIEW.md".to_string()));
        assert!(removed.contains(&"NOTES.md".to_string()));
        assert_eq!(removed.len(), 3);

        // Transient files should be gone
        assert!(!base.join("PLAN.md").exists());
        assert!(!base.join("REVIEW.md").exists());
        assert!(!base.join("NOTES.md").exists());
        // README.md must still exist
        assert!(base.join("README.md").exists());
    }

    #[test]
    fn remove_transient_artifacts_noop_when_no_files_exist() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let removed = remove_transient_artifacts(dir.path().to_str().unwrap());
        assert!(removed.is_empty());
    }

    #[test]
    fn is_rejection_error_detects_common_reject_phrases() {
        // Typical non-fast-forward rejection from `git push`
        let stderr = " ! [rejected]        main -> main (non-fast-forward)\n\
                      error: failed to push some refs to 'origin'\n\
                      hint: Updates were rejected because the tip of your current branch is behind\n\
                      hint: its remote counterpart. Integrate the remote changes (e.g.\n\
                      hint: 'git pull ...') before pushing again.\n";
        assert!(is_rejection_error(stderr));

        // "fetch first" variant (when remote has commits we don't)
        let stderr2 = " ! [rejected]  main -> main (fetch first)\n";
        assert!(is_rejection_error(stderr2));

        // Case-insensitive match
        assert!(is_rejection_error("REJECTED: NON-FAST-FORWARD"));
    }

    #[test]
    fn is_rejection_error_ignores_non_rejection_failures() {
        // Auth failure — retrying with fetch+rebase won't help
        let auth = "fatal: Authentication failed for 'https://github.com/foo/bar.git/'\n";
        assert!(!is_rejection_error(auth));

        // Network failure
        let net = "fatal: unable to access 'https://github.com/foo/bar.git/': Could not resolve host: github.com\n";
        assert!(!is_rejection_error(net));

        // Repo config error (missing remote etc.)
        let cfg = "fatal: 'origin' does not appear to be a git repository\nfatal: Could not read from remote repository.\n";
        assert!(!is_rejection_error(cfg));

        // Empty stderr
        assert!(!is_rejection_error(""));
    }

    /// Helper: create a temp git repo with an initial commit.
    fn init_test_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(p)
            .output()
            .unwrap();
        std::fs::write(p.join("file.txt"), "initial").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(p)
            .output()
            .unwrap();
        dir
    }

    /// Helper: check `git status --porcelain` is empty.
    fn is_clean(path: &Path) -> bool {
        let out = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().is_empty()
    }

    #[tokio::test]
    async fn ensure_clean_recovers_from_unstaged_changes() {
        let dir = init_test_repo();
        let p = dir.path();
        std::fs::write(p.join("file.txt"), "modified").unwrap();
        assert!(!is_clean(p));

        let result = ensure_clean_working_dir(p).await;
        assert!(result);
        assert!(is_clean(p));
    }

    #[tokio::test]
    async fn ensure_clean_recovers_from_staged_changes() {
        let dir = init_test_repo();
        let p = dir.path();
        std::fs::write(p.join("file.txt"), "modified").unwrap();
        std::process::Command::new("git")
            .args(["add", "file.txt"])
            .current_dir(p)
            .output()
            .unwrap();
        assert!(!is_clean(p));

        let result = ensure_clean_working_dir(p).await;
        assert!(result);
        assert!(is_clean(p));
    }

    #[tokio::test]
    async fn ensure_clean_recovers_from_merge_conflict() {
        let dir = init_test_repo();
        let p = dir.path();

        // Create a branch with conflicting changes
        std::process::Command::new("git")
            .args(["checkout", "-b", "conflict-branch"])
            .current_dir(p)
            .output()
            .unwrap();
        std::fs::write(p.join("file.txt"), "branch-content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "branch change"])
            .current_dir(p)
            .output()
            .unwrap();

        // Go back to main and make conflicting change
        std::process::Command::new("git")
            .args(["checkout", "master"])
            .current_dir(p)
            .output()
            .unwrap()
            .status
            .success()
            .then_some(())
            .or_else(|| {
                std::process::Command::new("git")
                    .args(["checkout", "main"])
                    .current_dir(p)
                    .output()
                    .unwrap()
                    .status
                    .success()
                    .then_some(())
            });
        std::fs::write(p.join("file.txt"), "main-content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "main change"])
            .current_dir(p)
            .output()
            .unwrap();

        // Try to merge — this should conflict
        let merge = std::process::Command::new("git")
            .args(["merge", "conflict-branch"])
            .current_dir(p)
            .output()
            .unwrap();
        assert!(!merge.status.success());
        assert!(!is_clean(p));

        let result = ensure_clean_working_dir(p).await;
        assert!(result);
        assert!(is_clean(p));
    }

    #[tokio::test]
    async fn ensure_clean_noop_on_clean_repo() {
        let dir = init_test_repo();
        let p = dir.path();
        assert!(is_clean(p));

        let result = ensure_clean_working_dir(p).await;
        assert!(result);
        assert!(is_clean(p));
    }

    #[tokio::test]
    async fn ensure_clean_recovers_from_in_progress_rebase() {
        let dir = init_test_repo();
        let p = dir.path();

        // Determine the default branch name (master or main)
        let branch_output = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(p)
            .output()
            .unwrap();
        let default_branch = String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_string();

        // Create a branch with a conflicting change
        std::process::Command::new("git")
            .args(["checkout", "-b", "rebase-branch"])
            .current_dir(p)
            .output()
            .unwrap();
        std::fs::write(p.join("file.txt"), "rebase-branch-content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "rebase branch change"])
            .current_dir(p)
            .output()
            .unwrap();

        // Go back to default branch and make a conflicting change
        let checkout = std::process::Command::new("git")
            .args(["checkout", &default_branch])
            .current_dir(p)
            .output()
            .unwrap();
        assert!(
            checkout.status.success(),
            "failed to checkout default branch"
        );
        std::fs::write(p.join("file.txt"), "main-content-for-rebase").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(p)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "main change for rebase"])
            .current_dir(p)
            .output()
            .unwrap();

        // Switch to the rebase branch and try to rebase onto default — this should conflict
        std::process::Command::new("git")
            .args(["checkout", "rebase-branch"])
            .current_dir(p)
            .output()
            .unwrap();
        let rebase = std::process::Command::new("git")
            .args(["rebase", &default_branch])
            .current_dir(p)
            .output()
            .unwrap();
        assert!(!rebase.status.success(), "expected rebase conflict");

        // Verify we are in a rebase state (rebase-merge or rebase-apply dir exists)
        let git_dir = p.join(".git");
        assert!(
            git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists(),
            "expected rebase-merge or rebase-apply directory to exist"
        );

        // ensure_clean_working_dir should abort the rebase and clean up
        let result = ensure_clean_working_dir(p).await;
        assert!(result);
        assert!(is_clean(p));

        // Verify the rebase state directories are gone
        assert!(
            !git_dir.join("rebase-merge").exists() && !git_dir.join("rebase-apply").exists(),
            "rebase state directories should be cleaned up"
        );
    }

    #[tokio::test]
    async fn ship_lock_for_same_path_serializes() {
        // Two concurrent ship_lock_for() calls against the same path must return
        // locks that serialize — i.e. while task A holds the guard, task B's
        // attempt to acquire blocks and should time out.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Acquire the lock for `path` and hold it inside a spawned task.
        let path_a = path.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let holder = tokio::spawn(async move {
            let lock = ship_lock_for(&path_a).await;
            let _guard = lock.lock().await;
            // Signal that we're holding the lock.
            let _ = ready_tx.send(());
            // Wait until the test tells us to release.
            let _ = release_rx.await;
        });

        // Wait until the holder task has acquired the lock.
        ready_rx.await.unwrap();

        // Now try to acquire the same-path lock from another task; it must block.
        let path_b = path.clone();
        let contender = tokio::spawn(async move {
            let lock = ship_lock_for(&path_b).await;
            let _guard = lock.lock().await;
        });

        // The contender should NOT finish while the holder still owns the guard.
        let contention = tokio::time::timeout(std::time::Duration::from_millis(200), async {
            contender.await.unwrap();
        })
        .await;
        assert!(
            contention.is_err(),
            "contender acquired same-path lock while holder was still active"
        );

        // Release the holder; contender should then be able to acquire.
        let _ = release_tx.send(());
        holder.await.unwrap();

        // Now re-acquire on a fresh contender to confirm liveness (the earlier
        // contender future was dropped by `timeout` so we spawn a new one).
        let path_c = path.clone();
        let follow_up = tokio::spawn(async move {
            let lock = ship_lock_for(&path_c).await;
            let _guard = lock.lock().await;
        });
        tokio::time::timeout(std::time::Duration::from_millis(500), follow_up)
            .await
            .expect("follow-up acquire should succeed after holder released")
            .unwrap();
    }

    #[tokio::test]
    async fn ship_lock_for_different_paths_are_independent() {
        // Two different paths should yield two distinct Arc<Mutex<()>> instances,
        // and both locks should be holdable concurrently without blocking.
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let path_a = dir_a.path().to_path_buf();
        let path_b = dir_b.path().to_path_buf();

        let lock_a = ship_lock_for(&path_a).await;
        let lock_b = ship_lock_for(&path_b).await;

        // Different paths ⇒ different Arc instances (pointer inequality).
        assert!(
            !Arc::ptr_eq(&lock_a, &lock_b),
            "distinct repo paths must produce distinct lock instances"
        );

        // Hold lock_a; lock_b acquire should still complete well within a short timeout.
        let _guard_a = lock_a.lock().await;

        let lock_b_for_task = lock_b.clone();
        let acquired = tokio::time::timeout(std::time::Duration::from_millis(500), async move {
            let _guard_b = lock_b_for_task.lock().await;
        })
        .await;
        assert!(
            acquired.is_ok(),
            "different-path lock must be acquirable while another path's lock is held"
        );

        // And ship_lock_for(same path) returns the same instance (pointer equality).
        let lock_a_again = ship_lock_for(&path_a).await;
        assert!(
            Arc::ptr_eq(&lock_a, &lock_a_again),
            "same repo path must return the cached lock instance"
        );
    }

    // -------------------------------------------------------------------------
    // find_root_ancestor_id — root / orphan / chain handling
    //
    // Regression guard for the unwrap-on-None panic previously in
    // merge_subtask_into_feature_branch: an orphan (parent_id = None) task
    // must not panic when callers walk the ancestor chain. Also verifies that
    // a 3-level chain traverses to the root.
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn find_root_ancestor_of_root_task_returns_self() {
        use crate::adapters::sqlite::{SqliteTaskRepository, create_migrated_test_pool};
        use crate::domain::models::Task;

        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = SqliteTaskRepository::new(pool);

        // Root task: parent_id = None.
        let root = Task::new("root task");
        assert!(root.parent_id.is_none());
        task_repo.create(&root).await.unwrap();

        // Must return the task's own id, not panic, not error.
        let resolved = find_root_ancestor_id(root.id, &task_repo).await;
        assert_eq!(
            resolved, root.id,
            "root-task (parent_id=None) must resolve to its own id"
        );
    }

    #[tokio::test]
    async fn find_root_ancestor_of_three_level_chain_returns_root() {
        use crate::adapters::sqlite::{SqliteTaskRepository, create_migrated_test_pool};
        use crate::domain::models::Task;

        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = SqliteTaskRepository::new(pool);

        // Build: root → mid → leaf
        let root = Task::new("root");
        let mut mid = Task::new("mid");
        mid.parent_id = Some(root.id);
        let mut leaf = Task::new("leaf");
        leaf.parent_id = Some(mid.id);

        task_repo.create(&root).await.unwrap();
        task_repo.create(&mid).await.unwrap();
        task_repo.create(&leaf).await.unwrap();

        // All three must resolve to `root.id`.
        assert_eq!(find_root_ancestor_id(leaf.id, &task_repo).await, root.id);
        assert_eq!(find_root_ancestor_id(mid.id, &task_repo).await, root.id);
        assert_eq!(find_root_ancestor_id(root.id, &task_repo).await, root.id);
    }

    #[tokio::test]
    async fn find_root_ancestor_falls_back_to_task_id_on_missing_task() {
        // Missing task: the repo returns an error from find_root_task_id;
        // the helper wraps unwrap_or(task_id) so callers never see a panic
        // even if the DB lookup fails. Guards against reintroducing an
        // unwrap() on the repository result.
        use crate::adapters::sqlite::{SqliteTaskRepository, create_migrated_test_pool};

        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = SqliteTaskRepository::new(pool);

        let bogus = Uuid::new_v4();
        assert_eq!(find_root_ancestor_id(bogus, &task_repo).await, bogus);
    }

    // -------------------------------------------------------------------------
    // auto_commit_worktree — exercises the rewritten `git add -A` error match.
    //
    // The old `add.is_err() || !add.unwrap().status.success()` form panicked
    // on Err (||'s short-circuit requires `true`, so the Err branch still
    // ran .unwrap()). These tests drive the function through success and
    // no-op paths end-to-end to guarantee the explicit-match rewrite stays
    // panic-free under real git invocations.
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn auto_commit_worktree_returns_false_on_clean_repo() {
        let dir = init_test_repo();
        let p = dir.path();

        let result = auto_commit_worktree(p.to_str().unwrap(), Uuid::new_v4()).await;
        assert!(!result, "clean repo must not produce a commit");
    }

    #[tokio::test]
    async fn auto_commit_worktree_commits_uncommitted_changes() {
        let dir = init_test_repo();
        let p = dir.path();

        // Introduce an uncommitted change.
        std::fs::write(p.join("file.txt"), "dirty").unwrap();
        assert!(!is_clean(p));

        let result = auto_commit_worktree(p.to_str().unwrap(), Uuid::new_v4()).await;
        assert!(result, "expected auto_commit_worktree to create a commit");
        assert!(
            is_clean(p),
            "working tree should be clean after auto-commit"
        );
    }

    #[tokio::test]
    async fn auto_commit_worktree_removes_transient_artifacts_before_commit() {
        // Transient artifacts (PLAN.md, REVIEW.md, …) must be stripped prior
        // to auto-commit — we don't want agent scratch surviving into main.
        let dir = init_test_repo();
        let p = dir.path();

        std::fs::write(p.join("PLAN.md"), "scratch").unwrap();
        std::fs::write(p.join("real.txt"), "real work").unwrap();

        let result = auto_commit_worktree(p.to_str().unwrap(), Uuid::new_v4()).await;
        assert!(result, "expected a commit for the real file");
        assert!(!p.join("PLAN.md").exists(), "transient PLAN.md must be removed");
        assert!(p.join("real.txt").exists(), "legitimate file must remain");
        assert!(is_clean(p));
    }

    #[tokio::test]
    async fn auto_commit_worktree_returns_false_on_non_git_path() {
        // Non-git directory: git status prints nothing to stdout, so the
        // early-return "empty status" branch kicks in and we return false
        // without panicking. Guards against reintroducing an unwrap on the
        // Command result.
        let dir = tempfile::tempdir().unwrap();
        let result = auto_commit_worktree(dir.path().to_str().unwrap(), Uuid::new_v4()).await;
        assert!(!result);
    }
}

//! Helper functions for the swarm orchestrator.
//!
//! Top-level utility functions used by spawned tasks that don't have access
//! to the orchestrator instance (e.g., auto-commit, post-completion workflow).

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{TaskStatus, WorktreeStatus};
use crate::domain::models::workflow_template::OutputDelivery;
use crate::domain::ports::{GoalRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, AuditLogService,
    IntegrationVerifierService, MergeQueue, MergeQueueConfig, VerifierConfig,
};

use super::types::SwarmEvent;

/// Auto-commit any uncommitted changes in a worktree as a safety net.
/// Returns true if a commit was made, false if the worktree was clean.
pub async fn auto_commit_worktree(worktree_path: &str, task_id: Uuid) -> bool {
    use tokio::process::Command;

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

    // Stage all changes
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree_path)
        .output()
        .await;

    if add.is_err() || !add.unwrap().status.success() {
        return false;
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
            "pr", "create",
            "--title", task_title,
            "--body", task_description,
            "--base", default_base_ref,
            "--head", branch,
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
async fn summarize_branch_changes(
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
fn truncate_description(description: &str, max_chars: usize) -> String {
    if description.len() <= max_chars {
        return description.to_string();
    }

    // Try to find the first paragraph (before the first blank line or markdown separator)
    let first_para = description
        .split("\n\n")
        .next()
        .unwrap_or(description);

    // Also try splitting on markdown horizontal rules
    let first_section = first_para
        .split("\n---\n")
        .next()
        .unwrap_or(first_para);

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
enum MergeBackOutcome {
    /// Subtask branch successfully merged into feature branch.
    Merged,
    /// Subtask had no commits ahead of feature branch.
    NoCommits,
    /// Merge conflict detected; queued in MergeQueue for specialist resolution.
    ConflictQueued,
}

/// Walk up the parent_id chain to find the root ancestor task.
/// Standalone helper for use by both infrastructure.rs and helpers.rs.
pub async fn find_root_ancestor_id<T: TaskRepository>(task_id: Uuid, task_repo: &T) -> Uuid {
    let mut current = task_id;
    for _ in 0..50 {
        match task_repo.get(current).await {
            Ok(Some(task)) => match task.parent_id {
                Some(pid) => current = pid,
                None => return current,
            },
            _ => return current,
        }
    }
    current
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
pub async fn run_post_completion_workflow<G, T, W>(
    task_id: Uuid,
    task_repo: Arc<T>,
    goal_repo: Arc<G>,
    worktree_repo: Arc<W>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    audit_log: &Arc<AuditLogService>,
    verify_on_completion: bool,
    use_merge_queue: bool,
    prefer_pull_requests: bool,
    repo_path: &std::path::Path,
    default_base_ref: &str,
    require_commits: bool,
    intent_satisfied: bool,
    output_delivery: OutputDelivery,
) -> DomainResult<()>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    // Dispatch based on output delivery mode.
    // MemoryOnly workflows have already persisted their findings to swarm memory;
    // no git operations are needed or appropriate.
    match &output_delivery {
        OutputDelivery::MemoryOnly => {
            tracing::debug!(
                task_id = %task_id,
                "OutputDelivery::MemoryOnly — skipping git post-completion workflow"
            );
            return Ok(());
        }
        OutputDelivery::PullRequest | OutputDelivery::DirectMerge => {
            // Continue with normal PR/merge flow below.
        }
    }

    // When the convergence engine has verified intent satisfaction (e.g., "remove dead code"
    // where the code is already clean), skip the commits gate so the task can complete
    // successfully without producing any commits.
    let require_commits_for_verification = require_commits && !intent_satisfied;
    if intent_satisfied && require_commits {
        tracing::info!(
            task_id = %task_id,
            "Intent verified as satisfied — overriding require_commits to false"
        );
    }

    // Step 1: Run lightweight verification if enabled (no code checks - those happen at merge time)
    let verification_passed = if verify_on_completion {
        let verifier = IntegrationVerifierService::new(
            task_repo.clone(),
            goal_repo.clone(),
            worktree_repo.clone(),
            VerifierConfig {
                run_tests: false,
                run_lint: false,
                check_format: false,
                require_commits: require_commits_for_verification,
                ..VerifierConfig::default()
            },
        );

        match verifier.verify_task(task_id).await {
            Ok(result) => {
                let checks_total = result.checks.len();
                let checks_passed = result.checks.iter().filter(|c| c.passed).count();

                let _ = event_tx.send(SwarmEvent::TaskVerified {
                    task_id,
                    passed: result.passed,
                    checks_passed,
                    checks_total,
                    failures_summary: result.failures_summary.clone(),
                }).await;

                if result.passed {
                    // Transition Validating -> Complete
                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            let _ = task.transition_to(TaskStatus::Complete);
                            let _ = task_repo.update(&task).await;
                        }
                    }

                    audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCompleted,
                        format!(
                            "Task {} passed verification: {}/{} checks",
                            task_id, checks_passed, checks_total
                        ),
                    ).await;
                } else if intent_satisfied {
                    // Intent verifier has confirmed satisfaction — integration
                    // verifier failures are advisory, not blocking.
                    tracing::warn!(
                        task_id = %task_id,
                        failures = ?result.failures_summary,
                        "Integration verifier failed but intent is satisfied — \
                         proceeding to Complete (advisory mode)"
                    );

                    // Transition Validating -> Complete despite integration failure
                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            let _ = task.transition_to(TaskStatus::Complete);
                            let _ = task_repo.update(&task).await;
                        }
                    }

                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskCompleted,
                            AuditActor::System,
                            format!(
                                "Task {} completed (intent satisfied) despite integration \
                                 verifier failures: {}",
                                task_id,
                                result.failures_summary.clone().unwrap_or_default()
                            ),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                } else {
                    // Transition Validating -> Failed and increment retry_count
                    let retry_count = if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            task.retry_count += 1;
                            let _ = task.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&task).await;
                        }
                        task.retry_count
                    } else {
                        0
                    };

                    // Emit TaskFailed event so handlers (retry, evolution, review-failure-loop) can react
                    let failure_msg = format!(
                        "verification-failed: {}",
                        result.failures_summary.clone().unwrap_or_default()
                    );
                    let _ = event_tx.send(SwarmEvent::TaskFailed {
                        task_id,
                        error: failure_msg,
                        retry_count,
                    }).await;

                    // Also mark worktree as failed so retry can create a fresh one
                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                        wt.fail("Verification failed".to_string());
                        let _ = worktree_repo.update(&wt).await;
                    }

                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!(
                                "Task {} failed verification (retry_count={}): {}",
                                task_id, retry_count, result.failures_summary.clone().unwrap_or_default()
                            ),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                }

                // When intent is satisfied, treat as passed even if
                // integration checks failed (advisory mode)
                result.passed || intent_satisfied
            }
            Err(e) => {
                if intent_satisfied {
                    // Infrastructure error with intent satisfied — advisory
                    tracing::warn!(
                        task_id = %task_id,
                        error = %e,
                        "Integration verification error but intent satisfied — \
                         proceeding to Complete"
                    );

                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            let _ = task.transition_to(TaskStatus::Complete);
                            let _ = task_repo.update(&task).await;
                        }
                    }

                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskCompleted,
                            AuditActor::System,
                            format!(
                                "Task {} completed (intent satisfied) despite verification error: {}",
                                task_id, e
                            ),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                    true
                } else {
                    // Transition Validating -> Failed on verification error and increment retry_count
                    let retry_count = if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            task.retry_count += 1;
                            let _ = task.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&task).await;
                        }
                        task.retry_count
                    } else {
                        0
                    };

                    // Emit TaskFailed event so handlers can react
                    let _ = event_tx.send(SwarmEvent::TaskFailed {
                        task_id,
                        error: format!("verification-error: {}", e),
                        retry_count,
                    }).await;

                    // Also mark worktree as failed so retry can create a fresh one
                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                        wt.fail(format!("Verification error: {}", e));
                        let _ = worktree_repo.update(&wt).await;
                    }

                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Task {} verification error (retry_count={}): {}", task_id, retry_count, e),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                    false
                }
            }
        }
    } else {
        true // Skip verification, assume passed
    };

    // Step 1.5: Feature branch handling
    if let Ok(Some(task)) = task_repo.get(task_id).await {
        // Case A: This is a subtask (has parent_id)
        if task.parent_id.is_some() {
            // Check if this is a merge-conflict-specialist completing
            let is_conflict_resolution = task.context.custom
                .get("feature_branch_conflict")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if is_conflict_resolution {
                // The specialist resolved the conflict directly in the root worktree.
                // Mark the original subtask's worktree as merged.
                if let Some(original_subtask_id_str) = task.context.custom
                    .get("original_subtask_id")
                    .and_then(|v| v.as_str())
                {
                    if let Ok(original_id) = Uuid::parse_str(original_subtask_id_str) {
                        if let Ok(Some(mut original_wt)) = worktree_repo.get_by_task(original_id).await {
                            original_wt.merged("conflict-resolved-by-specialist".to_string());
                            let _ = worktree_repo.update(&original_wt).await;
                        }
                    }
                }

                // Mark the merge request as completed in the queue
                if let Some(mr_id_str) = task.context.custom
                    .get("merge_request_id")
                    .and_then(|v| v.as_str())
                {
                    if let Ok(mr_id) = Uuid::parse_str(mr_id_str) {
                        let verifier = IntegrationVerifierService::new(
                            task_repo.clone(),
                            goal_repo.clone(),
                            worktree_repo.clone(),
                            VerifierConfig::default(),
                        );
                        let merge_config = MergeQueueConfig {
                            repo_path: repo_path.to_str().unwrap_or(".").to_string(),
                            main_branch: default_base_ref.to_string(),
                            ..Default::default()
                        };
                        let merge_queue = MergeQueue::new(
                            task_repo.clone(),
                            worktree_repo.clone(),
                            Arc::new(verifier),
                            merge_config,
                        );
                        let _ = merge_queue.retry_after_conflict_resolution(mr_id).await;
                    }
                }

                // Don't try to merge-back again (specialist already did the merge)
                // Fall through to auto-ship check below
            } else {
                // Normal subtask: attempt merge-back
                if verification_passed && require_commits {
                    let merge_result = merge_subtask_into_feature_branch(
                        task_id,
                        task_repo.clone(),
                        goal_repo.clone(),
                        worktree_repo.clone(),
                        event_tx,
                        audit_log,
                        repo_path,
                        default_base_ref,
                    ).await;

                    match merge_result {
                        Ok(MergeBackOutcome::Merged) => {
                            // Worktree cleanup for subtask
                            if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                wt.merged("merged-to-feature-branch".to_string());
                                let _ = worktree_repo.update(&wt).await;
                            }
                        }
                        Ok(MergeBackOutcome::NoCommits) => { /* nothing to merge */ }
                        Ok(MergeBackOutcome::ConflictQueued) => {
                            // Conflict recorded in merge queue; specialist trigger will handle.
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::warn!("Subtask {} merge-back failed: {}", task_id, e);
                        }
                    }
                }
            }

            // Check if the entire tree is ready to ship
            try_auto_ship(
                task_id, task_repo.clone(), worktree_repo.clone(),
                event_tx, audit_log, repo_path, default_base_ref,
            ).await;

            return Ok(()); // Skip normal per-task PR/merge flow
        }

        // Case B: This is a root task with children (Overmind)
        let subtasks = task_repo.get_subtasks(task_id).await.unwrap_or_default();
        if !subtasks.is_empty() {
            // Don't create per-task PR; check if all children are done
            try_auto_ship(
                task_id, task_repo.clone(), worktree_repo.clone(),
                event_tx, audit_log, repo_path, default_base_ref,
            ).await;
            return Ok(());
        }
    }

    // Step 2: Try PR creation if preferred and this is not a DirectMerge workflow.
    // DirectMerge bypasses PR creation and falls directly through to the merge queue.
    // (only for standalone tasks — no parent, no children)
    if verification_passed
        && prefer_pull_requests
        && require_commits
        && output_delivery != OutputDelivery::DirectMerge
    {
        if let Ok(Some(worktree)) = worktree_repo.get_by_task(task_id).await {
            // Look up task title/description for the PR
            let (pr_title, pr_body) = if let Ok(Some(task)) = task_repo.get(task_id).await {
                let intent = truncate_description(&task.description, 300);
                let changes = summarize_branch_changes(
                    &worktree.path,
                    &worktree.branch,
                    default_base_ref,
                ).await;
                let mut body = String::new();
                if !intent.is_empty() {
                    body.push_str("## Summary\n\n");
                    body.push_str(&intent);
                    body.push_str("\n\n");
                }
                if !changes.is_empty() {
                    body.push_str("## Changes\n\n");
                    body.push_str(&changes);
                    body.push_str("\n");
                }
                body.push_str("\n---\n🤖 Generated by [Abathur Swarm](https://github.com/abathur-swarm)\n");
                (task.title.clone(), body)
            } else {
                (format!("Task {}", task_id), String::new())
            };

            if let Some(pr_url) = try_create_pull_request(
                &worktree.path,
                &worktree.branch,
                &pr_title,
                &pr_body,
                default_base_ref,
            ).await {
                let _ = event_tx.send(SwarmEvent::PullRequestCreated {
                    task_id,
                    pr_url: pr_url.clone(),
                    branch: worktree.branch.clone(),
                }).await;

                audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!("Task {} PR created: {}", task_id, pr_url),
                ).await;

                return Ok(());
            }

            // PR creation failed — fall through to merge queue
            audit_log.info(
                AuditCategory::Task,
                AuditAction::TaskCompleted,
                format!("Task {} PR creation failed, falling back to merge queue", task_id),
            ).await;
        }
    }

    // Step 3: Queue for merge if verification passed and merge queue is enabled
    if verification_passed && use_merge_queue && require_commits {
        if let Ok(Some(worktree)) = worktree_repo.get_by_task(task_id).await {
            let verifier = IntegrationVerifierService::new(
                task_repo.clone(),
                goal_repo.clone(),
                worktree_repo.clone(),
                VerifierConfig::default(),
            );

            let merge_config = MergeQueueConfig {
                repo_path: repo_path.to_str().unwrap_or(".").to_string(),
                main_branch: default_base_ref.to_string(),
                require_verification: verify_on_completion,
                ..Default::default()
            };

            let merge_queue = MergeQueue::new(
                task_repo.clone(),
                worktree_repo.clone(),
                Arc::new(verifier),
                merge_config,
            );

            // Queue Stage 1: Agent worktree -> task branch
            let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
                task_id,
                stage: "AgentToTask".to_string(),
            }).await;

            match merge_queue.queue_stage1(
                task_id,
                &worktree.branch,
                &format!("task/{}", task_id),
            ).await {
                Ok(_) => {
                    audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCompleted,
                        format!("Task {} queued for stage 1 merge", task_id),
                    ).await;

                    // Process the queued merge
                    if let Ok(Some(result)) = merge_queue.process_next().await {
                        if result.success {
                            // Queue stage 2
                            let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
                                task_id,
                                stage: "TaskToMain".to_string(),
                            }).await;

                            if let Ok(_) = merge_queue.queue_stage2(task_id).await {
                                if let Ok(Some(result2)) = merge_queue.process_next().await {
                                    if result2.success {
                                        let _ = event_tx.send(SwarmEvent::TaskMerged {
                                            task_id,
                                            commit_sha: result2.commit_sha.clone().unwrap_or_default(),
                                        }).await;

                                        audit_log.info(
                                            AuditCategory::Task,
                                            AuditAction::TaskCompleted,
                                            format!(
                                                "Task {} merged to main: {}",
                                                task_id, result2.commit_sha.unwrap_or_default()
                                            ),
                                        ).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Task {} failed to queue for merge: {}", task_id, e),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                }
            }
        }
    }

    Ok(())
}

/// Merge a subtask's branch into the root ancestor's feature branch.
async fn merge_subtask_into_feature_branch<G, T, W>(
    task_id: Uuid,
    task_repo: Arc<T>,
    goal_repo: Arc<G>,
    worktree_repo: Arc<W>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    audit_log: &Arc<AuditLogService>,
    repo_path: &std::path::Path,
    default_base_ref: &str,
) -> DomainResult<MergeBackOutcome>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    use tokio::process::Command;

    let subtask_wt = match worktree_repo.get_by_task(task_id).await? {
        Some(wt) => wt,
        None => return Ok(MergeBackOutcome::NoCommits),
    };

    let task = match task_repo.get(task_id).await? {
        Some(t) => t,
        None => return Ok(MergeBackOutcome::NoCommits),
    };

    let root_id = find_root_ancestor_id(task.parent_id.unwrap(), &*task_repo).await;
    let root_wt = match worktree_repo.get_by_task(root_id).await? {
        Some(wt) => wt,
        None => return Ok(MergeBackOutcome::NoCommits),
    };

    // Check if subtask has commits ahead of feature branch
    let log_output = Command::new("git")
        .args(["log", &format!("{}..{}", root_wt.branch, subtask_wt.branch), "--oneline"])
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
    );

    // Queue merge: subtask branch → feature branch, in root's worktree
    merge_queue.queue_merge_back(
        task_id,
        &subtask_wt.branch,
        &root_wt.branch,
        &root_wt.path,
    ).await?;

    // Process immediately
    match merge_queue.process_next().await? {
        Some(result) if result.success => {
            let _ = event_tx.send(SwarmEvent::SubtaskMergedToFeature {
                task_id,
                feature_branch: root_wt.branch.clone(),
            }).await;

            audit_log.info(
                AuditCategory::Task,
                AuditAction::TaskCompleted,
                format!("Subtask {} merged into feature branch '{}'", task_id, root_wt.branch),
            ).await;

            Ok(MergeBackOutcome::Merged)
        }
        Some(result) if result.had_conflicts => {
            // Conflict detected. The MergeQueue recorded it with status=Conflict.
            // The existing process_merge_conflict_specialists (specialist_triggers.rs)
            // will pick this up on next tick and spawn a specialist.
            audit_log.info(
                AuditCategory::Task,
                AuditAction::TaskFailed,
                format!(
                    "Merge conflict merging subtask {} into feature branch: {:?}",
                    task_id, result.conflict_files
                ),
            ).await;

            Ok(MergeBackOutcome::ConflictQueued)
        }
        Some(result) => {
            // Non-conflict merge failure
            Err(DomainError::ExecutionFailed(
                result.error.unwrap_or_else(|| "Unknown merge failure".to_string())
            ))
        }
        None => Ok(MergeBackOutcome::NoCommits),
    }
}

/// Check if all tasks in a tree are terminal and, if so, create a single PR.
async fn try_auto_ship<T, W>(
    triggering_task_id: Uuid,
    task_repo: Arc<T>,
    worktree_repo: Arc<W>,
    event_tx: &mpsc::Sender<SwarmEvent>,
    audit_log: &Arc<AuditLogService>,
    repo_path: &std::path::Path,
    default_base_ref: &str,
) -> Option<String>
where
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    let root_id = find_root_ancestor_id(triggering_task_id, &*task_repo).await;
    let root_task = task_repo.get(root_id).await.ok()??;

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
        audit_log.info(
            AuditCategory::Task,
            AuditAction::TaskCompleted,
            format!("All tasks in tree {} terminal but none succeeded - no PR", root_id),
        ).await;
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

    // Check feature branch has commits ahead of base
    let has_commits = {
        use tokio::process::Command;
        Command::new("git")
            .args(["log", &format!("{}..{}", default_base_ref, root_wt.branch), "--oneline"])
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
        audit_log.info(
            AuditCategory::Task,
            AuditAction::TaskCompleted,
            format!("Feature branch {} has no commits ahead of {} - no PR", root_wt.branch, default_base_ref),
        ).await;
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

    // Build PR content from task tree
    let (pr_title, pr_body) = build_pr_description(
        root_id,
        &*task_repo,
        &root_wt.path,
        &root_wt.branch,
        default_base_ref,
    ).await;

    // Create the single PR using existing function
    let pr_url = try_create_pull_request(
        &root_wt.path,
        &root_wt.branch,
        &pr_title,
        &pr_body,
        default_base_ref,
    ).await?;

    let _ = event_tx.send(SwarmEvent::PullRequestCreated {
        task_id: root_id,
        pr_url: pr_url.clone(),
        branch: root_wt.branch.clone(),
    }).await;

    audit_log.info(
        AuditCategory::Task,
        AuditAction::TaskCompleted,
        format!("Feature branch PR created for tree {}: {}", root_id, pr_url),
    ).await;

    Some(pr_url)
}

/// BFS-walk the task tree from root and merge any unmerged descendant branches
/// into the root's worktree. This is a safety net to collect child work that
/// wasn't merged back during individual subtask completion (e.g., convergent
/// tasks where intent_satisfied skipped merge-back, or race conditions).
async fn collect_descendant_work<T: TaskRepository, W: WorktreeRepository>(
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
                    // Abort the failed merge to leave the worktree clean
                    let _ = Command::new("git")
                        .args(["merge", "--abort"])
                        .current_dir(&root_wt.path)
                        .output()
                        .await;
                }
                Err(e) => {
                    tracing::warn!(
                        "collect_descendant_work: failed to run merge for {}: {}",
                        st.id,
                        e
                    );
                }
            }
        }
    }
}

/// BFS check: all descendants in terminal state.
async fn all_descendants_terminal<T: TaskRepository>(root_id: Uuid, task_repo: &T) -> bool {
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        let subtasks = match task_repo.get_subtasks(id).await {
            Ok(s) => s,
            Err(_) => return false,
        };
        for st in subtasks {
            if !st.is_terminal() { return false; }
            queue.push(st.id);
        }
    }
    true
}

/// BFS check: any descendant completed successfully.
async fn has_any_successful_descendant<T: TaskRepository>(root_id: Uuid, task_repo: &T) -> bool {
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        if let Ok(subtasks) = task_repo.get_subtasks(id).await {
            for st in &subtasks {
                if st.status == TaskStatus::Complete { return true; }
                queue.push(st.id);
            }
        }
    }
    false
}

/// Build combined PR title and body from the task tree.
///
/// Produces a structured PR description with:
/// - A truncated intent summary (not the full convergence prompt)
/// - Git commit log and diffstat for the branch
/// - A checklist of subtask statuses
async fn build_pr_description<T: TaskRepository>(
    root_id: Uuid,
    task_repo: &T,
    worktree_path: &str,
    branch: &str,
    base_ref: &str,
) -> (String, String) {
    let root = task_repo.get(root_id).await.ok().flatten();

    let root_title = root.as_ref()
        .map(|t| t.title.clone())
        .unwrap_or_else(|| format!("Task {}", &root_id.to_string()[..8]));

    let root_desc = root.as_ref()
        .map(|t| t.description.clone())
        .unwrap_or_default();

    let mut body = String::new();

    // Intent section — short summary, never the full prompt
    let intent = truncate_description(&root_desc, 300);
    if !intent.is_empty() {
        body.push_str("## Summary\n\n");
        body.push_str(&intent);
        body.push_str("\n\n");
    }

    // Changes section from git
    let changes = summarize_branch_changes(worktree_path, branch, base_ref).await;
    if !changes.is_empty() {
        body.push_str("## Changes\n\n");
        body.push_str(&changes);
        body.push_str("\n");
    }

    // Subtasks section
    body.push_str("## Subtasks\n\n");
    let mut bfs = vec![root_id];
    while let Some(id) = bfs.pop() {
        if let Ok(subtasks) = task_repo.get_subtasks(id).await {
            for st in &subtasks {
                let marker = match st.status {
                    TaskStatus::Complete => "- [x]",
                    TaskStatus::Failed => "- [-]",
                    TaskStatus::Canceled => "- [~]",
                    _ => "- [ ]",
                };
                body.push_str(&format!("{} {} ({})\n", marker, st.title, st.status.as_str()));
                bfs.push(st.id);
            }
        }
    }

    body.push_str("\n---\n🤖 Generated by [Abathur Swarm](https://github.com/abathur-swarm)\n");

    (root_title, body)
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
                     relevant to reviewers at all. ".repeat(5);
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
        let desc = format!("# Header One\n## Header Two\n### Header Three\n\n{}", long_body);
        let result = truncate_description(&desc, 300);
        // First paragraph is all headers; after filtering, it's empty.
        // Falls back to first non-header, non-empty line from the full description.
        assert!(!result.starts_with('#'));
    }
}

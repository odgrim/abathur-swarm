//! Helper functions for the swarm orchestrator.
//!
//! Top-level utility functions used by spawned tasks that don't have access
//! to the orchestrator instance (e.g., auto-commit, post-completion workflow).

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::TaskStatus;
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
) -> DomainResult<()>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
    // When the convergence engine has verified intent satisfaction (e.g., "remove dead code"
    // where the code is already clean), skip the commits gate so the task can complete
    // successfully without producing any commits.
    let effective_require_commits = require_commits && !intent_satisfied;
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
                require_commits: effective_require_commits,
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
                if verification_passed && effective_require_commits {
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

    // Step 2: Try PR creation if preferred, then fall back to merge queue
    // (only for standalone tasks — no parent, no children)
    if verification_passed && prefer_pull_requests && effective_require_commits {
        if let Ok(Some(worktree)) = worktree_repo.get_by_task(task_id).await {
            // Look up task title/description for the PR
            let (pr_title, pr_body) = if let Ok(Some(task)) = task_repo.get(task_id).await {
                (task.title.clone(), task.description.clone())
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
    if verification_passed && use_merge_queue && effective_require_commits {
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
    if !root_task.is_terminal() { return None; }

    // All descendants must be terminal
    if !all_descendants_terminal(root_id, &*task_repo).await { return None; }

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
        audit_log.info(
            AuditCategory::Task,
            AuditAction::TaskCompleted,
            format!("Feature branch {} has no commits ahead of {} - no PR", root_wt.branch, default_base_ref),
        ).await;
        return None;
    }

    // Build PR content from task tree
    let (pr_title, pr_body) = build_pr_description(root_id, &*task_repo).await;

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
async fn build_pr_description<T: TaskRepository>(root_id: Uuid, task_repo: &T) -> (String, String) {
    let root_title = task_repo.get(root_id).await
        .ok().flatten()
        .map(|t| t.title.clone())
        .unwrap_or_else(|| format!("Task {}", &root_id.to_string()[..8]));

    let root_desc = task_repo.get(root_id).await
        .ok().flatten()
        .map(|t| t.description.clone())
        .unwrap_or_default();

    let mut body = String::new();
    if !root_desc.is_empty() {
        body.push_str(&root_desc);
        body.push_str("\n\n");
    }
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

    (root_title, body)
}

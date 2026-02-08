//! Helper functions for the swarm orchestrator.
//!
//! Top-level utility functions used by spawned tasks that don't have access
//! to the orchestrator instance (e.g., auto-commit, post-completion workflow).

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
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

/// Helper function to run post-completion workflow (verification and merging).
/// This is called from spawned tasks after successful task completion.
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
) -> DomainResult<()>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
{
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
                require_commits,
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
                } else {
                    // Transition Validating -> Failed
                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                        if task.status == TaskStatus::Validating {
                            let _ = task.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&task).await;
                        }
                    }

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
                                "Task {} failed verification: {}",
                                task_id, result.failures_summary.clone().unwrap_or_default()
                            ),
                        )
                        .with_entity(task_id, "task"),
                    ).await;
                }

                result.passed
            }
            Err(e) => {
                // Transition Validating -> Failed on verification error
                if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                    if task.status == TaskStatus::Validating {
                        let _ = task.transition_to(TaskStatus::Failed);
                        let _ = task_repo.update(&task).await;
                    }
                }

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
                        format!("Task {} verification error: {}", task_id, e),
                    )
                    .with_entity(task_id, "task"),
                ).await;
                false
            }
        }
    } else {
        true // Skip verification, assume passed
    };

    // Step 2: Try PR creation if preferred, then fall back to merge queue
    if verification_passed && prefer_pull_requests {
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
    if verification_passed && use_merge_queue {
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

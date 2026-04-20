//! Post-completion middleware: merge a subtask's branch back into the root
//! ancestor's feature branch (or handle merge-conflict-specialist completion).
//!
//! Only applies to tasks that have a `parent_id`. Standalone tasks and
//! root-with-children tasks are no-ops here. Sets `tree_handled = true` so
//! later PR / merge-queue middleware stand down — this matches the previous
//! `return Ok(())` early-exit after subtask handling.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::services::{IntegrationVerifierService, MergeQueue, MergeQueueConfig, VerifierConfig};

use super::super::helpers::{
    MergeBackOutcome, MergeSubtaskParams, merge_subtask_into_feature_branch,
};
use super::{PostCompletionContext, PostCompletionMiddleware};

pub struct SubtaskMergeBackMiddleware;

impl SubtaskMergeBackMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubtaskMergeBackMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostCompletionMiddleware for SubtaskMergeBackMiddleware {
    fn name(&self) -> &'static str {
        "subtask-merge-back"
    }

    async fn handle(&self, ctx: &mut PostCompletionContext) -> DomainResult<()> {
        if ctx.tree_handled {
            return Ok(());
        }

        let task_id = ctx.task_id;
        let task = match ctx.task_repo.get(task_id).await? {
            Some(t) => t,
            None => return Ok(()),
        };

        // Only applies to subtasks.
        if task.parent_id.is_none() {
            return Ok(());
        }

        // Merge-conflict-specialist completion has its own handling path.
        let is_conflict_resolution = task
            .context
            .custom
            .get("feature_branch_conflict")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_conflict_resolution {
            // The specialist resolved the conflict directly in the root worktree.
            // Mark the original subtask's worktree as merged.
            if let Some(original_subtask_id_str) = task
                .context
                .custom
                .get("original_subtask_id")
                .and_then(|v| v.as_str())
                && let Ok(original_id) = Uuid::parse_str(original_subtask_id_str)
                && let Ok(Some(mut original_wt)) = ctx.worktree_repo.get_by_task(original_id).await
            {
                original_wt.merged("conflict-resolved-by-specialist".to_string());
                let _ = ctx.worktree_repo.update(&original_wt).await;
            }

            if let Some(ref mr_repo) = ctx.merge_request_repo
                && let Some(mr_id_str) = task
                    .context
                    .custom
                    .get("merge_request_id")
                    .and_then(|v| v.as_str())
                && let Ok(mr_id) = Uuid::parse_str(mr_id_str)
            {
                let verifier = IntegrationVerifierService::new(
                    ctx.task_repo.clone(),
                    ctx.goal_repo.clone(),
                    ctx.worktree_repo.clone(),
                    VerifierConfig::default(),
                );
                let merge_config = MergeQueueConfig {
                    repo_path: ctx.repo_path.to_str().unwrap_or(".").to_string(),
                    main_branch: ctx.default_base_ref.clone(),
                    ..Default::default()
                };
                let merge_queue = MergeQueue::new(
                    ctx.task_repo.clone(),
                    ctx.worktree_repo.clone(),
                    Arc::new(verifier),
                    merge_config,
                    mr_repo.clone(),
                );
                if let Ok(true) = merge_queue.retry_after_conflict_resolution(mr_id).await {
                    match merge_queue.process_next().await {
                        Ok(Some(result)) if result.success => {
                            tracing::info!(task_id = %task_id, "merge succeeded after conflict resolution");
                        }
                        Ok(Some(result)) if result.had_conflicts => {
                            tracing::warn!(task_id = %task_id, "merge still has conflicts after specialist resolution");
                        }
                        Ok(Some(result)) => {
                            tracing::warn!(task_id = %task_id, error = ?result.error, "merge failed after conflict resolution");
                        }
                        Ok(None) => {
                            tracing::debug!(task_id = %task_id, "no merge to process after conflict resolution retry");
                        }
                        Err(e) => {
                            tracing::warn!(task_id = %task_id, error = %e, "error processing merge after conflict resolution");
                        }
                    }
                }
            }
            // Don't merge-back again; fall through so autoship still runs.
        } else if ctx.verification_passed && ctx.require_commits {
            let merge_result = merge_subtask_into_feature_branch(MergeSubtaskParams {
                task_id,
                task_repo: ctx.task_repo.clone(),
                goal_repo: ctx.goal_repo.clone(),
                worktree_repo: ctx.worktree_repo.clone(),
                event_tx: &ctx.event_tx,
                audit_log: &ctx.audit_log,
                repo_path: &ctx.repo_path,
                default_base_ref: &ctx.default_base_ref,
                merge_request_repo: ctx.merge_request_repo.clone(),
            })
            .await;

            match merge_result {
                Ok(MergeBackOutcome::Merged) => {
                    if let Ok(Some(mut wt)) = ctx.worktree_repo.get_by_task(task_id).await {
                        wt.merged("merged-to-feature-branch".to_string());
                        let _ = ctx.worktree_repo.update(&wt).await;
                    }
                }
                Ok(MergeBackOutcome::NoCommits) => { /* nothing to merge */ }
                Ok(MergeBackOutcome::ConflictQueued) => {
                    // Conflict recorded in merge queue; specialist trigger will
                    // handle. Mark handled so we skip autoship / PR / merge.
                    ctx.tree_handled = true;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Subtask {} merge-back failed: {}", task_id, e);
                }
            }
        }

        // Either conflict-resolution or normal subtask-merge path completed.
        // The original logic falls through to try_auto_ship and then returns
        // Ok(()), meaning PR/merge-queue should NOT run. We don't set
        // tree_handled here yet — we let AutoshipMiddleware run first, then
        // it (or we, below) block PR/merge. Instead, AutoshipMiddleware sets
        // tree_handled when it runs for a subtask.
        Ok(())
    }
}

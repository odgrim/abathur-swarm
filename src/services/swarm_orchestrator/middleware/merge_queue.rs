//! Post-completion middleware: queue a completed standalone task for the
//! two-stage merge queue.

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::services::swarm_orchestrator::types::SwarmEvent;
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, IntegrationVerifierService,
    MergeQueue, MergeQueueConfig, VerifierConfig,
};

use super::{PostCompletionContext, PostCompletionMiddleware};

pub struct MergeQueueMiddleware;

impl MergeQueueMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MergeQueueMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostCompletionMiddleware for MergeQueueMiddleware {
    fn name(&self) -> &'static str {
        "merge-queue"
    }

    async fn handle(&self, ctx: &mut PostCompletionContext) -> DomainResult<()> {
        if ctx.tree_handled {
            return Ok(());
        }

        if !(ctx.verification_passed && ctx.use_merge_queue && ctx.require_commits) {
            return Ok(());
        }

        let Some(ref mr_repo) = ctx.merge_request_repo else {
            return Ok(());
        };

        let task_id = ctx.task_id;
        let Ok(Some(worktree)) = ctx.worktree_repo.get_by_task(task_id).await else {
            return Ok(());
        };

        let verifier = IntegrationVerifierService::new(
            ctx.task_repo.clone(),
            ctx.goal_repo.clone(),
            ctx.worktree_repo.clone(),
            VerifierConfig::default(),
        );

        let merge_config = MergeQueueConfig {
            repo_path: ctx.repo_path.to_str().unwrap_or(".").to_string(),
            main_branch: ctx.default_base_ref.clone(),
            require_verification: ctx.verify_on_completion,
            ..Default::default()
        };

        let merge_queue = MergeQueue::new(
            ctx.task_repo.clone(),
            ctx.worktree_repo.clone(),
            Arc::new(verifier),
            merge_config,
            mr_repo.clone(),
        );

        // Stage 1: agent worktree -> task branch
        let _ = ctx
            .event_tx
            .send(SwarmEvent::TaskQueuedForMerge {
                task_id,
                stage: "AgentToTask".to_string(),
            })
            .await;

        match merge_queue
            .queue_stage1(task_id, &worktree.branch, &format!("task/{}", task_id))
            .await
        {
            Ok(_) => {
                ctx.audit_log
                    .info(
                        AuditCategory::Task,
                        AuditAction::TaskCompleted,
                        format!("Task {} queued for stage 1 merge", task_id),
                    )
                    .await;

                if let Ok(Some(result)) = merge_queue.process_next().await
                    && result.success
                {
                    let _ = ctx
                        .event_tx
                        .send(SwarmEvent::TaskQueuedForMerge {
                            task_id,
                            stage: "TaskToMain".to_string(),
                        })
                        .await;

                    if merge_queue.queue_stage2(task_id).await.is_ok()
                        && let Ok(Some(result2)) = merge_queue.process_next().await
                        && result2.success
                    {
                        let _ = ctx
                            .event_tx
                            .send(SwarmEvent::TaskMerged {
                                task_id,
                                commit_sha: result2.commit_sha.clone().unwrap_or_default(),
                            })
                            .await;

                        ctx.audit_log
                            .info(
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                format!(
                                    "Task {} merged to main: {}",
                                    task_id,
                                    result2.commit_sha.unwrap_or_default()
                                ),
                            )
                            .await;
                    }
                }
            }
            Err(e) => {
                ctx.audit_log
                    .log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Task {} failed to queue for merge: {}", task_id, e),
                        )
                        .with_entity(task_id, "task"),
                    )
                    .await;
            }
        }

        Ok(())
    }
}

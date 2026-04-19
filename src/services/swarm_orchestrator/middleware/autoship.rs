//! Post-completion middleware: attempt a tree-wide auto-ship.
//!
//! Runs when the task is either:
//! - a subtask (has `parent_id`), or
//! - a root with children (no `parent_id`, has subtasks).
//!
//! In both cases the original workflow short-circuited after this step,
//! skipping PR/merge-queue; we preserve that by setting `tree_handled = true`.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;

use super::super::helpers::try_auto_ship;
use super::{PostCompletionContext, PostCompletionMiddleware};

pub struct AutoshipMiddleware;

impl AutoshipMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AutoshipMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostCompletionMiddleware for AutoshipMiddleware {
    fn name(&self) -> &'static str {
        "autoship"
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

        let is_subtask = task.parent_id.is_some();
        let is_root_with_children = task.parent_id.is_none()
            && !ctx
                .task_repo
                .get_subtasks(task_id)
                .await
                .unwrap_or_default()
                .is_empty();

        if !is_subtask && !is_root_with_children {
            // Standalone task — no tree-wide ship; PR / merge-queue handle it.
            return Ok(());
        }

        try_auto_ship(
            task_id,
            ctx.task_repo.clone(),
            ctx.worktree_repo.clone(),
            &ctx.event_tx,
            &ctx.audit_log,
            &ctx.repo_path,
            &ctx.default_base_ref,
            ctx.output_delivery.clone(),
            ctx.fetch_on_sync,
        )
        .await;

        // Either way, the per-task PR / merge-queue flow is not appropriate
        // for tree tasks — they either ship as a whole or not at all.
        ctx.tree_handled = true;
        Ok(())
    }
}

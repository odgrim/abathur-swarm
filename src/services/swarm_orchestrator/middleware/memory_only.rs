//! Post-completion middleware: short-circuit for `OutputDelivery::MemoryOnly`.
//!
//! MemoryOnly workflows have already persisted findings to swarm memory; no
//! git operations are needed or appropriate. Setting `tree_handled = true`
//! causes downstream middleware (verification, merge, PR, merge queue) to
//! stand down — preserving the previous `return Ok(())` early-exit.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::domain::models::workflow_template::OutputDelivery;

use super::{PostCompletionContext, PostCompletionMiddleware};

pub struct MemoryOnlyShortCircuitMiddleware;

impl MemoryOnlyShortCircuitMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryOnlyShortCircuitMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PostCompletionMiddleware for MemoryOnlyShortCircuitMiddleware {
    fn name(&self) -> &'static str {
        "memory-only-short-circuit"
    }

    async fn handle(&self, ctx: &mut PostCompletionContext) -> DomainResult<()> {
        if ctx.output_delivery == OutputDelivery::MemoryOnly {
            tracing::debug!(
                task_id = %ctx.task_id,
                "OutputDelivery::MemoryOnly — skipping git post-completion workflow"
            );
            // Mark the tree as handled so later middleware stand down.
            ctx.tree_handled = true;
        }
        Ok(())
    }
}

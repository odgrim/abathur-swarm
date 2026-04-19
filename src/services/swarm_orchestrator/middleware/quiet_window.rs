//! Pre-spawn middleware: defer dispatch inside a cost/quiet window.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

pub struct QuietWindowMiddleware;

impl QuietWindowMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for QuietWindowMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for QuietWindowMiddleware {
    fn name(&self) -> &'static str {
        "quiet-window"
    }

    async fn handle(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
        let Some(ref cws) = ctx.cost_window_service else {
            return Ok(PreSpawnDecision::Continue);
        };

        let check = cws.is_in_quiet_window().await;
        if check.is_quiet {
            tracing::info!(
                task_id = %ctx.task.id,
                window_name = ?check.active_window_name,
                "spawn_task_agent: deferring task — inside quiet window"
            );
            return Ok(PreSpawnDecision::Skip {
                reason: format!(
                    "quiet-window:{}",
                    check
                        .active_window_name
                        .unwrap_or_else(|| "unnamed".to_string())
                ),
            });
        }

        Ok(PreSpawnDecision::Continue)
    }
}

//! Pre-spawn middleware: honour the orchestrator guardrails.
//!
//! Guardrails track per-agent spawn rate and global limits. If a spawn would
//! breach a limit, skip and let the task retry on the next poll.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

pub struct GuardrailsMiddleware;

impl GuardrailsMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GuardrailsMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for GuardrailsMiddleware {
    fn name(&self) -> &'static str {
        "guardrails"
    }

    async fn handle(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
        // The unique id used by guardrails is the task id as a string —
        // matches the previous inline behaviour. Registration happens after
        // atomic claim in the orchestrator, not here.
        let unique_id = ctx.task.id.to_string();
        let spawn_check = ctx.guardrails.check_agent_spawn(&unique_id).await;
        if spawn_check.is_blocked() {
            tracing::debug!(
                task_id = %ctx.task.id,
                "spawn_task_agent: blocked by guardrails — {:?}",
                spawn_check
            );
            return Ok(PreSpawnDecision::Skip {
                reason: format!("guardrails:{:?}", spawn_check),
            });
        }

        Ok(PreSpawnDecision::Continue)
    }
}

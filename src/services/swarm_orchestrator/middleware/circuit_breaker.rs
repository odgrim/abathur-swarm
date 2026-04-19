//! Pre-spawn middleware: honour the per-agent circuit breaker.
//!
//! If the circuit breaker for this agent is open/blocked, skip the spawn and
//! log an audit entry. Requires `RouteTaskMiddleware` to have set
//! `ctx.agent_type`; without a resolved agent there's nothing to scope on.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, CircuitScope,
};

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

pub struct CircuitBreakerMiddleware;

impl CircuitBreakerMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CircuitBreakerMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for CircuitBreakerMiddleware {
    fn name(&self) -> &'static str {
        "circuit-breaker"
    }

    async fn handle(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
        let Some(ref agent_type) = ctx.agent_type else {
            // Routing hasn't run; nothing to check. Stay out of the way.
            return Ok(PreSpawnDecision::Continue);
        };

        let scope = CircuitScope::agent(agent_type);
        let check_result = ctx.circuit_breaker.check(scope).await;

        if check_result.is_blocked() {
            ctx.audit_log
                .log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Execution,
                        AuditAction::CircuitBreakerTriggered,
                        AuditActor::System,
                        format!(
                            "Task {} blocked by circuit breaker for agent '{}'",
                            ctx.task.id, agent_type
                        ),
                    )
                    .with_entity(ctx.task.id, "task"),
                )
                .await;

            return Ok(PreSpawnDecision::Skip {
                reason: format!("circuit-breaker-blocked:{}", agent_type),
            });
        }

        Ok(PreSpawnDecision::Continue)
    }
}

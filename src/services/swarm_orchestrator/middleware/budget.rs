//! Pre-spawn middleware: budget-pressure gates.
//!
//! Two related middlewares:
//! - [`BudgetDispatchMiddleware`] defers low-priority tasks under elevated
//!   budget pressure (matches the previous `should_dispatch_task` gate).
//! - [`BudgetConcurrencyMiddleware`] enforces a budget-adjusted ceiling on
//!   concurrent agents.
//!
//! Both are no-ops when no `BudgetTracker` is attached to the orchestrator.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

pub struct BudgetDispatchMiddleware;

impl BudgetDispatchMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BudgetDispatchMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for BudgetDispatchMiddleware {
    fn name(&self) -> &'static str {
        "budget-dispatch"
    }

    async fn handle(
        &self,
        ctx: &mut PreSpawnContext,
    ) -> DomainResult<PreSpawnDecision> {
        let Some(ref bt) = ctx.budget_tracker else {
            return Ok(PreSpawnDecision::Continue);
        };

        if !bt.should_dispatch_task(ctx.task.priority).await {
            tracing::debug!(
                task_id = %ctx.task.id,
                priority = ?ctx.task.priority,
                "spawn_task_agent: deferring task — budget pressure"
            );
            return Ok(PreSpawnDecision::Skip {
                reason: "budget-pressure".to_string(),
            });
        }

        Ok(PreSpawnDecision::Continue)
    }
}

pub struct BudgetConcurrencyMiddleware;

impl BudgetConcurrencyMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BudgetConcurrencyMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for BudgetConcurrencyMiddleware {
    fn name(&self) -> &'static str {
        "budget-concurrency"
    }

    async fn handle(
        &self,
        ctx: &mut PreSpawnContext,
    ) -> DomainResult<PreSpawnDecision> {
        let Some(ref bt) = ctx.budget_tracker else {
            return Ok(PreSpawnDecision::Continue);
        };

        let running = ctx
            .max_agents
            .saturating_sub(ctx.agent_semaphore.available_permits());
        let budget_max = bt.effective_max_agents(ctx.max_agents as u32).await as usize;
        if running >= budget_max {
            tracing::debug!(
                task_id = %ctx.task.id,
                running,
                budget_max,
                "spawn_task_agent: skipping — at budget-adjusted agent limit"
            );
            return Ok(PreSpawnDecision::Skip {
                reason: format!(
                    "budget-concurrency-ceiling:{}/{}",
                    running, budget_max
                ),
            });
        }

        Ok(PreSpawnDecision::Continue)
    }
}

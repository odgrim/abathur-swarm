//! Pre-spawn middleware: inherit the parent goal's current priority onto the
//! task at spawn time.
//!
//! Task priority is set at creation and is not refreshed automatically. When a
//! federation handler (or a human, or a convergence signal) bumps the owning
//! goal's priority *after* the task was created, the task's own `priority`
//! field goes stale. This middleware reconciles that: if the task's parent
//! goal is currently at a higher priority than the task, it bumps the task's
//! in-memory priority to match before the rest of the pre-spawn chain runs.
//!
//! Downstream middleware (budget gates, guardrails) observe the bumped value
//! via `ctx.task.priority` and can apply priority-sensitive behavior (e.g.
//! allowing a Critical task through a normally-closed gate).
//!
//! Task→goal linkage follows the project-wide convention:
//! `task.context.custom["goal_id"]` stores the goal UUID as a JSON string. If
//! the key is absent or the goal cannot be loaded, the middleware is a no-op.
//!
//! Audit: every bump is recorded via the audit log so operators can see when
//! and why runtime priority changed.
//!
//! Why "federation" priority: goal priority on a child swarm is the signal
//! path federation uses to propagate parent-cerebrate urgency (convergence
//! poller / federation handler bump goal priority in response to upstream
//! signals). Reading it at spawn time is how those signals land on the
//! individual task's runtime behaviour without mutating persisted task rows
//! on every signal.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{GoalPriority, TaskPriority};
use crate::services::{AuditAction, AuditCategory};

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

pub struct FederationPriorityMiddleware;

impl FederationPriorityMiddleware {
    pub fn new() -> Self {
        Self
    }

    fn goal_priority_to_task_priority(p: GoalPriority) -> TaskPriority {
        match p {
            GoalPriority::Low => TaskPriority::Low,
            GoalPriority::Normal => TaskPriority::Normal,
            GoalPriority::High => TaskPriority::High,
            GoalPriority::Critical => TaskPriority::Critical,
        }
    }

    fn extract_goal_id(ctx: &PreSpawnContext) -> Option<Uuid> {
        ctx.task.goal_id()
    }
}

impl Default for FederationPriorityMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for FederationPriorityMiddleware {
    fn name(&self) -> &'static str {
        "federation-priority"
    }

    async fn handle(&self, ctx: &mut PreSpawnContext) -> DomainResult<PreSpawnDecision> {
        let Some(goal_id) = Self::extract_goal_id(ctx) else {
            return Ok(PreSpawnDecision::Continue);
        };

        // Repository lookup failures are not fatal — the middleware is
        // advisory. Log and carry on so a repo hiccup can't block spawn.
        let goal = match ctx.goal_repo.get(goal_id).await {
            Ok(Some(g)) => g,
            Ok(None) => {
                tracing::debug!(
                    task_id = %ctx.task.id,
                    %goal_id,
                    "federation-priority: referenced goal not found, skipping"
                );
                return Ok(PreSpawnDecision::Continue);
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %ctx.task.id,
                    %goal_id,
                    error = %e,
                    "federation-priority: goal lookup failed, skipping"
                );
                return Ok(PreSpawnDecision::Continue);
            }
        };

        let desired = Self::goal_priority_to_task_priority(goal.priority);
        if desired <= ctx.task.priority {
            return Ok(PreSpawnDecision::Continue);
        }

        let previous = ctx.task.priority;
        ctx.task.priority = desired;
        ctx.federation_priority_bumps = ctx.federation_priority_bumps.saturating_add(1);

        tracing::info!(
            task_id = %ctx.task.id,
            %goal_id,
            from = previous.as_str(),
            to = desired.as_str(),
            "federation-priority: bumped task priority to match goal"
        );

        ctx.audit_log
            .info(
                AuditCategory::Task,
                AuditAction::TaskStateChanged,
                format!(
                    "Bumped task {} priority {} -> {} to match goal {}",
                    ctx.task.id,
                    previous.as_str(),
                    desired.as_str(),
                    goal_id
                ),
            )
            .await;

        Ok(PreSpawnDecision::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::test_support;
    use crate::domain::models::{Goal, Task};
    use crate::domain::ports::{AgentRepository, GoalRepository, TaskRepository};
    use crate::services::swarm_orchestrator::middleware::PreSpawnContext;
    use crate::services::{AuditLogService, CircuitBreakerService, Guardrails};
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    async fn make_ctx_with_goal(
        goal: Option<Goal>,
        task_priority: TaskPriority,
        goal_id_in_task: Option<Uuid>,
    ) -> PreSpawnContext {
        let (task_repo_concrete, agent_repo_concrete, goal_repo_concrete) =
            test_support::setup_task_agent_goal_repos().await;

        if let Some(ref g) = goal {
            goal_repo_concrete.create(g).await.unwrap();
        }

        let mut task = Task::with_title("fp-test", "priority bump test");
        task.priority = task_priority;
        if let Some(gid) = goal_id_in_task {
            task.set_goal_id(gid);
        }

        let task_repo: Arc<dyn TaskRepository> = task_repo_concrete;
        let agent_repo: Arc<dyn AgentRepository> = agent_repo_concrete;
        let goal_repo: Arc<dyn GoalRepository> = goal_repo_concrete;

        PreSpawnContext {
            task,
            agent_type: None,
            task_repo,
            agent_repo,
            goal_repo,
            audit_log: Arc::new(AuditLogService::with_defaults()),
            circuit_breaker: Arc::new(CircuitBreakerService::with_defaults()),
            guardrails: Arc::new(Guardrails::with_defaults()),
            cost_window_service: None,
            budget_tracker: None,
            agent_semaphore: Arc::new(Semaphore::new(4)),
            max_agents: 4,
            federation_priority_bumps: 0,
        }
    }

    #[tokio::test]
    async fn no_op_when_task_has_no_goal_id() {
        let mut ctx = make_ctx_with_goal(None, TaskPriority::Normal, None).await;
        let before = ctx.task.priority;

        let decision = FederationPriorityMiddleware::new()
            .handle(&mut ctx)
            .await
            .unwrap();

        assert!(matches!(decision, PreSpawnDecision::Continue));
        assert_eq!(ctx.task.priority, before);
        assert_eq!(ctx.federation_priority_bumps, 0);
    }

    #[tokio::test]
    async fn no_op_when_goal_priority_is_not_higher() {
        let goal = Goal::new("g", "d").with_priority(GoalPriority::Normal);
        let gid = goal.id;
        let mut ctx = make_ctx_with_goal(Some(goal), TaskPriority::High, Some(gid)).await;

        let decision = FederationPriorityMiddleware::new()
            .handle(&mut ctx)
            .await
            .unwrap();

        assert!(matches!(decision, PreSpawnDecision::Continue));
        assert_eq!(ctx.task.priority, TaskPriority::High);
        assert_eq!(ctx.federation_priority_bumps, 0);
    }

    #[tokio::test]
    async fn bumps_task_priority_up_to_goal_priority() {
        let goal = Goal::new("g", "d").with_priority(GoalPriority::Critical);
        let gid = goal.id;
        let mut ctx = make_ctx_with_goal(Some(goal), TaskPriority::Normal, Some(gid)).await;

        let decision = FederationPriorityMiddleware::new()
            .handle(&mut ctx)
            .await
            .unwrap();

        assert!(matches!(decision, PreSpawnDecision::Continue));
        assert_eq!(ctx.task.priority, TaskPriority::Critical);
        assert_eq!(ctx.federation_priority_bumps, 1);
    }

    #[tokio::test]
    async fn missing_goal_is_non_fatal() {
        // goal_id present on task but the goal row doesn't exist.
        let stray = Uuid::new_v4();
        let mut ctx = make_ctx_with_goal(None, TaskPriority::Normal, Some(stray)).await;

        let decision = FederationPriorityMiddleware::new()
            .handle(&mut ctx)
            .await
            .unwrap();

        assert!(matches!(decision, PreSpawnDecision::Continue));
        assert_eq!(ctx.task.priority, TaskPriority::Normal);
        assert_eq!(ctx.federation_priority_bumps, 0);
    }
}

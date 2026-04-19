//! Pre-spawn middleware: resolve the concrete agent template (agent_type).
//!
//! Resolution priority matches the previous inline logic:
//! 1. Explicit `task.agent_type` (user specified `--agent`)
//! 2. `task.routing_hints.preferred_agent` (validated against the agent repo)
//! 3. Capability matching against `task.routing_hints.required_tools`
//! 4. Default to `"overmind"`
//!
//! The resolved value is stored on the context AND (when the task didn't
//! previously have `agent_type` set) persisted back on the task record so
//! audit logs and task queries reflect the routing decision.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::domain::models::Task;
use crate::domain::ports::{AgentFilter, AgentRepository};

use super::{PreSpawnContext, PreSpawnDecision, PreSpawnMiddleware};

/// Resolve a task's agent_type.
pub struct RouteTaskMiddleware;

impl RouteTaskMiddleware {
    pub fn new() -> Self {
        Self
    }

    async fn route(task: &Task, agent_repo: &dyn AgentRepository) -> String {
        // 1. Explicit assignment
        if let Some(ref agent) = task.agent_type {
            return agent.clone();
        }

        // 2. Preferred agent (validate existence)
        if let Some(ref preferred) = task.routing_hints.preferred_agent
            && let Ok(Some(_)) = agent_repo.get_template_by_name(preferred).await
        {
            return preferred.clone();
        }

        // 3. Capability matching
        if !task.routing_hints.required_tools.is_empty()
            && let Some(matched) =
                Self::match_agent_by_tools(agent_repo, &task.routing_hints.required_tools).await
        {
            return matched;
        }

        // 4. Default: route to overmind
        if task.parent_id.is_some() {
            tracing::warn!(
                task_id = %task.id,
                parent_id = ?task.parent_id,
                "route_task: subtask has no agent — Overmind should set `agent` in workflow_fan_out slices"
            );
        }
        "overmind".to_string()
    }

    async fn match_agent_by_tools(
        agent_repo: &dyn AgentRepository,
        required_tools: &[String],
    ) -> Option<String> {
        let templates = agent_repo.list_templates(AgentFilter::default()).await.ok()?;

        let mut best_match: Option<(String, usize)> = None;

        for template in &templates {
            // Skip meta-level agents for direct tool matching
            if template.name == "overmind" {
                continue;
            }

            let tool_names: Vec<&str> =
                template.tools.iter().map(|t| t.name.as_str()).collect();
            let matched_count = required_tools
                .iter()
                .filter(|req| tool_names.iter().any(|t| t.eq_ignore_ascii_case(req)))
                .count();

            if matched_count > 0 {
                if let Some((_, best_count)) = &best_match {
                    if matched_count > *best_count {
                        best_match = Some((template.name.clone(), matched_count));
                    }
                } else {
                    best_match = Some((template.name.clone(), matched_count));
                }
            }
        }

        best_match.map(|(name, _)| name)
    }
}

impl Default for RouteTaskMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreSpawnMiddleware for RouteTaskMiddleware {
    fn name(&self) -> &'static str {
        "route-task"
    }

    async fn handle(
        &self,
        ctx: &mut PreSpawnContext,
    ) -> DomainResult<PreSpawnDecision> {
        let agent_type = Self::route(&ctx.task, &*ctx.agent_repo).await;

        // Persist routing decision only when task.agent_type was None — same
        // condition the previous inline logic used.
        if ctx.task.agent_type.is_none()
            && let Ok(Some(mut updated)) = ctx.task_repo.get(ctx.task.id).await
        {
            updated.agent_type = Some(agent_type.clone());
            let _ = ctx.task_repo.update(&updated).await;
        }

        ctx.agent_type = Some(agent_type);
        Ok(PreSpawnDecision::Continue)
    }
}

//! Agent lifecycle subsystem for the swarm orchestrator.
//!
//! Manages agent template evolution, capability registration with the A2A gateway,
//! system prompt generation, goal context building, and goal alignment evaluation.

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{GoalStatus, SubstrateRequest};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    EvolutionAction, RefinementRequest,
};

use super::types::SwarmEvent;
use super::SwarmOrchestrator;

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Register an agent's capabilities with the A2A registry.
    ///
    /// This allows other agents to discover and communicate with this agent.
    pub async fn register_agent_capabilities(
        &self,
        agent_name: &str,
        capabilities: Vec<String>,
    ) -> DomainResult<()> {
        use crate::domain::models::a2a::A2AAgentCard;

        let mut card = A2AAgentCard::new(agent_name);
        for cap in capabilities {
            card = card.with_capability(cap);
        }

        // Log the registration
        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Agent '{}' registered with {} capabilities",
                agent_name, card.capabilities.len()
            ),
        ).await;

        // If A2A gateway is configured, register the agent card
        if let Some(ref gateway_url) = self.config.mcp_servers.a2a_gateway {
            let register_url = format!("{}/agents", gateway_url.trim_end_matches('/'));

            let client = reqwest::Client::new();
            match client.post(&register_url)
                .json(&card)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            "Agent '{}' card registered with A2A gateway",
                            agent_name
                        );
                    } else {
                        tracing::warn!(
                            "A2A gateway returned error status {} when registering agent '{}'",
                            response.status(), agent_name
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to register agent '{}' with A2A gateway: {}",
                        agent_name, e
                    );
                    // Don't fail the operation - registration is best-effort
                }
            }
        }

        Ok(())
    }

    /// Register all existing agent templates with the A2A gateway at startup.
    ///
    /// This enables agent discovery for all known agent types.
    pub(super) async fn register_all_agent_templates(&self) -> DomainResult<()> {
        // Get all agent templates from the repository
        use crate::domain::ports::AgentFilter;
        let templates = self.agent_repo.list_templates(AgentFilter::default()).await?;

        if templates.is_empty() {
            self.audit_log.info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned,
                "No agent templates to register with A2A gateway".to_string(),
            ).await;
            return Ok(());
        }

        let mut registered_count = 0;
        for template in templates {
            // Extract capabilities from template tools
            let capabilities: Vec<String> = template.tools
                .iter()
                .map(|t| t.name.clone())
                .collect();

            // Add default capability if no tools defined
            let capabilities = if capabilities.is_empty() {
                vec!["task-execution".to_string()]
            } else {
                capabilities
            };

            if self.register_agent_capabilities(&template.name, capabilities).await.is_ok() {
                registered_count += 1;
            }
        }

        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Registered {} agent templates with A2A gateway at startup",
                registered_count
            ),
        ).await;

        Ok(())
    }

    /// Process pending evolution refinement requests.
    ///
    /// Checks for agent templates that need refinement and uses MetaPlanner
    /// to create improved versions based on failure patterns.
    pub(super) async fn process_evolution_refinements(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // First, evaluate all templates to detect any that need refinement
        // This checks success rates, goal violations, and regression patterns
        let evolution_events = self.evolution_loop.evaluate().await;

        // Emit events for any evolution triggers detected
        for event in &evolution_events {
            let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                template_name: event.template_name.clone(),
                trigger: format!("{:?}", event.trigger),
            }).await;

            self.audit_log.info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned,
                format!(
                    "Evolution triggered for '{}': {:?} (success rate: {:.0}%)",
                    event.template_name,
                    event.trigger,
                    event.stats_at_trigger.success_rate * 100.0
                ),
            ).await;
        }

        // Handle revert events — rollback template version
        for event in &evolution_events {
            if let EvolutionAction::Reverted { from_version, to_version } = &event.action_taken {
                if let Ok(Some(mut template)) = self.agent_repo.get_template_by_name(&event.template_name).await {
                    template.version = *to_version;
                    template.system_prompt = format!(
                        "{}\n\n## Reverted (v{} → v{})\n\nReverted due to regression detected after version upgrade.",
                        template.system_prompt, from_version, to_version
                    );

                    if let Ok(_) = self.agent_repo.update_template(&template).await {
                        self.evolution_loop.record_version_change(
                            &event.template_name,
                            *to_version,
                        ).await;

                        let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                            template_name: event.template_name.clone(),
                            trigger: format!("Reverted from v{} to v{}", from_version, to_version),
                        }).await;

                        self.audit_log.info(
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            format!(
                                "Agent '{}' reverted from v{} to v{} due to regression",
                                event.template_name, from_version, to_version
                            ),
                        ).await;
                    }
                }
            }
        }

        // Get pending refinement requests from evolution loop
        let pending_refinements = self.evolution_loop.get_pending_refinements().await;

        for request in pending_refinements {
            // Mark as in progress
            if !self.evolution_loop.start_refinement(request.id).await {
                continue; // Already being processed
            }

            // Log the refinement attempt
            self.audit_log.info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned,
                format!(
                    "Processing evolution refinement for '{}': {:?}",
                    request.template_name, request.severity
                ),
            ).await;

            // Get the current agent template
            let template = match self.agent_repo.get_template_by_name(&request.template_name).await {
                Ok(Some(t)) => t,
                _ => {
                    self.evolution_loop.complete_refinement(request.id, false).await;
                    continue;
                }
            };

            // Create a refined version based on the failure patterns
            // Try LLM-powered refinement via Substrate, fall back to heuristic string-append
            let refined_prompt = if self.overmind.is_some() {
                let refinement_request = SubstrateRequest::new(
                    Uuid::new_v4(),
                    "overmind",
                    "You are an expert prompt engineer. Your task is to improve an agent's system prompt based on its performance data.",
                    format!(
                        "The following agent template has been underperforming and needs refinement.\n\n\
                        ## Current System Prompt\n\n{}\n\n\
                        ## Performance Data\n\n\
                        - Template: {} (v{})\n\
                        - Total tasks: {}\n\
                        - Success rate: {:.0}%\n\
                        - Failed tasks: {}\n\
                        - Trigger: {:?}\n\n\
                        ## Instructions\n\n\
                        Please produce an improved version of the system prompt that addresses the failure patterns. \
                        Return ONLY the improved system prompt text, nothing else. \
                        Keep the core purpose and capabilities intact, but add guidance to prevent the observed failures.",
                        template.system_prompt,
                        request.template_name,
                        template.version,
                        request.stats.total_tasks,
                        request.stats.success_rate * 100.0,
                        request.failed_task_ids.len(),
                        request.trigger,
                    ),
                );

                match self.substrate.execute(refinement_request).await {
                    Ok(session) if session.result.is_some() => {
                        session.result.unwrap()
                    }
                    Ok(_) => {
                        tracing::warn!("LLM refinement returned no result for '{}', falling back to heuristic", request.template_name);
                        Self::heuristic_refinement_prompt(&template.system_prompt, &request)
                    }
                    Err(e) => {
                        tracing::warn!("LLM refinement failed for '{}': {}, falling back to heuristic", request.template_name, e);
                        Self::heuristic_refinement_prompt(&template.system_prompt, &request)
                    }
                }
            } else {
                Self::heuristic_refinement_prompt(&template.system_prompt, &request)
            };

            // Create new version of the template
            let mut new_template = template.clone();
            new_template.version += 1;
            new_template.system_prompt = refined_prompt;

            match self.agent_repo.update_template(&new_template).await {
                Ok(_) => {
                    // Record version change for regression detection
                    self.evolution_loop.record_version_change(
                        &request.template_name,
                        new_template.version,
                    ).await;

                    // Complete the refinement
                    self.evolution_loop.complete_refinement(request.id, true).await;

                    let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                        template_name: request.template_name.clone(),
                        trigger: format!("Refined to v{}", new_template.version),
                    }).await;

                    self.audit_log.info(
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        format!(
                            "Agent '{}' refined to version {}",
                            request.template_name, new_template.version
                        ),
                    ).await;
                }
                Err(e) => {
                    self.evolution_loop.complete_refinement(request.id, false).await;
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!(
                                "Failed to refine agent '{}': {}",
                                request.template_name, e
                            ),
                        ),
                    ).await;
                }
            }
        }

        Ok(())
    }

    /// Generate a heuristic-based refined prompt by appending failure context notes.
    pub(super) fn heuristic_refinement_prompt(current_prompt: &str, request: &RefinementRequest) -> String {
        format!(
            "{}\n\n## Refinement Notes (v{})\n\n\
            Based on {} recent executions with {:.0}% success rate.\n\
            Trigger: {:?}\n\
            {} failed tasks tracked.\n\n\
            Please pay special attention to:\n\
            - Careful validation of inputs and outputs\n\
            - Handling edge cases gracefully\n\
            - Clear error reporting for debugging",
            current_prompt,
            request.template_version + 1,
            request.stats.total_tasks,
            request.stats.success_rate * 100.0,
            request.trigger,
            request.failed_task_ids.len()
        )
    }

    /// Get the system prompt for an agent type, including goal context and API docs.
    pub(super) async fn get_agent_system_prompt(&self, agent_type: &str) -> String {
        let base_prompt = match self.agent_repo.get_template_by_name(agent_type).await {
            Ok(Some(template)) => template.system_prompt.clone(),
            _ => {
                // Default system prompt if agent template not found
                format!(
                    "You are a specialized agent for executing tasks.\n\
                    Follow the task description carefully and complete the work.\n\
                    Agent type: {}",
                    agent_type
                )
            }
        };

        // Append git workflow instructions
        let git_instructions = "\n\n## Git Workflow\n\
            You are working in a git worktree. When you have completed your work:\n\
            1. Stage all changed files with `git add` (be specific about which files you changed)\n\
            2. Create a commit with a descriptive message summarizing what you did and why\n\
            3. Do NOT push to any remote\n\
            \n\
            Your work will not be preserved unless it is committed. Always commit before finishing.";

        let with_git = format!("{}{}", base_prompt, git_instructions);

        // Append tool restrictions to prevent agents from using Claude Code's built-in
        // orchestration tools that bypass Abathur's swarm layer
        let tool_restrictions = "\n\n## Tool Restrictions\n\n\
            You are running inside the Abathur swarm. You MUST NOT use Claude Code's built-in \
            Task, TodoWrite, TeamCreate, TaskCreate, or any other built-in task/team management tools. \
            These bypass Abathur's orchestration and make your work invisible to the swarm.\n\n\
            Instead, use the Abathur MCP tools (task_submit, agent_create, memory_store, etc.) \
            which are available as native tools in your tool list.";

        let with_restrictions = format!("{}{}", with_git, tool_restrictions);

        // Tools are now provided via MCP stdio server — no REST API docs needed.
        // The agent sees task_submit, agent_create, memory_search, etc. as native tools.
        let api_docs = "\n\n## Abathur Tools\n\n\
            You have native tools for interacting with the Abathur swarm:\n\
            - task_submit, task_list, task_get, task_update_status: Manage tasks and subtasks\n\
            - agent_create, agent_list, agent_get: Create and discover agent templates\n\
            - memory_search, memory_store, memory_get: Query and store swarm memory\n\
            - goals_list: View active goals for context\n\n\
            Use these tools directly — do NOT use WebFetch to call HTTP endpoints.";

        let with_apis = format!("{}{}", with_restrictions, api_docs);

        // Append goal context to the system prompt
        let goal_context = self.build_goal_context().await;
        if goal_context.is_empty() {
            with_apis
        } else {
            format!("{}\n\n{}", with_apis, goal_context)
        }
    }

    // build_api_docs removed — tools are now provided via MCP stdio server.
    // The agent sees task_submit, agent_create, memory_search, etc. as native tools.

    /// Refresh the cache of active goals for context injection.
    pub(super) async fn refresh_active_goals_cache(&self) -> DomainResult<()> {
        use crate::domain::ports::GoalFilter;
        let goals = self.goal_repo.list(GoalFilter {
            status: Some(GoalStatus::Active),
            ..Default::default()
        }).await?;
        let mut cache = self.active_goals_cache.write().await;
        *cache = goals;
        Ok(())
    }

    /// Build goal context string for agent prompts.
    pub(super) async fn build_goal_context(&self) -> String {
        let goals = self.active_goals_cache.read().await;
        if goals.is_empty() {
            return String::new();
        }

        let mut context = String::from("\n## Active Goals Context\n\n");
        context.push_str("Your work must align with these active goals:\n\n");

        for goal in goals.iter() {
            context.push_str(&format!("### {} (Priority: {:?})\n", goal.name, goal.priority));
            context.push_str(&format!("{}\n", goal.description));

            if !goal.constraints.is_empty() {
                context.push_str("\n**Constraints:**\n");
                for constraint in &goal.constraints {
                    context.push_str(&format!("- {}: {}\n", constraint.name, constraint.description));
                }
            }
            context.push('\n');
        }

        context.push_str("Ensure your implementation satisfies all constraints and contributes to these goals.\n");
        context
    }
}

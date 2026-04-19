//! Goal processing subsystem for the swarm orchestrator.
//!
//! Handles task spawning for ready tasks, dependency management,
//! task readiness updates, and retry logic.
//!
//! Goals no longer decompose into tasks or own tasks. Instead, goals provide
//! aspirational guidance via GoalContextService when tasks are executed.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

use crate::domain::errors::DomainResult;
use crate::domain::models::{
    AgentTier,
    RelevanceWeights, ScoredMemory,
    SessionStatus, SubstrateConfig, SubstrateRequest,
    Task, TaskStatus,
};
use crate::services::memory_service::MemoryService;
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AgentTierHint, AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    CircuitScope, GoalContextService, ModelRouter,
    TaskExecution, TaskOutcome,
    command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand},
};

use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::workflow_template::WorkflowTemplate;

use super::helpers::{auto_commit_worktree, run_post_completion_workflow};
use super::types::SwarmEvent;
use super::SwarmOrchestrator;

/// Re-emit `WorkflowGateRejected` for tasks that were rejected via MCP.
///
/// When an overmind agent calls `workflow_gate(reject)`, the event is emitted
/// on the MCP session's local event bus (no handlers). The orchestrator must
/// re-emit it so `AdapterLifecycleSyncHandler` can fire egress actions.
async fn replay_gate_rejection_event(
    task: &Task,
    event_bus: &crate::services::event_bus::EventBus,
    workflows: &[WorkflowTemplate],
) {
    let state = match task.context.custom.get("workflow_state")
        .and_then(|v| serde_json::from_value::<WorkflowState>(v.clone()).ok())
    {
        Some(s) => s,
        None => return,
    };

    if let WorkflowState::Rejected { workflow_name, phase_index, reason } = state {
        let phase_name = workflows
            .iter()
            .find(|t| t.name == workflow_name)
            .and_then(|t| t.phases.get(phase_index))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("phase_{}", phase_index));

        event_bus.publish(crate::services::event_factory::make_event(
            crate::services::event_bus::EventSeverity::Warning,
            crate::services::event_bus::EventCategory::Workflow,
            None,
            Some(task.id),
            crate::services::event_bus::EventPayload::WorkflowGateRejected {
                task_id: task.id,
                phase_index,
                phase_name,
                reason,
            },
        )).await;
    }
}

/// Map agent template tool names (lowercase YAML) to Claude Code CLI tool names.
///
/// Template tools like "read", "shell", "memory" need to be translated to
/// the PascalCase names that `claude --allowedTools` expects.
/// Tools like "memory" and "tasks" are Abathur MCP tools, mapped to specific
/// `mcp__abathur__*` tool names. Use "task_status" for worker agents
/// (only task_update_status + task_get) and "tasks" for orchestrators.
fn map_template_tools_to_cli(template_tool_names: &[String]) -> Vec<String> {
    let mut cli_tools = Vec::new();

    for tool in template_tool_names {
        match tool.as_str() {
            "read" => cli_tools.push("Read".to_string()),
            "write" => {
                cli_tools.push("Write".to_string());
            }
            "edit" => {
                cli_tools.push("Edit".to_string());
                cli_tools.push("MultiEdit".to_string());
            }
            "shell" => cli_tools.push("Bash".to_string()),
            "glob" => cli_tools.push("Glob".to_string()),
            "grep" => cli_tools.push("Grep".to_string()),
            // Abathur APIs are provided via MCP stdio server as native tools.
            // Claude Code still needs these in --allowedTools to use them in headless mode.
            "memory" => {
                cli_tools.push("mcp__abathur__memory_search".to_string());
                cli_tools.push("mcp__abathur__memory_store".to_string());
                cli_tools.push("mcp__abathur__memory_get".to_string());
            }
            "tasks" => {
                cli_tools.push("mcp__abathur__task_submit".to_string());
                cli_tools.push("mcp__abathur__task_list".to_string());
                cli_tools.push("mcp__abathur__task_get".to_string());
                cli_tools.push("mcp__abathur__task_update_status".to_string());
                cli_tools.push("mcp__abathur__task_assign".to_string());
                cli_tools.push("mcp__abathur__task_wait".to_string());
                cli_tools.push("mcp__abathur__goals_list".to_string());
                cli_tools.push("mcp__abathur__workflow_select".to_string());
                cli_tools.push("mcp__abathur__workflow_advance".to_string());
                cli_tools.push("mcp__abathur__workflow_fan_out".to_string());
                cli_tools.push("mcp__abathur__workflow_gate".to_string());
                cli_tools.push("mcp__abathur__workflow_status".to_string());
                cli_tools.push("mcp__abathur__task_cancel".to_string());
                cli_tools.push("mcp__abathur__task_retry".to_string());
            }
            "task_status" => {
                cli_tools.push("mcp__abathur__task_update_status".to_string());
                cli_tools.push("mcp__abathur__task_get".to_string());
            }
            "agents" => {
                cli_tools.push("mcp__abathur__agent_create".to_string());
                cli_tools.push("mcp__abathur__agent_list".to_string());
                cli_tools.push("mcp__abathur__agent_get".to_string());
            }
            // Pass through any already-PascalCase tool names, but reject blocked tools
            other => {
                const BLOCKED: &[&str] = &[
                    "task", "todowrite", "todoread", "taskcreate", "taskupdate",
                    "tasklist", "taskget", "taskstop", "taskoutput",
                    "teamcreate", "teamdelete", "sendmessage",
                    "enterplanmode", "exitplanmode", "skill", "notebookedit",
                ];
                if BLOCKED.contains(&other.to_lowercase().as_str()) {
                    tracing::warn!("Agent template requested blocked tool '{}' - skipping", other);
                } else {
                    cli_tools.push(other.to_string());
                }
            }
        }
    }

    // Inject baseline read-only tools for agents that interact with code.
    // Orchestration-only agents (overmind, aggregator) should NOT get these —
    // they delegate to workers instead of exploring the codebase themselves.
    let is_orchestration_only = template_tool_names.iter().all(|t| {
        matches!(t.as_str(), "memory" | "tasks" | "agents" | "task_status" | "egress_publish")
    });
    if !is_orchestration_only {
        for baseline in &["Read", "Glob", "Grep"] {
            if !cli_tools.contains(&baseline.to_string()) {
                cli_tools.push(baseline.to_string());
            }
        }
    }

    cli_tools.sort();
    cli_tools.dedup();
    cli_tools
}

/// Format scored memories as contextual guidance text for agent task prompts.
fn format_memory_context(memories: &[ScoredMemory]) -> String {
    let mut output = String::from(
        "## Relevant Context from Memory\nThe following memories from previous work are relevant to this task:\n\n",
    );
    for entry in memories {
        let mem = &entry.memory;
        output.push_str(&format!(
            "**{}** *(tier: {}, score: {:.2})*\n{}\n\n",
            mem.key, mem.tier.as_str(), entry.score, mem.content,
        ));
    }
    output
}

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Process ready tasks by spawning agents for them.
    ///
    /// Goals no longer decompose into tasks. Tasks are created independently
    /// (by humans, system triggers, or goal evaluation service). This method
    /// simply finds ready tasks and spawns agents to execute them.
    pub(super) async fn process_goals(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Get ready tasks and spawn agents for them
        let ready_tasks = self.task_repo.get_ready_tasks(self.config.max_agents).await?;

        for task in &ready_tasks {
            self.spawn_task_agent(task, event_tx).await?;
        }

        Ok(())
    }

    /// Like `process_goals` but skips tasks already attempted in the current drain cycle.
    pub(super) async fn process_goals_excluding(
        &self,
        event_tx: &mpsc::Sender<SwarmEvent>,
        already_spawned: &std::collections::HashSet<uuid::Uuid>,
    ) -> DomainResult<()> {
        let ready_tasks = self.task_repo.get_ready_tasks(self.config.max_agents).await?;

        for task in &ready_tasks {
            if !already_spawned.contains(&task.id) {
                self.spawn_task_agent(task, event_tx).await?;
            }
        }

        Ok(())
    }

    /// Spawn an agent for a ready task.
    ///
    /// Runs the registered pre-spawn middleware chain (routing, circuit
    /// breaker, quiet-window, budget gates, guardrails, etc.); on `Continue`
    /// acquires an agent permit and invokes the substrate. On `Skip` returns
    /// without spawning — the task stays `Ready` for the next cycle.
    pub(super) async fn spawn_task_agent(
        &self,
        task: &Task,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        use super::middleware::{PreSpawnContext, PreSpawnDecision};

        // Build the pre-spawn context. Repos are coerced to trait objects so
        // middleware can operate without being generic over the orchestrator.
        let task_repo: Arc<dyn crate::domain::ports::TaskRepository> = self.task_repo.clone();
        let agent_repo: Arc<dyn crate::domain::ports::AgentRepository> = self.agent_repo.clone();
        let goal_repo: Arc<dyn crate::domain::ports::GoalRepository> = self.goal_repo.clone();

        let mut ctx = PreSpawnContext {
            task: task.clone(),
            agent_type: None,
            task_repo,
            agent_repo,
            goal_repo,
            audit_log: self.audit_log.clone(),
            circuit_breaker: self.circuit_breaker.clone(),
            guardrails: self.guardrails.clone(),
            cost_window_service: self.cost_window_service.clone(),
            budget_tracker: self.budget_tracker.clone(),
            agent_semaphore: self.agent_semaphore.clone(),
            max_agents: self.config.max_agents,
            federation_priority_bumps: 0,
        };

        // Run the registered pre-spawn middleware. Each middleware may
        // short-circuit with a Skip decision (logged below) or enrich the
        // context (e.g. RouteTaskMiddleware sets ctx.agent_type).
        let decision = {
            let chain = self.pre_spawn_chain.read().await;
            chain.run(&mut ctx).await?
        };

        if let PreSpawnDecision::Skip { reason } = decision {
            tracing::debug!(
                task_id = %task.id,
                %reason,
                "spawn_task_agent: pre-spawn chain requested skip"
            );
            return Ok(());
        }

        // Routing middleware is required to have populated agent_type before
        // we reach substrate invocation. If it didn't, the chain is broken.
        let agent_type = ctx.agent_type.clone().ok_or_else(|| {
            crate::domain::errors::DomainError::ExecutionFailed(
                "pre-spawn chain completed without resolving an agent_type — \
                 RouteTaskMiddleware must be registered".to_string(),
            )
        })?;

        // Circuit-breaker scope is needed later for recording success/failure
        // outcomes on the spawned task. The gate check already ran in the
        // pre-spawn chain; this just recreates the scope value.
        let scope = CircuitScope::agent(&agent_type);
        let agent_unique_id = task.id.to_string();

        // Try to acquire agent permit
        if let Ok(permit) = self.agent_semaphore.clone().try_acquire_owned() {
            // Atomically claim the task (Ready→Running) BEFORE spawning.
            // This prevents TOCTOU races where multiple poll cycles see the
            // same Ready task and spawn duplicate agents.
            match self.task_repo.claim_task_atomic(task.id, &agent_type).await {
                Ok(None) => {
                    // Task was already claimed by another cycle — nothing to do
                    tracing::debug!("Task {} already claimed, skipping spawn", task.id);
                    drop(permit);
                    return Ok(());
                }
                Ok(Some(_)) => {
                    // Register agent spawn with guardrails using unique task_id
                    self.guardrails.register_agent_spawn(&agent_unique_id).await;

                    // Successfully claimed — publish event and continue to spawn
                    self.event_bus.publish(crate::services::event_factory::task_event(
                        crate::services::event_bus::EventSeverity::Info,
                        None,
                        task.id,
                        crate::services::event_bus::EventPayload::TaskClaimed {
                            task_id: task.id,
                            agent_type: agent_type.clone(),
                        },
                    )).await;

                    // Workflow stays in Pending state — the Overmind decides
                    // whether to workflow_advance (single subtask) or
                    // workflow_fan_out (parallel slices) for the first phase.
                }
                Err(e) => {
                    tracing::warn!("Failed to atomically claim task {}: {}", task.id, e);
                    drop(permit);
                    return Ok(());
                }
            }

            let system_prompt = self.get_agent_system_prompt(&agent_type).await;

            // Get agent template for version tracking, capabilities, and tool restrictions
            let (template_version, capabilities, cli_tools, agent_can_write, is_template_read_only, template_max_turns, template_preferred_model, template_tier) = match self.agent_repo.get_template_by_name(&agent_type).await {
                Ok(Some(template)) => {
                    let caps: Vec<String> = template.tools.iter()
                        .map(|t| t.name.clone())
                        .collect();
                    let tools = map_template_tools_to_cli(&caps);
                    let can_write = caps.iter().any(|c| {
                        let lower = c.to_lowercase();
                        lower == "write" || lower == "edit" || lower == "shell"
                    });
                    (template.version, caps, tools, can_write, template.read_only, template.max_turns, template.preferred_model.clone(), template.tier)
                }
                // Default to true when template lookup fails (safer to require commits from unknown agents)
                _ => (1, vec!["task-execution".to_string()], vec![], true, false, 0, None, AgentTier::Worker),
            };

            // Read-only agent roles never produce commits regardless of tool capabilities.
            // The template's `read_only` field is the primary signal (set at creation time).
            // The name-based heuristic is kept as a legacy fallback for agents created
            // before the `read_only` field existed.
            let is_read_only_role = is_template_read_only || {
                let lower = agent_type.to_lowercase();
                lower == "overmind"
                    || lower == "aggregator"
                    || lower.contains("researcher")
                    || lower.contains("planner")
                    || lower.contains("analyst")
                    || lower.contains("architect")
            };

            // Register agent capabilities with A2A gateway if configured
            if self.config.mcp_servers.a2a_gateway.is_some()
                && let Err(e) = self.register_agent_capabilities(&agent_type, capabilities).await {
                    tracing::warn!("Failed to register agent '{}' capabilities: {}", agent_type, e);
                }

            // Publish TaskSpawned via EventBus (bridge forwards to event_tx)
            self.event_bus.publish(crate::services::event_factory::task_event(
                crate::services::event_bus::EventSeverity::Info,
                None,
                task.id,
                crate::services::event_bus::EventPayload::TaskSpawned {
                    task_id: task.id,
                    task_title: task.title.clone(),
                    agent_type: Some(agent_type.clone()),
                },
            )).await;

            // Resolve workflow template for this task to determine workspace kind and
            // output delivery mode. SwarmConfig.workflow_template is populated at
            // startup from the resolved config; see `cli::commands::swarm`.
            let task_workflow = self
                .config
                .workflow_template
                .clone()
                .expect("workflow_template must be resolved at swarm startup");
            let task_workspace_kind = task_workflow.workspace_kind;
            let task_output_delivery = task_workflow.output_delivery.clone();

            // Provision workspace based on workflow's WorkspaceKind.
            // WorkspaceKind::Worktree → git worktree (existing behaviour)
            // WorkspaceKind::TempDir  → plain temp directory (no git)
            // WorkspaceKind::None     → no workspace (read-only agents)
            let worktree_path = self
                .provision_workspace_for_task(task.id, task_workspace_kind)
                .await;

            // Write CLAUDE.md to worktree with tool restrictions.
            // Claude Code reads CLAUDE.md as project-level instructions.
            if let Some(ref wt_path) = worktree_path {
                let claude_md_path = std::path::Path::new(wt_path).join("CLAUDE.md");
                let claude_md_content = "\
# Abathur Agent Rules

IMPORTANT: You are running inside the Abathur swarm orchestration system.

## Prohibited Tools
NEVER use these Claude Code built-in tools — they bypass Abathur's orchestration:
- Task (subagent spawner)
- TodoWrite / TodoRead
- TaskCreate, TaskUpdate, TaskList, TaskGet, TaskStop, TaskOutput
- TeamCreate, TeamDelete, SendMessage
- EnterPlanMode, ExitPlanMode
- Skill
- NotebookEdit

## How to manage work
- Advance workflow: Use `workflow_advance` or `workflow_fan_out` to create phase subtasks
- Change spine: Use `workflow_select` before first advance (if auto-selected spine is wrong)
- Cancel tasks: Use `task_cancel` to stop work that is no longer needed
- Retry failed tasks: Use `task_retry` to reset a failed task to Ready
- Create agents: Use the `agent_create` tool directly
- Track progress: Use `task_list` and `task_get` tools
- Store learnings: Use the `memory_store` tool directly

## Efficiency Rules
- Use Glob for file discovery — never shell ls or find.
- Use Grep to search code — never Read entire files looking for a pattern.
- NEVER re-read a file you already read this session.
- Store findings incrementally via memory_store as you go, not all at the end.
- When done, call task_update_status immediately — no self-verification reads.
- If retrying a task, call memory_search FIRST to find prior work and build on it.
";
                if let Err(e) = std::fs::write(&claude_md_path, claude_md_content) {
                    tracing::warn!("Failed to write CLAUDE.md to worktree: {}", e);
                } else {
                    tracing::debug!("Wrote CLAUDE.md with tool restrictions to {:?}", claude_md_path);
                }

                // Bootstrap .claude/settings.json in worktree.
                // We write the permissions block directly (no mcpServers —
                // the orchestrator provides MCP via --mcp-config with absolute paths).
                let claude_dir = std::path::Path::new(wt_path).join(".claude");
                let _ = std::fs::create_dir_all(&claude_dir);
                let tools: Vec<serde_json::Value> = crate::ABATHUR_ALLOWED_TOOLS
                    .iter()
                    .map(|t| serde_json::Value::String(t.to_string()))
                    .collect();
                let settings_content = serde_json::json!({
                    "permissions": {
                        "allowedTools": tools
                    }
                });
                if let Ok(pretty) = serde_json::to_string_pretty(&settings_content)
                    && let Err(e) = std::fs::write(claude_dir.join("settings.json"), format!("{pretty}\n")) {
                        tracing::warn!("Failed to write .claude/settings.json to worktree: {}", e);
                    }
            }

            // Load relevant goal context for the task
            let goal_context_service = GoalContextService::new(self.goal_repo.clone());
            let goal_context = match goal_context_service.get_goals_for_task(task).await {
                Ok(goals) if !goals.is_empty() => {
                    let context_text = GoalContextService::<G>::format_goal_context(&goals);
                    self.audit_log.info(
                        AuditCategory::Goal,
                        AuditAction::GoalEvaluated,
                        format!(
                            "Task {} received guidance from {} relevant goal(s)",
                            task.id, goals.len()
                        ),
                    ).await;
                    Some(context_text)
                }
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!("Failed to load goal context for task {}: {}", task.id, e);
                    None
                }
            };

            // Load relevant memory context for the task using budget-aware selection.
            let memory_context = if let Some(ref mem_repo) = self.memory_repo {
                let memory_service = MemoryService::new(mem_repo.clone());
                let desc_preview: String = task.description.chars().take(500).collect();
                let query = format!("{} {}", task.title, desc_preview);
                match memory_service.load_context_with_budget(
                    &query,
                    None,
                    2000, // 25% of 8000-token context budget
                    RelevanceWeights::semantic_biased(),
                ).await {
                    Ok(memories) if !memories.is_empty() => Some(format_memory_context(&memories)),
                    Ok(_) => None,
                    Err(e) => {
                        tracing::debug!(task_id = %task.id, "Failed to load memory context: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            // Build the task description: goal context first, memory context second, task prompt last.
            let task_description = {
                let mut parts: Vec<&str> = Vec::new();
                if let Some(ref ctx) = goal_context { parts.push(ctx.as_str()); }
                if let Some(ref ctx) = memory_context { parts.push(ctx.as_str()); }
                // Include intent gap context from a previous attempt if present.
                let intent_gap_ctx = task.context.custom.get("intent_gap_context")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(ref gap_ctx) = intent_gap_ctx { parts.push(gap_ctx.as_str()); }
                if parts.is_empty() {
                    task.description.clone()
                } else {
                    format!("{}\n\n---\n\n{}", parts.join("\n\n---\n\n"), task.description)
                }
            };

            // Spawn task execution
            let task_id = task.id;

            // Runtime upgrade: if the stored mode is Direct but the agent is
            // write-capable and non-read-only, upgrade to Convergent when
            // convergence is enabled. This applies to both standalone tasks
            // and workflow subtasks — read-only phases are already protected
            // by the is_read_only_role and agent_can_write checks.
            let effective_mode = if task.execution_mode.is_direct()
                && self.config.convergence_enabled
                && !is_read_only_role
                && agent_can_write
            {
                tracing::info!(
                    task_id = %task_id,
                    %agent_type,
                    "Upgrading execution mode Direct -> Convergent (write-capable, non-read-only agent)"
                );
                crate::domain::models::ExecutionMode::Convergent { parallel_samples: None }
            } else {
                task.execution_mode.clone()
            };

            let is_convergent = effective_mode.is_convergent()
                && self.config.convergence_enabled
                && !is_read_only_role;

            tracing::info!(
                task_id = %task_id,
                %agent_type,
                stored_mode = ?task.execution_mode,
                effective_mode = ?effective_mode,
                convergence_enabled = self.config.convergence_enabled,
                is_read_only = is_read_only_role,
                agent_can_write = agent_can_write,
                will_converge = is_convergent,
                "Task execution mode resolved"
            );

            let task_clone = task.clone();
            let substrate = self.substrate.clone();
            let task_repo = self.task_repo.clone();
            let worktree_repo = self.worktree_repo.clone();
            // Trait-object clones for post-completion chain / helpers that
            // require Arc<dyn ...> so they aren't generic over the orchestrator.
            let post_task_repo: Arc<dyn crate::domain::ports::TaskRepository> =
                self.task_repo.clone();
            let post_goal_repo: Arc<dyn crate::domain::ports::GoalRepository> =
                self.goal_repo.clone();
            let post_worktree_repo: Arc<dyn crate::domain::ports::WorktreeRepository> =
                self.worktree_repo.clone();
            let post_completion_chain = self.post_completion_chain.clone();
            let event_tx = event_tx.clone();
            let event_bus = self.event_bus.clone();
            let all_workflows = self.config.all_workflows.clone();
            let command_bus = self.command_bus.read().await.clone();
            // Role-aware max_turns defaults — the ceiling should be 2-3x
            // typical usage so agents aren't cut short on complex tasks.
            let role_max_turns = {
                let lower = agent_type.to_lowercase();
                if lower.contains("researcher") || lower.contains("analyst")
                    || lower.contains("explorer") || lower.contains("auditor")
                {
                    51  // Research: typical ~15 turns, ceiling 51
                } else if lower.contains("planner") || lower.contains("architect")
                    || lower.contains("designer")
                    || lower.contains("reviewer") || lower.contains("verifier")
                {
                    30  // Planning/Review: typical ~10 turns, ceiling 30
                } else if lower.contains("implement") || lower.contains("coder")
                    || lower.contains("builder") || lower.contains("fixer")
                {
                    75  // Implementation: typical ~25 turns, ceiling 75
                } else {
                    self.config.default_max_turns  // Fallback to config default
                }
            };

            // Use agent template's max_turns if explicitly set (non-zero),
            // then role-aware default, then orchestrator config default.
            let mut max_turns = if template_max_turns > 0 {
                template_max_turns.max(role_max_turns)
            } else {
                role_max_turns
            };

            // Bump turn budget for tasks retrying after max_turns exhaustion.
            // Increase by 50% per retry, capped to prevent unbounded growth.
            if task.context.hints.iter().any(|h| h == "retry:max_turns_exceeded") {
                let multiplier = 1.5_f64.powi(task.retry_count as i32);
                max_turns = ((max_turns as f64 * multiplier) as u32).min(100);
            }
            let total_tokens = self.total_tokens.clone();
            let circuit_breaker = self.circuit_breaker.clone();
            let audit_log = self.audit_log.clone();
            let evolution_loop = self.evolution_loop.clone();
            let track_evolution = self.config.track_evolution;
            let agent_type_for_evolution = agent_type.clone();
            let template_version_for_evolution = template_version;
            let verify_on_completion = self.config.verify_on_completion;
            // Without --dangerously-skip-permissions, disable direct merges to main
            // and force PR-only mode so a human must approve before merging.
            let use_merge_queue = self.config.use_merge_queue
                && self.config.dangerously_skip_permissions;
            let prefer_pull_requests = self.config.prefer_pull_requests
                || !self.config.dangerously_skip_permissions;
            let repo_path = self.config.repo_path.clone();
            let default_base_ref = self.config.default_base_ref.clone();
            let fetch_on_sync = self.config.fetch_on_sync;
            let require_commits = agent_can_write && !is_read_only_role;
            // Clone output_delivery so it can be moved into the spawn block.
            let output_delivery_for_spawn = task_output_delivery.clone();
            let circuit_scope = scope;

            // Convergence infrastructure (cloned into spawn block only when needed)
            let overseer_cluster = self.overseer_cluster.clone();
            let trajectory_repo = self.trajectory_repo.clone();
            let convergence_engine_config = self.convergence_engine_config.clone();
            let memory_repo = self.memory_repo.clone();
            let guardrails = self.guardrails.clone();
            let agent_unique_id_for_spawn = agent_unique_id.clone();
            let merge_request_repo = self.merge_request_repo.clone();

            // Cast the generic IntentVerifierService to the trait object for
            // convergent execution. This erases the <G, T> generics.
            // Intent verification is required for convergent execution.
            let convergent_intent_verifier: Option<Arc<
                dyn super::convergent_execution::ConvergentIntentVerifier,
            >> = self.intent_verifier.as_ref().map(|iv| {
                Arc::clone(iv) as Arc<dyn super::convergent_execution::ConvergentIntentVerifier>
            });

            tokio::spawn(async move {
                let _permit = permit;
                let output_delivery = output_delivery_for_spawn;

                // Task is already Running (claimed atomically before spawn).

                // -----------------------------------------------------------------
                // Convergent execution path (Phase 3)
                // -----------------------------------------------------------------
                if is_convergent {
                    // Validate that all convergence infrastructure is available.
                    // If any piece is missing, fall back to direct execution with a warning.
                    let can_converge = overseer_cluster.is_some()
                        && trajectory_repo.is_some()
                        && memory_repo.is_some()
                        && convergent_intent_verifier.is_some();

                    if can_converge {
                        let overseer_cluster = overseer_cluster.unwrap();
                        let trajectory_repo_arc = trajectory_repo.unwrap();
                        let memory_repo = memory_repo.unwrap();
                        let convergent_intent_verifier = convergent_intent_verifier.unwrap();

                        // Wrap the dyn TrajectoryRepository in a Sized newtype so
                        // it can satisfy the generic T parameter of ConvergenceEngine.
                        let trajectory_repo_wrapped = Arc::new(
                            crate::services::convergence_bridge::DynTrajectoryRepository(
                                trajectory_repo_arc,
                            ),
                        );

                        // Build or reuse convergence engine config
                        let engine_config = convergence_engine_config.unwrap_or_else(|| {
                            crate::services::convergence_bridge::build_engine_config_from_defaults()
                        });

                        // Construct the convergence engine for this task.
                        // The engine takes ownership of Arcs; clone so we can also
                        // pass the trajectory repo to run_convergent_execution.
                        let engine = crate::services::convergence_engine::ConvergenceEngine::new(
                            trajectory_repo_wrapped.clone(),
                            memory_repo,
                            overseer_cluster,
                            engine_config,
                        );

                        // Resolve goal_id for event correlation (best-effort)
                        let goal_id: Option<uuid::Uuid> = None; // Tasks don't carry goal_id; bridge handles None

                        // Create worktree ONCE for the entire convergence loop
                        // (not recreated between iterations).
                        // Use worktree_path.is_some() to detect whether a workspace was
                        // provisioned (replaces the old use_worktrees config flag check).
                        let convergent_worktree_path = if worktree_path.is_some() {
                            match worktree_repo.get_by_task(task_id).await {
                                Ok(Some(wt)) => Some(wt.path.clone()),
                                _ => worktree_path.clone(),
                            }
                        } else {
                            None
                        };

                        audit_log.log(
                            crate::services::AuditEntry::new(
                                AuditLevel::Info,
                                AuditCategory::Execution,
                                AuditAction::TaskCompleted, // reuse; no dedicated convergence action
                                AuditActor::System,
                                format!(
                                    "Task {} entering convergent execution (mode: {:?})",
                                    task_id, task_clone.execution_mode
                                ),
                            )
                            .with_entity(task_id, "task"),
                        ).await;

                        // Create a cancellation token for this convergent execution.
                        // Currently not wired to external cancellation signals, but
                        // provides the mechanism for graceful shutdown propagation.
                        let cancellation_token = tokio_util::sync::CancellationToken::new();

                        // Apply SLA deadline if the task has one
                        let deadline = task_clone.deadline;

                        // Run the convergent execution loop.
                        // If the task requests parallel samples AND worktrees are
                        // available, dispatch to the parallel path; otherwise use
                        // the sequential convergent loop.
                        let outcome = if let crate::domain::models::ExecutionMode::Convergent {
                            parallel_samples: Some(n),
                        } = &effective_mode
                        {
                            if worktree_path.is_some() {
                                super::convergent_execution::run_parallel_convergent_execution(
                                    &task_clone,
                                    goal_id,
                                    &substrate,
                                    &task_repo,
                                    &trajectory_repo_wrapped,
                                    &engine,
                                    &event_bus,
                                    &agent_type,
                                    &system_prompt,
                                    max_turns,
                                    cancellation_token,
                                    deadline,
                                    *n,
                                    &default_base_ref,
                                    &format!(
                                        "{}/convergent_parallel_{}",
                                        repo_path.display(),
                                        task_id
                                    ),
                                    convergent_intent_verifier.clone(),
                                )
                                .await
                            } else {
                                tracing::warn!(
                                    task_id = %task_id,
                                    parallel_samples = n,
                                    "Parallel convergent mode requested but worktrees disabled; falling back to sequential"
                                );
                                super::convergent_execution::run_convergent_execution(
                                    &task_clone,
                                    goal_id,
                                    &substrate,
                                    &task_repo,
                                    &trajectory_repo_wrapped,
                                    &engine,
                                    &event_bus,
                                    &agent_type,
                                    &system_prompt,
                                    convergent_worktree_path.as_deref(),
                                    max_turns,
                                    cancellation_token,
                                    deadline,
                                    convergent_intent_verifier.clone(),
                                )
                                .await
                            }
                        } else {
                            super::convergent_execution::run_convergent_execution(
                                &task_clone,
                                goal_id,
                                &substrate,
                                &task_repo,
                                &trajectory_repo_wrapped,
                                &engine,
                                &event_bus,
                                &agent_type,
                                &system_prompt,
                                convergent_worktree_path.as_deref(),
                                max_turns,
                                cancellation_token,
                                deadline,
                                convergent_intent_verifier,
                            )
                            .await
                        };

                        // Auto-commit safety net after convergence terminates
                        if let Some(ref wt_path) = convergent_worktree_path {
                            let _ = auto_commit_worktree(wt_path, task_id).await;
                        }

                        // Store convergent outcome in task context (Step 5.1)
                        // so workflow verification can skip redundant checks.
                        {
                            let outcome_str = match &outcome {
                                Ok(super::convergent_execution::ConvergentOutcome::Converged) => "converged",
                                Ok(super::convergent_execution::ConvergentOutcome::IndeterminateAccepted) => "indeterminate_accepted",
                                Ok(super::convergent_execution::ConvergentOutcome::PartialAccepted) => "partial_accepted",
                                Ok(super::convergent_execution::ConvergentOutcome::IntentGapsFound(_)) => "intent_gaps_found",
                                Ok(super::convergent_execution::ConvergentOutcome::Decomposed(_)) => "decomposed",
                                Ok(super::convergent_execution::ConvergentOutcome::Failed(_)) => "failed",
                                Ok(super::convergent_execution::ConvergentOutcome::Cancelled) => "cancelled",
                                Err(_) => "error",
                            };
                            if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                t.context.custom.insert(
                                    "convergence_outcome".to_string(),
                                    serde_json::json!(outcome_str),
                                );
                                let _ = task_repo.update(&t).await;
                            }
                        }

                        // Map ConvergentOutcome to task status transitions
                        match outcome {
                            Ok(ref convergent_outcome @ super::convergent_execution::ConvergentOutcome::Converged)
                            | Ok(ref convergent_outcome @ super::convergent_execution::ConvergentOutcome::IndeterminateAccepted)
                            | Ok(ref convergent_outcome @ super::convergent_execution::ConvergentOutcome::PartialAccepted) => {
                                let intent_satisfied = !matches!(
                                    convergent_outcome,
                                    super::convergent_execution::ConvergentOutcome::IndeterminateAccepted
                                );

                                // Check if the agent already completed the task via MCP
                                // (e.g. task_update_status tool). If so, the TaskCompleted
                                // event was already published and downstream handlers
                                // (WorkflowSubtaskCompletionHandler) have already reacted.
                                let current_task = task_repo.get(task_id).await.ok().flatten();
                                let already_terminal = current_task.as_ref()
                                    .is_some_and(|t| t.status.is_terminal());

                                if !already_terminal {
                                    // When convergent execution has already verified intent
                                    // satisfaction, skip the Validating intermediate state
                                    // and go directly to Complete. The Validating→Complete
                                    // transition in run_post_completion_workflow only sends
                                    // SwarmEvent (CLI display), not EventPayload::TaskCompleted
                                    // to the EventBus, so WorkflowSubtaskCompletionHandler
                                    // never fires and the workflow stalls.
                                    let target_status = if verify_on_completion && !intent_satisfied {
                                        // Don't auto-complete to Validating if the task
                                        // has a non-terminal workflow — that creates an
                                        // illegal Validating+PhaseReady deadlock.
                                        if current_task.as_ref().is_some_and(|t| !can_safely_auto_complete(t)) {
                                            tracing::warn!(
                                                task_id = %task_id,
                                                "Overmind exhausted turns mid-workflow — failing instead of auto-completing to Validating"
                                            );
                                            TaskStatus::Failed
                                        } else {
                                            TaskStatus::Validating
                                        }
                                    } else {
                                        TaskStatus::Complete
                                    };

                                    if let Some(ref cb) = command_bus {
                                        let envelope = CommandEnvelope::new(
                                            CommandSource::System,
                                            DomainCommand::Task(TaskCommand::Transition {
                                                task_id,
                                                new_status: target_status,
                                            }),
                                        );
                                        if let Err(e) = cb.dispatch(envelope).await {
                                            tracing::warn!(
                                                "Failed to complete convergent task {} via CommandBus: {}",
                                                task_id, e
                                            );
                                            if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                                && !t.status.is_terminal() {
                                                    let _ = t.transition_to(target_status);
                                                    let _ = task_repo.update(&t).await;
                                                }
                                            // Only emit TaskCompleted when actually completing.
                                            // When target is Validating, the verification workflow
                                            // in run_post_completion_workflow will emit the event
                                            // after transitioning Validating -> Complete.
                                            if target_status == TaskStatus::Complete {
                                                event_bus.publish(crate::services::event_factory::task_event(
                                                    crate::services::event_bus::EventSeverity::Info,
                                                    None,
                                                    task_id,
                                                    crate::services::event_bus::EventPayload::TaskCompleted {
                                                        task_id,
                                                        tokens_used: 0,
                                                    },
                                                )).await;
                                            }
                                        }
                                    } else if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                        if !t.status.is_terminal() {
                                            let _ = t.transition_to(target_status);
                                            let _ = task_repo.update(&t).await;
                                        }
                                        // Only emit TaskCompleted when actually completing.
                                        // When target is Validating, the verification workflow
                                        // in run_post_completion_workflow will emit the event
                                        // after transitioning Validating -> Complete.
                                        if target_status == TaskStatus::Complete {
                                            event_bus.publish(crate::services::event_factory::task_event(
                                                crate::services::event_bus::EventSeverity::Info,
                                                None,
                                                task_id,
                                                crate::services::event_bus::EventPayload::TaskCompleted {
                                                    task_id,
                                                    tokens_used: 0,
                                                },
                                            )).await;
                                        }
                                    }
                                } else {
                                    tracing::debug!(
                                        task_id = %task_id,
                                        status = ?current_task.as_ref().map(|t| t.status),
                                        "Skipping convergent task transition — already terminal (completed via MCP)"
                                    );
                                }

                                circuit_breaker.record_success(circuit_scope.clone()).await;

                                // Mark worktree as completed
                                if worktree_path.is_some()
                                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.complete();
                                        let _ = worktree_repo.update(&wt).await;
                                    }

                                // Run post-completion workflow (verify, merge, PR)
                                if verify_on_completion || use_merge_queue || prefer_pull_requests {
                                    let _ = run_post_completion_workflow(
                                        task_id,
                                        post_task_repo.clone(),
                                        post_goal_repo.clone(),
                                        post_worktree_repo.clone(),
                                        &event_tx,
                                        &event_bus,
                                        &audit_log,
                                        verify_on_completion,
                                        use_merge_queue,
                                        prefer_pull_requests,
                                        &repo_path,
                                        &default_base_ref,
                                        require_commits,
                                        intent_satisfied,
                                        output_delivery.clone(),
                                        merge_request_repo.clone(),
                                        fetch_on_sync,
                                        post_completion_chain.clone(),
                                    ).await;
                                }

                                // Record success in evolution loop for template improvement
                                if track_evolution {
                                    let execution = TaskExecution {
                                        task_id,
                                        template_name: agent_type_for_evolution.clone(),
                                        template_version: template_version_for_evolution,
                                        outcome: TaskOutcome::Success,
                                        executed_at: chrono::Utc::now(),
                                        turns_used: 0, // convergent mode tracks iterations, not turns
                                        tokens_used: 0, // token tracking aggregated inside convergence loop
                                        downstream_tasks: vec![],
                                    };
                                    evolution_loop.record_execution(execution).await;
                                }

                                audit_log.log(
                                    crate::services::AuditEntry::new(
                                        AuditLevel::Info,
                                        AuditCategory::Task,
                                        AuditAction::TaskCompleted,
                                        AuditActor::System,
                                        format!("Convergent task {} completed successfully", task_id),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;
                            }

                            Ok(super::convergent_execution::ConvergentOutcome::IntentGapsFound(ivr)) => {
                                // Overseers confirmed the code compiles/passes tests,
                                // but intent verification found semantic gaps. Store
                                // gap context on the task and fail it so the workflow
                                // engine (or standalone retry) can re-enqueue with the
                                // gap information available to the next attempt.
                                let total_gaps = ivr.gaps.len() + ivr.implicit_gaps.len();
                                let gap_descriptions: Vec<String> = ivr.all_gaps()
                                    .map(|g| {
                                        let action = g.suggested_action.as_deref().unwrap_or("(no suggestion)");
                                        format!("- [{}] {}: {}", g.severity.as_str(), g.description, action)
                                    })
                                    .collect();
                                let gap_context = format!(
                                    "## Intent Verification Gaps (from previous attempt)\n\
                                     Satisfaction: {} (confidence: {:.2})\n\
                                     Accomplishment: {}\n\n\
                                     ### Gaps to address:\n{}",
                                    ivr.satisfaction.as_str(),
                                    ivr.confidence,
                                    ivr.accomplishment_summary,
                                    gap_descriptions.join("\n"),
                                );

                                audit_log.log(
                                    crate::services::AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        format!(
                                            "Task {} overseer-converged but intent unsatisfied ({}, {} gaps). Failing with gap context for retry.",
                                            task_id, ivr.satisfaction.as_str(), total_gaps,
                                        ),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Store structured gap context on the task so that
                                // when the workflow engine retries it (or a standalone
                                // retry task is created), the agent gets the gaps.
                                if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                    t.context.custom.insert(
                                        "intent_gaps".to_string(),
                                        serde_json::json!({
                                            "satisfaction": ivr.satisfaction.as_str(),
                                            "confidence": ivr.confidence,
                                            "accomplishment": ivr.accomplishment_summary,
                                            "gaps": ivr.gaps.iter().map(|g| serde_json::json!({
                                                "description": g.description,
                                                "severity": g.severity.as_str(),
                                                "category": g.category.as_str(),
                                                "suggested_action": g.suggested_action,
                                            })).collect::<Vec<_>>(),
                                            "implicit_gaps": ivr.implicit_gaps.iter().map(|g| serde_json::json!({
                                                "description": g.description,
                                                "severity": g.severity.as_str(),
                                                "category": g.category.as_str(),
                                                "suggested_action": g.suggested_action,
                                            })).collect::<Vec<_>>(),
                                        }),
                                    );
                                    t.context.custom.insert(
                                        "intent_gap_context".to_string(),
                                        serde_json::json!(gap_context),
                                    );

                                    let is_workflow_subtask = t.context.custom.contains_key("workflow_phase");

                                    if !t.status.is_terminal() {
                                        let _ = t.transition_to(TaskStatus::Failed);
                                    }
                                    let retry_count = t.retry_count;
                                    let _ = task_repo.update(&t).await;

                                    event_bus.publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Warning,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskFailed {
                                            task_id,
                                            error: format!(
                                                "Intent verification found {} gap(s); {}",
                                                total_gaps,
                                                if is_workflow_subtask {
                                                    "workflow engine will retry with gap context"
                                                } else {
                                                    "creating retry task with gap context"
                                                },
                                            ),
                                            retry_count,
                                        },
                                    )).await;

                                    // For standalone tasks (not workflow subtasks),
                                    // create an explicit retry task since there is no
                                    // workflow engine to manage the retry lifecycle.
                                    if !is_workflow_subtask {
                                        let new_description = format!(
                                            "{}\n\n{}", t.description, gap_context,
                                        );
                                        let mut retry_task = Task::with_title(
                                            format!("[retry] {}", t.title),
                                            &new_description,
                                        );
                                        retry_task.parent_id = t.parent_id;
                                        retry_task.task_type = t.task_type;
                                        retry_task.priority = t.priority;
                                        retry_task.source = t.source;
                                        retry_task.context.custom.insert(
                                            "intent_gaps".to_string(),
                                            t.context.custom.get("intent_gaps").cloned()
                                                .unwrap_or_default(),
                                        );
                                        retry_task.context.custom.insert(
                                            "retry_reason".to_string(),
                                            serde_json::json!("intent_gaps_found"),
                                        );
                                        retry_task.context.custom.insert(
                                            "previous_task_id".to_string(),
                                            serde_json::json!(task_id.to_string()),
                                        );
                                        let _ = retry_task.transition_to(TaskStatus::Ready);

                                        match task_repo.create(&retry_task).await {
                                            Ok(_) => {
                                                tracing::info!(
                                                    original_task_id = %task_id,
                                                    retry_task_id = %retry_task.id,
                                                    gaps = total_gaps,
                                                    "Created standalone retry task with intent gap context"
                                                );
                                                event_bus.publish(crate::services::event_factory::task_event(
                                                    crate::services::event_bus::EventSeverity::Info,
                                                    None,
                                                    retry_task.id,
                                                    crate::services::event_bus::EventPayload::TaskReady {
                                                        task_id: retry_task.id,
                                                        task_title: retry_task.title.clone(),
                                                    },
                                                )).await;
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    task_id = %task_id,
                                                    error = %e,
                                                    "Failed to create retry task for intent gaps"
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            Ok(super::convergent_execution::ConvergentOutcome::Decomposed(trajectory)) => {
                                // The convergence engine determined the task should be decomposed.
                                // Extract subtask descriptions from the trajectory specification's
                                // success criteria -- each criterion becomes a child task.
                                // The parent task stays Running; it completes when children finish.
                                let spec = &trajectory.specification.effective;
                                let criteria = &spec.success_criteria;

                                let child_count = if criteria.is_empty() { 1 } else { criteria.len() };

                                audit_log.log(
                                    crate::services::AuditEntry::new(
                                        AuditLevel::Info,
                                        AuditCategory::Task,
                                        AuditAction::TaskCompleted,
                                        AuditActor::System,
                                        format!(
                                            "Convergent task {} decomposed into {} subtask(s) (trajectory {})",
                                            task_id, child_count, trajectory.id,
                                        ),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                if criteria.is_empty() {
                                    // No granular criteria -- create a single child with the
                                    // full specification as a retry in Direct mode
                                    let mut child = Task::with_title(
                                        format!("Decomposed from {}", task_id),
                                        &spec.content,
                                    );
                                    child.parent_id = Some(task_id);
                                    child.execution_mode = crate::domain::models::ExecutionMode::Direct;
                                    let _ = child.transition_to(TaskStatus::Ready);
                                    if let Err(e) = task_repo.create(&child).await {
                                        tracing::warn!(
                                            "Failed to create decomposed subtask for {}: {}",
                                            task_id, e
                                        );
                                    }
                                } else {
                                    for (i, criterion) in criteria.iter().enumerate() {
                                        let title = format!(
                                            "Subtask {}/{} of {}",
                                            i + 1, criteria.len(), task_id,
                                        );
                                        let description = format!(
                                            "{}\n\nFocus: {}",
                                            spec.content, criterion,
                                        );
                                        let mut child = Task::with_title(&title, &description);
                                        child.parent_id = Some(task_id);
                                        child.execution_mode = crate::domain::models::ExecutionMode::Direct;
                                        let _ = child.transition_to(TaskStatus::Ready);
                                        if let Err(e) = task_repo.create(&child).await {
                                            tracing::warn!(
                                                "Failed to create decomposed subtask {} for {}: {}",
                                                i + 1, task_id, e
                                            );
                                        }
                                    }
                                }

                                // Worktree stays alive; children may use it or create their own
                            }

                            Ok(super::convergent_execution::ConvergentOutcome::Failed(msg)) => {
                                // Check if the agent already completed the task via MCP
                                // before attempting failure transition.
                                let current_task = task_repo.get(task_id).await.ok().flatten();
                                let already_terminal = current_task.as_ref()
                                    .is_some_and(|t| t.status.is_terminal());

                                if already_terminal {
                                    tracing::warn!(
                                        task_id = %task_id,
                                        status = ?current_task.as_ref().map(|t| t.status),
                                        error = %msg,
                                        "Skipping convergent task failure — already terminal (completed via MCP)"
                                    );
                                } else {
                                    if let Some(ref cb) = command_bus {
                                        let envelope = CommandEnvelope::new(
                                            CommandSource::System,
                                            DomainCommand::Task(TaskCommand::Fail {
                                                task_id,
                                                error: Some(msg.clone()),
                                            }),
                                        );
                                        if let Err(e) = cb.dispatch(envelope).await {
                                            tracing::warn!(
                                                "Failed to fail convergent task {} via CommandBus: {}",
                                                task_id, e
                                            );
                                            if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                                && !t.status.is_terminal() {
                                                    let _ = t.transition_to(TaskStatus::Failed);
                                                    let _ = task_repo.update(&t).await;
                                                }
                                        }
                                    } else if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                        && !t.status.is_terminal() {
                                            let _ = t.transition_to(TaskStatus::Failed);
                                            let _ = task_repo.update(&t).await;
                                        }

                                    let current_retry_count = task_repo.get(task_id).await
                                        .ok().flatten().map(|t| t.retry_count).unwrap_or(0);
                                    event_bus.publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Warning,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskFailed {
                                            task_id,
                                            error: msg.clone(),
                                            retry_count: current_retry_count,
                                        },
                                    )).await;
                                }

                                circuit_breaker.record_failure(circuit_scope.clone(), &msg).await;

                                // Mark worktree as failed
                                if worktree_path.is_some()
                                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }

                                // Record failure in evolution loop for template improvement
                                if track_evolution {
                                    let execution = TaskExecution {
                                        task_id,
                                        template_name: agent_type_for_evolution.clone(),
                                        template_version: template_version_for_evolution,
                                        outcome: TaskOutcome::Failure,
                                        executed_at: chrono::Utc::now(),
                                        turns_used: 0,
                                        tokens_used: 0,
                                        downstream_tasks: vec![],
                                    };
                                    evolution_loop.record_execution(execution).await;
                                }

                                audit_log.log(
                                    crate::services::AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        format!("Convergent task {} failed: {}", task_id, msg),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;
                            }

                            Ok(super::convergent_execution::ConvergentOutcome::Cancelled) => {
                                // Trajectory has been persisted by the convergence loop.
                                // Transition task to Canceled status.
                                if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                    let _ = t.transition_to(TaskStatus::Canceled);
                                    let _ = task_repo.update(&t).await;
                                }

                                if worktree_path.is_some()
                                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail("cancelled".to_string());
                                        let _ = worktree_repo.update(&wt).await;
                                    }

                                audit_log.log(
                                    crate::services::AuditEntry::new(
                                        AuditLevel::Info,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        format!("Convergent task {} cancelled", task_id),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;
                            }

                            Err(e) => {
                                let error_msg = format!("Convergent execution error: {}", e);

                                if let Some(ref cb) = command_bus {
                                    let envelope = CommandEnvelope::new(
                                        CommandSource::System,
                                        DomainCommand::Task(TaskCommand::Fail {
                                            task_id,
                                            error: Some(error_msg.clone()),
                                        }),
                                    );
                                    let _ = cb.dispatch(envelope).await;
                                } else if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                    let _ = t.transition_to(TaskStatus::Failed);
                                    let _ = task_repo.update(&t).await;
                                }

                                circuit_breaker.record_failure(circuit_scope.clone(), &error_msg).await;

                                if worktree_path.is_some()
                                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(error_msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }

                                audit_log.log(
                                    crate::services::AuditEntry::new(
                                        AuditLevel::Error,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        error_msg,
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;
                            }
                        }

                        // Convergent path complete -- skip direct execution below
                        guardrails.register_agent_end(&agent_unique_id_for_spawn).await;
                        return;
                    } else {
                        // Missing convergence infrastructure -- fall back to direct
                        tracing::warn!(
                            "Task {} has convergent execution mode but convergence infrastructure \
                             is not fully configured (overseer_cluster={}, trajectory_repo={}, \
                             memory_repo={}, intent_verifier={}). Falling back to direct execution.",
                            task_id,
                            overseer_cluster.is_some(),
                            trajectory_repo.is_some(),
                            memory_repo.is_some(),
                            convergent_intent_verifier.is_some(),
                        );
                        audit_log.log(
                            crate::services::AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Execution,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!(
                                    "Task {} requested convergent execution but infrastructure not configured; using direct mode",
                                    task_id
                                ),
                            )
                            .with_entity(task_id, "task"),
                        ).await;
                    }
                }

                // -----------------------------------------------------------------
                // Direct execution path (single-shot substrate invocation)
                // -----------------------------------------------------------------

                // Build substrate request with MCP servers for agent access to system services
                let mut config = SubstrateConfig::default().with_max_turns(max_turns);
                if let Some(ref wt_path) = worktree_path {
                    config = config.with_working_dir(wt_path);
                }

                // Model selection priority:
                //   1. Template's preferred_model (explicit override, e.g. "haiku" for aggregator)
                //   2. ModelRouter (cost-aware routing by complexity + tier + retry count)
                // Without this, config.model stays None and the substrate falls back to its
                // default model (Opus for Claude Code) for every agent — the swarm never
                // downgrades to cheaper models.
                if let Some(ref model) = template_preferred_model {
                    config.model = Some(model.clone());
                } else {
                    let tier_hint = match template_tier {
                        AgentTier::Architect => AgentTierHint::Architect,
                        AgentTier::Specialist => AgentTierHint::Specialist,
                        AgentTier::Worker => AgentTierHint::Worker,
                    };
                    let selection = ModelRouter::with_defaults().select_model(
                        task_clone.routing_hints.complexity,
                        Some(tier_hint),
                        task_clone.retry_count,
                    );
                    tracing::debug!(
                        task_id = %task_id,
                        %agent_type,
                        model = %selection.model,
                        reason = %selection.reason,
                        "ModelRouter selected model for direct execution"
                    );
                    config.model = Some(selection.model);
                }

                // Apply agent-specific tool restrictions from template
                // (empty vec falls back to DEFAULT_TOOLS in build_args)
                if !cli_tools.is_empty() {
                    config = config.with_allowed_tools(cli_tools);
                }

                // Construct MCP stdio server command for agent access to Abathur APIs.
                // Use absolute path so the MCP server finds the DB regardless of
                // the agent's working directory (worktrees have a different CWD).
                let abathur_exe = std::env::current_exe()
                    .unwrap_or_else(|_| std::path::PathBuf::from("abathur"));
                let db_path = std::env::current_dir()
                    .unwrap_or_else(|_| repo_path.clone())
                    .join(".abathur")
                    .join("abathur.db");

                // Build MCP args — overmind gets --workflow-session to block task_submit
                let mut mcp_args = vec![
                    "mcp".to_string(), "stdio".to_string(),
                    "--db-path".to_string(), db_path.to_string_lossy().to_string(),
                    "--task-id".to_string(), task_id.to_string(),
                ];
                if agent_type.to_lowercase() == "overmind" {
                    mcp_args.push("--workflow-session".to_string());
                }
                let mcp_config = serde_json::json!({
                    "mcpServers": {
                        "abathur": {
                            "command": abathur_exe.to_string_lossy(),
                            "args": mcp_args
                        }
                    }
                });
                config = config.with_mcp_server(mcp_config.to_string());

                let request = SubstrateRequest::new(
                    task_id,
                    &agent_type,
                    &system_prompt,
                    &task_description,
                ).with_config(config);

                let result = substrate.execute(request).await;

                // Auto-commit safety net: capture any uncommitted work
                if let Some(ref wt_path) = worktree_path {
                    let _ = auto_commit_worktree(wt_path, task_id).await;
                }

                // Update task based on result
                if let Ok(Some(mut completed_task)) = task_repo.get(task_id).await {
                    match result {
                        Ok(session) if session.status == SessionStatus::Completed => {
                            let tokens = session.total_tokens();
                            let turns = session.turns_completed;
                            total_tokens.fetch_add(tokens, Ordering::Relaxed);

                            // Check if the agent already completed the task via MCP
                            // (e.g. task_update_status tool during execution). If so,
                            // the TaskCompleted event was already published and
                            // downstream handlers have already reacted.
                            if !completed_task.status.is_terminal() {
                                // Transition task via CommandBus (journals event).
                                // Guard: if verify_on_completion would set Validating but
                                // the task has a non-terminal workflow state that isn't
                                // Verifying, skip Validating to avoid a deadlock. The
                                // centralized guard in transition_to_validating() catches
                                // the CommandBus path, but the fallback paths below bypass
                                // it, so we also guard here at the decision point.
                                let target_status = if verify_on_completion && can_safely_auto_complete(&completed_task) {
                                    TaskStatus::Validating
                                } else if verify_on_completion && !can_safely_auto_complete(&completed_task) {
                                    tracing::warn!(
                                        task_id = %task_id,
                                        "Skipping Validating transition for task with active workflow — completing directly"
                                    );
                                    TaskStatus::Complete
                                } else {
                                    TaskStatus::Complete
                                };
                                if let Some(ref cb) = command_bus {
                                    let envelope = CommandEnvelope::new(
                                        CommandSource::System,
                                        DomainCommand::Task(TaskCommand::Transition {
                                            task_id,
                                            new_status: target_status,
                                        }),
                                    );
                                    if let Err(e) = cb.dispatch(envelope).await {
                                        tracing::warn!("Failed to complete task {} via CommandBus, using non-atomic fallback: {}", task_id, e);
                                        if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                            && !t.status.is_terminal() {
                                                let _ = t.transition_to(target_status);
                                                let _ = task_repo.update(&t).await;
                                            }
                                        // Only emit TaskCompleted when actually completing.
                                        // When target is Validating, the verification workflow
                                        // will emit the event after Validating -> Complete.
                                        if target_status == TaskStatus::Complete {
                                            event_bus.publish(crate::services::event_factory::task_event(
                                                crate::services::event_bus::EventSeverity::Info,
                                                None,
                                                task_id,
                                                crate::services::event_bus::EventPayload::TaskCompleted {
                                                    task_id,
                                                    tokens_used: tokens,
                                                },
                                            )).await;
                                        }
                                    }
                                } else {
                                    tracing::warn!("CommandBus not available for task {} completion, using non-atomic fallback", task_id);
                                    let _ = completed_task.transition_to(target_status);
                                    let _ = task_repo.update(&completed_task).await;
                                    // Only emit TaskCompleted when actually completing.
                                    // When target is Validating, the verification workflow
                                    // will emit the event after Validating -> Complete.
                                    if target_status == TaskStatus::Complete {
                                        event_bus.publish(crate::services::event_factory::task_event(
                                            crate::services::event_bus::EventSeverity::Info,
                                            None,
                                            task_id,
                                            crate::services::event_bus::EventPayload::TaskCompleted {
                                                task_id,
                                                tokens_used: tokens,
                                            },
                                        )).await;
                                    }
                                }
                            } else {
                                tracing::debug!(
                                    task_id = %task_id,
                                    status = ?completed_task.status,
                                    "Skipping task transition — already terminal (completed via MCP)"
                                );
                                // Re-emit WorkflowGateRejected on the main event bus so
                                // AdapterLifecycleSyncHandler can close/comment on the
                                // external issue. The original event was published on the
                                // MCP session's local bus which has no handlers.
                                replay_gate_rejection_event(&completed_task, &event_bus, &all_workflows).await;
                            }

                            // Record success with circuit breaker
                            circuit_breaker.record_success(circuit_scope.clone()).await;

                            // Log task completion
                            audit_log.log(
                                AuditEntry::new(
                                    AuditLevel::Info,
                                    AuditCategory::Task,
                                    AuditAction::TaskCompleted,
                                    AuditActor::System,
                                    format!("Task completed: {} tokens used, {} turns", tokens, turns),
                                )
                                .with_entity(task_id, "task"),
                            ).await;

                            // Mark worktree as completed and create artifact reference
                            if worktree_path.is_some()
                                && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                    wt.complete();
                                    let _ = worktree_repo.update(&wt).await;

                                    if let Ok(Some(mut task)) = task_repo.get(task_id).await {
                                        let artifact = crate::domain::models::ArtifactRef {
                                            uri: format!("worktree://{}/{}", task_id, wt.branch),
                                            artifact_type: crate::domain::models::ArtifactType::Code,
                                            checksum: wt.merge_commit.clone(),
                                        };
                                        task.artifacts.push(artifact);
                                        task.worktree_path = Some(wt.path.clone());
                                        let _ = task_repo.update(&task).await;
                                    }
                                }

                            // (Bridge forwards EventBus→event_tx automatically)

                            // Run post-completion workflow: verify and merge
                            if verify_on_completion || use_merge_queue || prefer_pull_requests {
                                let workflow_result = run_post_completion_workflow(
                                    task_id,
                                    post_task_repo.clone(),
                                    post_goal_repo.clone(),
                                    post_worktree_repo.clone(),
                                    &event_tx,
                                    &event_bus,
                                    &audit_log,
                                    verify_on_completion,
                                    use_merge_queue,
                                    prefer_pull_requests,
                                    &repo_path,
                                    &default_base_ref,
                                    require_commits,
                                    false, // intent_satisfied: no convergence verification on this path
                                    output_delivery.clone(),
                                    merge_request_repo.clone(),
                                    fetch_on_sync,
                                    post_completion_chain.clone(),
                                ).await;

                                if let Err(e) = workflow_result {
                                    audit_log.log(
                                        AuditEntry::new(
                                            AuditLevel::Warning,
                                            AuditCategory::Task,
                                            AuditAction::TaskFailed,
                                            AuditActor::System,
                                            format!("Post-completion workflow error for task {}: {}", task_id, e),
                                        )
                                        .with_entity(task_id, "task"),
                                    ).await;
                                }
                            }

                            // Record execution in evolution loop AFTER verification
                            // so we capture the true outcome (success vs verification failure)
                            if track_evolution {
                                let outcome = if let Ok(Some(post_task)) = task_repo.get(task_id).await {
                                    if post_task.status == TaskStatus::Failed {
                                        TaskOutcome::Failure
                                    } else {
                                        TaskOutcome::Success
                                    }
                                } else {
                                    TaskOutcome::Success // fallback if we can't read task
                                };
                                let execution = TaskExecution {
                                    task_id,
                                    template_name: agent_type_for_evolution.clone(),
                                    template_version: template_version_for_evolution,
                                    outcome,
                                    executed_at: chrono::Utc::now(),
                                    turns_used: turns,
                                    tokens_used: tokens,
                                    downstream_tasks: vec![],
                                };
                                evolution_loop.record_execution(execution).await;
                            }

                            // Note: evolution loop evaluation is handled by process_evolution_refinements()
                            // which runs every reconciliation_interval_secs. Inline evaluation here
                            // was removed to prevent System B (EvolutionTriggeredTemplateUpdateHandler)
                            // from creating untracked evolve: tasks that bypass the RefinementRequest lifecycle.
                        }
                        Ok(session) => {
                            let tokens = session.total_tokens();
                            let turns = session.turns_completed;
                            total_tokens.fetch_add(tokens, Ordering::Relaxed);

                            let error_msg = session.error.clone().unwrap_or_else(|| "Unknown error".to_string());

                            // Check if the agent already completed the task via MCP
                            // before attempting failure transition.
                            let auto_completed = if completed_task.status.is_terminal() {
                                tracing::warn!(
                                    task_id = %task_id,
                                    status = ?completed_task.status,
                                    error = %error_msg,
                                    "Skipping task failure — already terminal (completed via MCP)"
                                );
                                replay_gate_rejection_event(&completed_task, &event_bus, &all_workflows).await;
                                false
                            } else if is_max_turns_auto_completable(&error_msg) {
                                // Agent exhausted max_turns but its last output says "completed".
                                // Auto-complete instead of failing — the agent finished its work
                                // but the session didn't end cleanly.
                                //
                                // However, if the task has a non-terminal workflow, don't auto-complete
                                // to Validating — that creates an illegal Validating+PhaseReady deadlock.
                                if !can_safely_auto_complete(&completed_task) {
                                    tracing::warn!(
                                        task_id = %task_id,
                                        error = %error_msg,
                                        "Overmind exhausted turns mid-workflow — failing instead of auto-completing"
                                    );
                                    false
                                } else {
                                tracing::warn!(
                                    task_id = %task_id,
                                    error = %error_msg,
                                    "Auto-completing task — agent exhausted turns but reported completion"
                                );
                                let target_status = if verify_on_completion {
                                    TaskStatus::Validating
                                } else {
                                    TaskStatus::Complete
                                };
                                if let Some(ref cb) = command_bus {
                                    let envelope = CommandEnvelope::new(
                                        CommandSource::System,
                                        DomainCommand::Task(TaskCommand::Transition {
                                            task_id,
                                            new_status: target_status,
                                        }),
                                    );
                                    if let Err(e) = cb.dispatch(envelope).await {
                                        tracing::warn!("Failed to auto-complete task {} via CommandBus, using non-atomic fallback: {}", task_id, e);
                                        if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                            && !t.status.is_terminal() {
                                                let _ = t.transition_to(target_status);
                                                let _ = task_repo.update(&t).await;
                                            }
                                        if target_status == TaskStatus::Complete {
                                            event_bus.publish(crate::services::event_factory::task_event(
                                                crate::services::event_bus::EventSeverity::Info,
                                                None,
                                                task_id,
                                                crate::services::event_bus::EventPayload::TaskCompleted {
                                                    task_id,
                                                    tokens_used: tokens,
                                                },
                                            )).await;
                                        }
                                    }
                                } else {
                                    tracing::warn!("CommandBus not available for task {} auto-completion, using non-atomic fallback", task_id);
                                    let _ = completed_task.transition_to(target_status);
                                    let _ = task_repo.update(&completed_task).await;
                                    if target_status == TaskStatus::Complete {
                                        event_bus.publish(crate::services::event_factory::task_event(
                                            crate::services::event_bus::EventSeverity::Info,
                                            None,
                                            task_id,
                                            crate::services::event_bus::EventPayload::TaskCompleted {
                                                task_id,
                                                tokens_used: tokens,
                                            },
                                        )).await;
                                    }
                                }
                                true
                                } // end else (can_safely_auto_complete)
                            } else {
                                // Fail task via CommandBus (transitions + journals event)
                                if let Some(ref cb) = command_bus {
                                    let envelope = CommandEnvelope::new(
                                        CommandSource::System,
                                        DomainCommand::Task(TaskCommand::Fail {
                                            task_id,
                                            error: Some(error_msg.clone()),
                                        }),
                                    );
                                    if let Err(e) = cb.dispatch(envelope).await {
                                        tracing::warn!("Failed to fail task {} via CommandBus, using non-atomic fallback: {}", task_id, e);
                                        if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                            && !t.status.is_terminal() {
                                                let _ = t.transition_to(TaskStatus::Failed);
                                                let _ = task_repo.update(&t).await;
                                            }
                                        event_bus.publish(crate::services::event_factory::task_event(
                                            crate::services::event_bus::EventSeverity::Warning,
                                            None,
                                            task_id,
                                            crate::services::event_bus::EventPayload::TaskFailed {
                                                task_id,
                                                error: error_msg.clone(),
                                                retry_count: completed_task.retry_count,
                                            },
                                        )).await;
                                    }
                                } else {
                                    tracing::warn!("CommandBus not available for task {} failure, using non-atomic fallback", task_id);
                                    let _ = completed_task.transition_to(TaskStatus::Failed);
                                    let _ = task_repo.update(&completed_task).await;
                                    event_bus.publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Warning,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskFailed {
                                            task_id,
                                            error: error_msg.clone(),
                                            retry_count: completed_task.retry_count,
                                        },
                                    )).await;
                                }
                                false
                            };

                            if !auto_completed {
                                // Record failure with circuit breaker
                                circuit_breaker.record_failure(
                                    circuit_scope.clone(),
                                    &error_msg,
                                ).await;
                            }

                            // Record execution in evolution loop for template improvement
                            if track_evolution {
                                let execution = TaskExecution {
                                    task_id,
                                    template_name: agent_type_for_evolution.clone(),
                                    template_version: template_version_for_evolution,
                                    outcome: if auto_completed { TaskOutcome::Success } else { TaskOutcome::Failure },
                                    executed_at: chrono::Utc::now(),
                                    turns_used: turns,
                                    tokens_used: tokens,
                                    downstream_tasks: vec![],
                                };
                                evolution_loop.record_execution(execution).await;
                            }

                            if auto_completed {
                                // Log auto-completion
                                audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Task,
                                        AuditAction::TaskCompleted,
                                        AuditActor::System,
                                        format!(
                                            "Task auto-completed (max_turns with completion signal): {}",
                                            error_msg,
                                        ),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Mark worktree as completed for auto-completed tasks
                                if worktree_path.is_some()
                                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.complete();
                                        let _ = worktree_repo.update(&wt).await;
                                    }
                            } else {
                                // Log task failure with retry state for debugging
                                let consecutive_budget = completed_task.context.custom
                                    .get("consecutive_budget_failures")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Task,
                                        AuditAction::TaskFailed,
                                        AuditActor::System,
                                        format!(
                                            "Task failed: {} (retry {}/{}, consecutive_budget_failures: {})",
                                            error_msg,
                                            completed_task.retry_count,
                                            completed_task.max_retries,
                                            consecutive_budget,
                                        ),
                                    )
                                    .with_entity(task_id, "task"),
                                ).await;

                                // Mark worktree as failed
                                if worktree_path.is_some()
                                    && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(error_msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }
                            }

                            // (Bridge forwards EventBus→event_tx automatically)
                        }
                        Err(e) => {
                            let error_msg = e.to_string();

                            // Check if the agent already completed the task via MCP
                            // before attempting failure transition.
                            if completed_task.status.is_terminal() {
                                tracing::warn!(
                                    task_id = %task_id,
                                    status = ?completed_task.status,
                                    error = %error_msg,
                                    "Skipping task failure — already terminal (completed via MCP)"
                                );
                                replay_gate_rejection_event(&completed_task, &event_bus, &all_workflows).await;
                            } else {
                                // Fail task via CommandBus (transitions + journals event)
                                if let Some(ref cb) = command_bus {
                                    let envelope = CommandEnvelope::new(
                                        CommandSource::System,
                                        DomainCommand::Task(TaskCommand::Fail {
                                            task_id,
                                            error: Some(error_msg.clone()),
                                        }),
                                    );
                                    if let Err(e) = cb.dispatch(envelope).await {
                                        tracing::warn!("Failed to fail task {} via CommandBus, using non-atomic fallback: {}", task_id, e);
                                        if let Ok(Some(mut t)) = task_repo.get(task_id).await
                                            && !t.status.is_terminal() {
                                                let _ = t.transition_to(TaskStatus::Failed);
                                                let _ = task_repo.update(&t).await;
                                            }
                                        event_bus.publish(crate::services::event_factory::task_event(
                                            crate::services::event_bus::EventSeverity::Warning,
                                            None,
                                            task_id,
                                            crate::services::event_bus::EventPayload::TaskFailed {
                                                task_id,
                                                error: error_msg.clone(),
                                                retry_count: completed_task.retry_count,
                                            },
                                        )).await;
                                    }
                                } else {
                                    tracing::warn!("CommandBus not available for task {} failure, using non-atomic fallback", task_id);
                                    let _ = completed_task.transition_to(TaskStatus::Failed);
                                    let _ = task_repo.update(&completed_task).await;
                                    event_bus.publish(crate::services::event_factory::task_event(
                                        crate::services::event_bus::EventSeverity::Warning,
                                        None,
                                        task_id,
                                        crate::services::event_bus::EventPayload::TaskFailed {
                                            task_id,
                                            error: error_msg.clone(),
                                            retry_count: completed_task.retry_count,
                                        },
                                    )).await;
                                }
                            }

                            // Record failure with circuit breaker
                            circuit_breaker.record_failure(
                                circuit_scope.clone(),
                                &error_msg,
                            ).await;

                            // Record failure in evolution loop for template improvement
                            if track_evolution {
                                let execution = TaskExecution {
                                    task_id,
                                    template_name: agent_type_for_evolution.clone(),
                                    template_version: template_version_for_evolution,
                                    outcome: TaskOutcome::Failure,
                                    executed_at: chrono::Utc::now(),
                                    turns_used: 0,
                                    tokens_used: 0,
                                    downstream_tasks: vec![],
                                };
                                evolution_loop.record_execution(execution).await;
                            }

                            // Log task failure
                            audit_log.log(
                                AuditEntry::new(
                                    AuditLevel::Error,
                                    AuditCategory::Task,
                                    AuditAction::TaskFailed,
                                    AuditActor::System,
                                    format!("Task execution error: {}", error_msg),
                                )
                                .with_entity(task_id, "task"),
                            ).await;

                            // Mark worktree as failed
                            if worktree_path.is_some()
                                && let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                    wt.fail(error_msg.clone());
                                    let _ = worktree_repo.update(&wt).await;
                                }

                            // (Bridge forwards EventBus→event_tx automatically)
                        }
                    }
                }

                // Unregister agent from guardrails on ALL direct execution exit paths
                guardrails.register_agent_end(&agent_unique_id_for_spawn).await;
            });
        }

        Ok(())
    }

}

/// Checks whether a task can safely be auto-completed (to Validating or Complete).
///
/// Returns `false` if the task has a non-terminal `WorkflowState` in `context.custom`,
/// because auto-completing a workflow parent mid-workflow (e.g. while PhaseReady)
/// can create an illegal Validating+PhaseReady combination that causes a deadlock.
pub(crate) fn can_safely_auto_complete(task: &Task) -> bool {
    if let Some(ws_val) = task.context.custom.get("workflow_state")
        && let Ok(ws) = serde_json::from_value::<WorkflowState>(ws_val.clone())
            && !ws.is_terminal() {
                return false;
            }
    true
}

/// Checks if an error message represents a max-turns exhaustion where the agent's
/// last output indicates it believed it had completed successfully. In this case,
/// the task can be auto-completed instead of failed, since the agent did finish
/// its work but ran out of turns before the session could end cleanly.
///
/// Returns `true` when the error starts with `error_max_turns` AND the text after
/// "Last output:" (case-insensitive) contains language indicating the agent
/// believed it was done: "completed", "complete", "done", "finished",
/// "stored", "marking complete", or "task_update_status".
pub(crate) fn is_max_turns_auto_completable(error_msg: &str) -> bool {
    if !error_msg.starts_with("error_max_turns") {
        return false;
    }

    // Find "last output:" (case-insensitive) and check what follows
    let lower = error_msg.to_lowercase();
    if let Some(idx) = lower.find("last output:") {
        let after = &lower[idx + "last output:".len()..];
        let trimmed = after.trim();
        const COMPLETION_SIGNALS: &[&str] = &[
            "completed",
            "complete",
            "done",
            "finished",
            "stored",
            "marking complete",
            "task_update_status",
        ];
        COMPLETION_SIGNALS.iter().any(|signal| trimmed.contains(signal))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{Memory, ScoreBreakdown, ScoredMemory};

    fn scored(memory: Memory, score: f32) -> ScoredMemory {
        ScoredMemory {
            memory,
            score,
            score_breakdown: ScoreBreakdown::default(),
        }
    }

    #[test]
    fn test_format_memory_context_empty() {
        let output = format_memory_context(&[]);
        assert!(
            output.contains("## Relevant Context from Memory"),
            "Header should be present even for empty input"
        );
        assert!(
            output.contains("The following memories from previous work"),
            "Intro text should be present"
        );
    }

    #[test]
    fn test_format_memory_context_single_entry() {
        let entry = scored(
            Memory::semantic("rust-patterns", "Use iterators and closures for idiomatic Rust."),
            0.85,
        );
        let output = format_memory_context(&[entry]);

        assert!(output.contains("rust-patterns"), "Key should appear in output");
        assert!(output.contains("0.85"), "Score should appear in output");
        assert!(output.contains("Use iterators and closures"), "Content should appear in output");
        assert!(output.contains("semantic"), "Tier should appear in output");
    }

    #[test]
    fn test_format_memory_context_two_entries() {
        let first = scored(Memory::working("key-alpha", "First memory content."), 0.90);
        let second = scored(Memory::episodic("key-beta", "Second memory content."), 0.70);
        let output = format_memory_context(&[first, second]);

        assert!(output.contains("key-alpha"), "First key should appear");
        assert!(output.contains("First memory content."), "First content should appear");
        assert!(output.contains("key-beta"), "Second key should appear");
        assert!(output.contains("Second memory content."), "Second content should appear");
        assert!(output.contains("0.90"), "First score should appear");
        assert!(output.contains("0.70"), "Second score should appear");
    }

    #[test]
    fn test_max_turns_floor_enforcement() {
        // When template sets max_turns lower than role default, role default should win
        let template_max_turns: u32 = 25;
        let role_max_turns: u32 = 50;

        let max_turns = if template_max_turns > 0 {
            template_max_turns.max(role_max_turns)
        } else {
            role_max_turns
        };
        assert_eq!(max_turns, 50, "role floor should override lower template value");

        // When template is higher than role default, template should win
        let template_high: u32 = 75;
        let max_turns_high = if template_high > 0 {
            template_high.max(role_max_turns)
        } else {
            role_max_turns
        };
        assert_eq!(max_turns_high, 75, "template should win when higher than role floor");

        // When template is zero (unset), role default should be used
        let template_zero: u32 = 0;
        let max_turns_zero = if template_zero > 0 {
            template_zero.max(role_max_turns)
        } else {
            role_max_turns
        };
        assert_eq!(max_turns_zero, 50, "role default should be used when template is zero");
    }

    // -- is_max_turns_auto_completable tests ---------------------------------

    #[test]
    fn test_auto_completable_typical_message() {
        let msg = "error_max_turns: agent exhausted 31 turns without completing. Last output: completed";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_with_complete_variant() {
        let msg = "error_max_turns: Agent exhausted turns without completing. Last output: complete";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_not_auto_completable_different_last_output() {
        let msg = "error_max_turns: agent exhausted 25 turns. Last output: working on tests";
        assert!(!is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_not_auto_completable_non_max_turns_error() {
        let msg = "error_timeout: agent timed out. Last output: completed";
        assert!(!is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_not_auto_completable_no_last_output() {
        let msg = "error_max_turns: agent exceeded 40 turns";
        assert!(!is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_done() {
        let msg = "error_max_turns: agent exhausted 31 turns without completing. Last output: done";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_finished() {
        let msg = "error_max_turns: agent exhausted 31 turns without completing. Last output: finished";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_stored() {
        let msg = "error_max_turns: agent exhausted 31 turns without completing. Last output: stored results in memory";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_marking_complete() {
        let msg = "error_max_turns: agent exhausted 31 turns without completing. Last output: marking complete";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_task_update_status() {
        let msg = "error_max_turns: agent exhausted 31 turns without completing. Last output: task_update_status";
        assert!(is_max_turns_auto_completable(msg));
    }

    // --- can_safely_auto_complete tests (Fix 2 / Fix 8) ---

    #[test]
    fn test_can_safely_auto_complete_no_workflow_state() {
        let task = Task::new("Simple task without workflow");
        assert!(
            can_safely_auto_complete(&task),
            "Task with no workflow state should be safely auto-completable"
        );
    }

    #[test]
    fn test_can_safely_auto_complete_terminal_workflow_state() {
        let mut task = Task::new("Task with completed workflow");
        let ws = WorkflowState::Completed {
            workflow_name: "code".to_string(),
        };
        task.context.custom.insert(
            "workflow_state".to_string(),
            serde_json::to_value(&ws).unwrap(),
        );
        assert!(
            can_safely_auto_complete(&task),
            "Task with terminal workflow state should be safely auto-completable"
        );
    }

    #[test]
    fn test_can_safely_auto_complete_non_terminal_workflow_state() {
        let mut task = Task::new("Task with active workflow");
        let ws = WorkflowState::PhaseReady {
            workflow_name: "code".to_string(),
            phase_index: 1,
            phase_name: "implement".to_string(),
        };
        task.context.custom.insert(
            "workflow_state".to_string(),
            serde_json::to_value(&ws).unwrap(),
        );
        assert!(
            !can_safely_auto_complete(&task),
            "Task with non-terminal workflow state should NOT be safely auto-completable"
        );
    }
}

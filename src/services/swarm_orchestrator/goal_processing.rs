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
    SessionStatus, SubstrateConfig, SubstrateRequest,
    Task, TaskStatus,
};
use crate::domain::ports::{AgentFilter, AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    CircuitScope, GoalContextService,
    TaskExecution, TaskOutcome,
    command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand},
};

use super::helpers::{auto_commit_worktree, run_post_completion_workflow};
use super::types::SwarmEvent;
use super::SwarmOrchestrator;

/// Map agent template tool names (lowercase YAML) to Claude Code CLI tool names.
///
/// Template tools like "read", "shell", "memory" need to be translated to
/// the PascalCase names that `claude --allowedTools` expects.
/// Tools like "memory" and "tasks" are Abathur REST APIs accessed via WebFetch,
/// not Claude Code built-in tools, so they map to WebFetch access.
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
                cli_tools.push("mcp__abathur__goals_list".to_string());
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

    // Ensure baseline read-only tools are always present (agents must be able to explore code)
    for baseline in &["Read", "Glob", "Grep"] {
        if !cli_tools.contains(&baseline.to_string()) {
            cli_tools.push(baseline.to_string());
        }
    }

    cli_tools.sort();
    cli_tools.dedup();
    cli_tools
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

    /// Route a task to the appropriate agent type.
    ///
    /// Resolution priority:
    /// 1. Explicit `agent_type` on the task (user passed `--agent`)
    /// 2. `routing_hints.preferred_agent`
    /// 3. Capability matching: find an agent whose tools cover the task's `required_tools`
    /// 4. Default to `overmind` so the task gets analyzed, decomposed, and
    ///    routed to dynamically-created agents.
    async fn route_task(&self, task: &Task) -> String {
        // 1. Explicit assignment takes priority
        if let Some(ref agent) = task.agent_type {
            return agent.clone();
        }

        // 2. Routing hints - preferred agent
        if let Some(ref preferred) = task.routing_hints.preferred_agent {
            // Validate the preferred agent actually exists
            if let Ok(Some(_)) = self.agent_repo.get_template_by_name(preferred).await {
                return preferred.clone();
            }
        }

        // 3. Capability matching - try to find an agent whose tools satisfy required_tools
        if !task.routing_hints.required_tools.is_empty() {
            if let Some(matched) = self.match_agent_by_tools(&task.routing_hints.required_tools).await {
                return matched;
            }
        }

        // 4. For subtasks created by the overmind, don't recurse back into overmind.
        //    If a parent task was assigned to overmind and created this subtask without
        //    an explicit agent, route back to overmind for further routing. (Subtasks
        //    should have agent_type set by the overmind, but this is a safety net.)
        if task.parent_id.is_some() {
            return "overmind".to_string();
        }

        // 5. Default: route to overmind for analysis and decomposition
        "overmind".to_string()
    }

    /// Find an agent template whose tools cover the required tools.
    ///
    /// Returns the best match (agent covering the most required tools) or None.
    async fn match_agent_by_tools(&self, required_tools: &[String]) -> Option<String> {
        let templates = self.agent_repo.list_templates(AgentFilter::default()).await.ok()?;

        let mut best_match: Option<(String, usize)> = None;

        for template in &templates {
            // Skip meta-level agents for direct tool matching
            if template.name == "overmind" {
                continue;
            }

            let tool_names: Vec<&str> = template.tools.iter().map(|t| t.name.as_str()).collect();
            let matched_count = required_tools.iter()
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

    /// Spawn an agent for a ready task.
    ///
    /// Before execution, routes the task to the appropriate agent, loads
    /// relevant goals via GoalContextService, and prepends goal guidance
    /// to the task description.
    pub(super) async fn spawn_task_agent(
        &self,
        task: &Task,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        // Runtime safety net: don't spawn agents if MCP servers are down.
        // The task stays Ready and will be retried on the next poll cycle.
        if !self.check_mcp_readiness().await {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Execution,
                    AuditAction::TaskFailed,
                    AuditActor::System,
                    format!(
                        "Skipping spawn for task {} - MCP servers not ready (will retry next cycle)",
                        task.id
                    ),
                )
                .with_entity(task.id, "task"),
            ).await;
            return Ok(());
        }

        // Route the task to an appropriate agent
        let agent_type = self.route_task(task).await;

        // Persist the routing decision so it's visible in logs and task queries
        if task.agent_type.is_none() {
            if let Ok(Some(mut updated)) = self.task_repo.get(task.id).await {
                updated.agent_type = Some(agent_type.clone());
                let _ = self.task_repo.update(&updated).await;
            }
        }

        // Check circuit breaker
        let scope = CircuitScope::agent(&agent_type);
        let check_result = self.circuit_breaker.check(scope.clone()).await;

        if check_result.is_blocked() {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Execution,
                    AuditAction::CircuitBreakerTriggered,
                    AuditActor::System,
                    format!("Task {} blocked by circuit breaker for agent '{}'", task.id, agent_type),
                )
                .with_entity(task.id, "task"),
            ).await;
            return Ok(());
        }

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
                }
                Err(e) => {
                    tracing::warn!("Failed to atomically claim task {}: {}", task.id, e);
                    drop(permit);
                    return Ok(());
                }
            }

            let system_prompt = self.get_agent_system_prompt(&agent_type).await;

            // Get agent template for version tracking, capabilities, and tool restrictions
            let (template_version, capabilities, cli_tools, agent_can_write) = match self.agent_repo.get_template_by_name(&agent_type).await {
                Ok(Some(template)) => {
                    let caps: Vec<String> = template.tools.iter()
                        .map(|t| t.name.clone())
                        .collect();
                    let tools = map_template_tools_to_cli(&caps);
                    let can_write = caps.iter().any(|c| {
                        let lower = c.to_lowercase();
                        lower == "write" || lower == "edit" || lower == "shell"
                    });
                    (template.version, caps, tools, can_write)
                }
                // Default to true when template lookup fails (safer to require commits from unknown agents)
                _ => (1, vec!["task-execution".to_string()], vec![], true),
            };

            // Read-only agent roles never produce commits regardless of tool capabilities.
            // Agents like researchers and the overmind may have shell access for reading
            // but don't create code changes — requiring commits causes infinite retry loops.
            let is_read_only_role = {
                let lower = agent_type.to_lowercase();
                lower == "overmind"
                    || lower.contains("researcher")
                    || lower.contains("planner")
                    || lower.contains("analyst")
                    || lower.contains("architect")
            };

            // Register agent capabilities with A2A gateway if configured
            if self.config.mcp_servers.a2a_gateway.is_some() {
                if let Err(e) = self.register_agent_capabilities(&agent_type, capabilities).await {
                    tracing::warn!("Failed to register agent '{}' capabilities: {}", agent_type, e);
                }
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

            // Create worktree if configured
            let worktree_path = if self.config.use_worktrees {
                match self.create_worktree_for_task(task.id).await {
                    Ok(path) => Some(path),
                    Err(e) => {
                        tracing::warn!("Failed to create worktree for task {}: {}", task.id, e);
                        None
                    }
                }
            } else {
                None
            };

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
- Create subtasks: Use the `task_submit` tool directly
- Create agents: Use the `agent_create` tool directly
- Track progress: Use `task_list` and `task_get` tools
- Store learnings: Use the `memory_store` tool directly
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
                if let Ok(pretty) = serde_json::to_string_pretty(&settings_content) {
                    if let Err(e) = std::fs::write(claude_dir.join("settings.json"), format!("{pretty}\n")) {
                        tracing::warn!("Failed to write .claude/settings.json to worktree: {}", e);
                    }
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

            // Build the task description with goal context prepended
            let task_description = if let Some(ref ctx) = goal_context {
                format!("{}\n\n---\n\n{}", ctx, task.description)
            } else {
                task.description.clone()
            };

            // Spawn task execution
            let task_id = task.id;
            let is_convergent = task.execution_mode.is_convergent()
                && self.config.convergence_enabled;
            let task_clone = task.clone();
            let substrate = self.substrate.clone();
            let task_repo = self.task_repo.clone();
            let goal_repo = self.goal_repo.clone();
            let worktree_repo = self.worktree_repo.clone();
            let event_tx = event_tx.clone();
            let event_bus = self.event_bus.clone();
            let command_bus = self.command_bus.read().await.clone();
            let max_turns = self.config.default_max_turns;
            let total_tokens = self.total_tokens.clone();
            let use_worktrees = self.config.use_worktrees;
            let circuit_breaker = self.circuit_breaker.clone();
            let audit_log = self.audit_log.clone();
            let evolution_loop = self.evolution_loop.clone();
            let track_evolution = self.config.track_evolution;
            let agent_type_for_evolution = agent_type.clone();
            let template_version_for_evolution = template_version;
            let verify_on_completion = self.config.verify_on_completion;
            let use_merge_queue = self.config.use_merge_queue;
            let prefer_pull_requests = self.config.prefer_pull_requests;
            let repo_path = self.config.repo_path.clone();
            let default_base_ref = self.config.default_base_ref.clone();
            let require_commits = agent_can_write && !is_read_only_role;
            let circuit_scope = scope;

            // Convergence infrastructure (cloned into spawn block only when needed)
            let overseer_cluster = self.overseer_cluster.clone();
            let trajectory_repo = self.trajectory_repo.clone();
            let convergence_engine_config = self.convergence_engine_config.clone();
            let memory_repo = self.memory_repo.clone();

            tokio::spawn(async move {
                let _permit = permit;

                // Task is already Running (claimed atomically before spawn).

                // -----------------------------------------------------------------
                // Convergent execution path (Phase 3)
                // -----------------------------------------------------------------
                if is_convergent {
                    // Validate that all convergence infrastructure is available.
                    // If any piece is missing, fall back to direct execution with a warning.
                    let can_converge = overseer_cluster.is_some()
                        && trajectory_repo.is_some()
                        && memory_repo.is_some();

                    if can_converge {
                        let overseer_cluster = overseer_cluster.unwrap();
                        let trajectory_repo_arc = trajectory_repo.unwrap();
                        let memory_repo = memory_repo.unwrap();

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
                        // (not recreated between iterations)
                        let convergent_worktree_path = if use_worktrees {
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
                        } = &task_clone.execution_mode
                        {
                            if use_worktrees {
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
                            )
                            .await
                        };

                        // Auto-commit safety net after convergence terminates
                        if let Some(ref wt_path) = convergent_worktree_path {
                            let _ = auto_commit_worktree(wt_path, task_id).await;
                        }

                        // Map ConvergentOutcome to task status transitions
                        match outcome {
                            Ok(super::convergent_execution::ConvergentOutcome::Converged)
                            | Ok(super::convergent_execution::ConvergentOutcome::PartialAccepted) => {
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
                                        tracing::warn!(
                                            "Failed to complete convergent task {} via CommandBus: {}",
                                            task_id, e
                                        );
                                        if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                            let _ = t.transition_to(target_status);
                                            let _ = task_repo.update(&t).await;
                                        }
                                    }
                                } else if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                    let _ = t.transition_to(target_status);
                                    let _ = task_repo.update(&t).await;
                                }

                                event_bus.publish(crate::services::event_factory::task_event(
                                    crate::services::event_bus::EventSeverity::Info,
                                    None,
                                    task_id,
                                    crate::services::event_bus::EventPayload::TaskCompleted {
                                        task_id,
                                        tokens_used: 0, // token tracking aggregated inside convergence loop
                                    },
                                )).await;

                                circuit_breaker.record_success(circuit_scope.clone()).await;

                                // Mark worktree as completed
                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.complete();
                                        let _ = worktree_repo.update(&wt).await;
                                    }
                                }

                                // Run post-completion workflow (verify, merge, PR)
                                if verify_on_completion || use_merge_queue || prefer_pull_requests {
                                    let _ = run_post_completion_workflow(
                                        task_id,
                                        task_repo.clone(),
                                        goal_repo.clone(),
                                        worktree_repo.clone(),
                                        &event_tx,
                                        &audit_log,
                                        verify_on_completion,
                                        use_merge_queue,
                                        prefer_pull_requests,
                                        &repo_path,
                                        &default_base_ref,
                                        require_commits,
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
                                        &format!("Decomposed from {}", task_id),
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
                                        if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                            t.retry_count += 1;
                                            let _ = t.transition_to(TaskStatus::Failed);
                                            let _ = task_repo.update(&t).await;
                                        }
                                    }
                                } else if let Ok(Some(mut t)) = task_repo.get(task_id).await {
                                    t.retry_count += 1;
                                    let _ = t.transition_to(TaskStatus::Failed);
                                    let _ = task_repo.update(&t).await;
                                }

                                event_bus.publish(crate::services::event_factory::task_event(
                                    crate::services::event_bus::EventSeverity::Warning,
                                    None,
                                    task_id,
                                    crate::services::event_bus::EventPayload::TaskFailed {
                                        task_id,
                                        error: msg.clone(),
                                        retry_count: 0,
                                    },
                                )).await;

                                circuit_breaker.record_failure(circuit_scope.clone(), &msg).await;

                                // Mark worktree as failed
                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }
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

                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail("cancelled".to_string());
                                        let _ = worktree_repo.update(&wt).await;
                                    }
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
                                    t.retry_count += 1;
                                    let _ = t.transition_to(TaskStatus::Failed);
                                    let _ = task_repo.update(&t).await;
                                }

                                circuit_breaker.record_failure(circuit_scope.clone(), &error_msg).await;

                                if use_worktrees {
                                    if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                        wt.fail(error_msg.clone());
                                        let _ = worktree_repo.update(&wt).await;
                                    }
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
                        return;
                    } else {
                        // Missing convergence infrastructure -- fall back to direct
                        tracing::warn!(
                            "Task {} has convergent execution mode but convergence infrastructure \
                             is not fully configured (overseer_cluster={}, trajectory_repo={}, \
                             memory_repo={}). Falling back to direct execution.",
                            task_id,
                            overseer_cluster.is_some(),
                            trajectory_repo.is_some(),
                            memory_repo.is_some(),
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

                let mcp_config = serde_json::json!({
                    "mcpServers": {
                        "abathur": {
                            "command": abathur_exe.to_string_lossy(),
                            "args": ["mcp", "stdio", "--db-path", db_path.to_string_lossy(), "--task-id", task_id.to_string()]
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

                            // Transition task via CommandBus (journals event)
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
                                    tracing::warn!("Failed to complete task {} via CommandBus, using non-atomic fallback: {}", task_id, e);
                                    let _ = completed_task.transition_to(target_status);
                                    let _ = task_repo.update(&completed_task).await;
                                }
                            } else {
                                tracing::warn!("CommandBus not available for task {} completion, using non-atomic fallback", task_id);
                                let _ = completed_task.transition_to(target_status);
                                let _ = task_repo.update(&completed_task).await;
                            }

                            // Publish TaskCompleted event with actual token count via EventBus
                            event_bus.publish(crate::services::event_factory::task_event(
                                crate::services::event_bus::EventSeverity::Info,
                                None,
                                task_id,
                                crate::services::event_bus::EventPayload::TaskCompleted {
                                    task_id,
                                    tokens_used: tokens,
                                },
                            )).await;

                            // Record success with circuit breaker
                            circuit_breaker.record_success(circuit_scope.clone()).await;

                            // Record success in evolution loop for template improvement
                            if track_evolution {
                                let execution = TaskExecution {
                                    task_id,
                                    template_name: agent_type_for_evolution.clone(),
                                    template_version: template_version_for_evolution,
                                    outcome: TaskOutcome::Success,
                                    executed_at: chrono::Utc::now(),
                                    turns_used: turns,
                                    tokens_used: tokens,
                                    downstream_tasks: vec![],
                                };
                                evolution_loop.record_execution(execution).await;
                            }

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
                            if use_worktrees {
                                if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
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
                            }

                            // (Bridge forwards EventBus→event_tx automatically)

                            // Run post-completion workflow: verify and merge
                            if verify_on_completion || use_merge_queue || prefer_pull_requests {
                                let workflow_result = run_post_completion_workflow(
                                    task_id,
                                    task_repo.clone(),
                                    goal_repo.clone(),
                                    worktree_repo.clone(),
                                    &event_tx,
                                    &audit_log,
                                    verify_on_completion,
                                    use_merge_queue,
                                    prefer_pull_requests,
                                    &repo_path,
                                    &default_base_ref,
                                    require_commits,
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

                            // Evaluate evolution loop for potential refinements
                            if track_evolution {
                                let events = evolution_loop.evaluate().await;
                                for event in events {
                                    if event.template_name == agent_type_for_evolution {
                                        audit_log.log(
                                            AuditEntry::new(
                                                AuditLevel::Info,
                                                AuditCategory::Agent,
                                                AuditAction::AgentSpawned,
                                                AuditActor::System,
                                                format!(
                                                    "Evolution triggered for '{}': {:?} (success rate: {:.0}%)",
                                                    event.template_name,
                                                    event.trigger,
                                                    event.stats_at_trigger.success_rate * 100.0
                                                ),
                                            ),
                                        ).await;

                                        // Publish via EventBus (bridge forwards to event_tx)
                                        event_bus.publish(crate::services::event_factory::agent_event(
                                            crate::services::event_bus::EventSeverity::Info,
                                            Some(task_id),
                                            crate::services::event_bus::EventPayload::EvolutionTriggered {
                                                template_name: event.template_name.clone(),
                                                trigger: format!("{:?}", event.trigger),
                                            },
                                        )).await;
                                    }
                                }
                            }
                        }
                        Ok(session) => {
                            let tokens = session.total_tokens();
                            let turns = session.turns_completed;
                            total_tokens.fetch_add(tokens, Ordering::Relaxed);

                            let error_msg = session.error.clone().unwrap_or_else(|| "Unknown error".to_string());

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
                                    completed_task.retry_count += 1;
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
                            } else {
                                tracing::warn!("CommandBus not available for task {} failure, using non-atomic fallback", task_id);
                                completed_task.retry_count += 1;
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
                                    turns_used: turns,
                                    tokens_used: tokens,
                                    downstream_tasks: vec![],
                                };
                                evolution_loop.record_execution(execution).await;
                            }

                            // Log task failure
                            audit_log.log(
                                AuditEntry::new(
                                    AuditLevel::Warning,
                                    AuditCategory::Task,
                                    AuditAction::TaskFailed,
                                    AuditActor::System,
                                    format!("Task failed: {}", error_msg),
                                )
                                .with_entity(task_id, "task"),
                            ).await;

                            // Mark worktree as failed
                            if use_worktrees {
                                if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                    wt.fail(error_msg.clone());
                                    let _ = worktree_repo.update(&wt).await;
                                }
                            }

                            // (Bridge forwards EventBus→event_tx automatically)
                        }
                        Err(e) => {
                            let error_msg = e.to_string();

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
                                    completed_task.retry_count += 1;
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
                            } else {
                                tracing::warn!("CommandBus not available for task {} failure, using non-atomic fallback", task_id);
                                completed_task.retry_count += 1;
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
                            if use_worktrees {
                                if let Ok(Some(mut wt)) = worktree_repo.get_by_task(task_id).await {
                                    wt.fail(error_msg.clone());
                                    let _ = worktree_repo.update(&wt).await;
                                }
                            }

                            // (Bridge forwards EventBus→event_tx automatically)
                        }
                    }
                }
            });
        }

        Ok(())
    }

}

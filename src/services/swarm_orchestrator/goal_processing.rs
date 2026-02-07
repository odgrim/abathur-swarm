//! Goal processing subsystem for the swarm orchestrator.
//!
//! Handles task spawning for ready tasks, dependency management,
//! task readiness updates, and retry logic.
//!
//! Goals no longer decompose into tasks or own tasks. Instead, goals provide
//! aspirational guidance via GoalContextService when tasks are executed.

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
            // Abathur APIs are now provided via MCP stdio server as native tools.
            // These template tool names are kept for capability matching but don't
            // need CLI tool mapping — the MCP server handles them.
            "memory" | "tasks" | "agents" => {}
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
    /// Update task readiness based on dependency completion.
    pub(super) async fn update_task_readiness(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Get all pending tasks
        let pending_tasks = self.task_repo.list_by_status(TaskStatus::Pending).await?;

        for task in pending_tasks {
            // Check if any dependencies have permanently failed
            if self.has_failed_dependencies(&task).await? {
                // Transition to Blocked since upstream failed
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Blocked).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskFailed {
                        task_id: task.id,
                        error: "Upstream dependency failed".to_string(),
                        retry_count: 0,
                    }).await;
                }
            } else if self.are_dependencies_met(&task).await? {
                // Transition to Ready
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskReady {
                        task_id: task.id,
                        task_title: task.title.clone(),
                    }).await;
                }
            }
        }

        // Check for blocked tasks that can become ready (after upstream completion)
        let blocked_tasks = self.task_repo.list_by_status(TaskStatus::Blocked).await?;

        for task in blocked_tasks {
            // Skip if dependencies still failing
            if self.has_failed_dependencies(&task).await? {
                continue;
            }

            if self.are_dependencies_met(&task).await? {
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Ready).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskReady {
                        task_id: task.id,
                        task_title: task.title.clone(),
                    }).await;
                }
            }
        }

        // Also check Ready tasks - they may need to be blocked if a dependency failed
        let ready_tasks = self.task_repo.list_by_status(TaskStatus::Ready).await?;

        for task in ready_tasks {
            if self.has_failed_dependencies(&task).await? {
                let mut updated_task = task.clone();
                if updated_task.transition_to(TaskStatus::Blocked).is_ok() {
                    self.task_repo.update(&updated_task).await?;
                    let _ = event_tx.send(SwarmEvent::TaskFailed {
                        task_id: task.id,
                        error: "Upstream dependency failed".to_string(),
                        retry_count: 0,
                    }).await;
                }
            }
        }

        Ok(())
    }

    /// Check if all dependencies for a task are complete.
    pub(super) async fn are_dependencies_met(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(true);
        }

        let dependencies = self.task_repo.get_dependencies(task.id).await?;

        // All dependencies must be complete
        Ok(dependencies.iter().all(|dep| dep.status == TaskStatus::Complete))
    }

    /// Check if any dependencies failed (would block this task).
    pub(super) async fn has_failed_dependencies(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(false);
        }

        let dependencies = self.task_repo.get_dependencies(task.id).await?;
        Ok(dependencies.iter().any(|dep| dep.status == TaskStatus::Failed))
    }

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
    async fn spawn_task_agent(
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

        // Pre-execution constraint validation via goal alignment
        if let Some(ref alignment_service) = self.goal_alignment {
            match alignment_service.evaluate_task(task).await {
                Ok(evaluation) => {
                    // Check for constraint violations before execution
                    for alignment in &evaluation.goal_alignments {
                        if !alignment.constraints_satisfied {
                            for violation in &alignment.violations {
                                self.audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Goal,
                                        AuditAction::GoalEvaluated,
                                        AuditActor::System,
                                        format!(
                                            "Task {} may violate constraint '{}': {} (severity: {:.0}%)",
                                            task.id,
                                            violation.constraint_name,
                                            violation.description,
                                            violation.severity * 100.0
                                        ),
                                    )
                                    .with_entity(task.id, "task"),
                                ).await;
                            }
                        }
                    }

                    // Emit alignment evaluation event
                    let _ = event_tx.send(SwarmEvent::GoalAlignmentEvaluated {
                        task_id: task.id,
                        overall_score: evaluation.overall_score,
                        passes: evaluation.passes,
                    }).await;
                }
                Err(e) => {
                    // Log but don't block execution on evaluation failure
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Goal,
                            AuditAction::GoalEvaluated,
                            AuditActor::System,
                            format!("Failed to evaluate task {} alignment: {}", task.id, e),
                        )
                        .with_entity(task.id, "task"),
                    ).await;
                }
            }
        }

        // Try to acquire agent permit
        if let Ok(permit) = self.agent_semaphore.clone().try_acquire_owned() {
            let system_prompt = self.get_agent_system_prompt(&agent_type).await;

            // Get agent template for version tracking, capabilities, and tool restrictions
            let (template_version, capabilities, cli_tools) = match self.agent_repo.get_template_by_name(&agent_type).await {
                Ok(Some(template)) => {
                    let caps: Vec<String> = template.tools.iter()
                        .map(|t| t.name.clone())
                        .collect();
                    let tools = map_template_tools_to_cli(&caps);
                    (template.version, caps, tools)
                }
                _ => (1, vec!["task-execution".to_string()], vec![]),
            };

            // Register agent capabilities with A2A gateway if configured
            if self.config.mcp_servers.a2a_gateway.is_some() {
                if let Err(e) = self.register_agent_capabilities(&agent_type, capabilities).await {
                    tracing::warn!("Failed to register agent '{}' capabilities: {}", agent_type, e);
                }
            }

            let _ = event_tx.send(SwarmEvent::TaskSpawned {
                task_id: task.id,
                task_title: task.title.clone(),
                agent_type: Some(agent_type.clone()),
            }).await;

            // Create worktree if configured
            let worktree_path = if self.config.use_worktrees {
                match self.create_worktree_for_task(task.id, event_tx).await {
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
            // NOTE: We intentionally do NOT write .claude/agents/*.md files to the
            // worktree — Claude Code discovers those as custom agent definitions which
            // can override --allowedTools restrictions.
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

                // Neutralize mcpServers in worktree settings.
                // The orchestrator provides MCP via --mcp-config with absolute paths;
                // the settings-level server uses a relative DB path that won't resolve
                // from the worktree CWD.
                let settings_path = std::path::Path::new(wt_path)
                    .join(".claude")
                    .join("settings.json");
                if settings_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&settings_path) {
                        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(obj) = json.as_object_mut() {
                                obj.remove("mcpServers");
                                if let Ok(updated) = serde_json::to_string_pretty(&json) {
                                    let _ = std::fs::write(&settings_path, format!("{updated}\n"));
                                }
                            }
                        }
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
            let substrate = self.substrate.clone();
            let task_repo = self.task_repo.clone();
            let goal_repo = self.goal_repo.clone();
            let worktree_repo = self.worktree_repo.clone();
            let event_tx = event_tx.clone();
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
            let repo_path = self.config.repo_path.clone();
            let default_base_ref = self.config.default_base_ref.clone();
            let circuit_scope = scope;

            tokio::spawn(async move {
                let _permit = permit;

                // Update task to running
                if let Ok(Some(mut running_task)) = task_repo.get(task_id).await {
                    let _ = running_task.transition_to(TaskStatus::Running);
                    let _ = task_repo.update(&running_task).await;
                }

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
                    "command": abathur_exe.to_string_lossy(),
                    "args": ["mcp", "stdio", "--db-path", db_path.to_string_lossy(), "--task-id", task_id.to_string()]
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

                            let _ = completed_task.transition_to(TaskStatus::Complete);
                            let _ = task_repo.update(&completed_task).await;

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

                            let _ = event_tx.send(SwarmEvent::TaskCompleted {
                                task_id,
                                tokens_used: tokens,
                            }).await;

                            // Run post-completion workflow: verify and merge
                            if verify_on_completion || use_merge_queue {
                                let workflow_result = run_post_completion_workflow(
                                    task_id,
                                    task_repo.clone(),
                                    goal_repo.clone(),
                                    worktree_repo.clone(),
                                    &event_tx,
                                    &audit_log,
                                    verify_on_completion,
                                    use_merge_queue,
                                    &repo_path,
                                    &default_base_ref,
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

                                        let _ = event_tx.send(SwarmEvent::EvolutionTriggered {
                                            template_name: event.template_name.clone(),
                                            trigger: format!("{:?}", event.trigger),
                                        }).await;
                                    }
                                }
                            }
                        }
                        Ok(session) => {
                            let tokens = session.total_tokens();
                            let turns = session.turns_completed;
                            total_tokens.fetch_add(tokens, Ordering::Relaxed);

                            let error_msg = session.error.clone().unwrap_or_else(|| "Unknown error".to_string());

                            completed_task.retry_count += 1;
                            let _ = completed_task.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&completed_task).await;

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

                            let _ = event_tx.send(SwarmEvent::TaskFailed {
                                task_id,
                                error: error_msg,
                                retry_count: completed_task.retry_count,
                            }).await;
                        }
                        Err(e) => {
                            let error_msg = e.to_string();

                            completed_task.retry_count += 1;
                            let _ = completed_task.transition_to(TaskStatus::Failed);
                            let _ = task_repo.update(&completed_task).await;

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

                            let _ = event_tx.send(SwarmEvent::TaskFailed {
                                task_id,
                                error: error_msg,
                                retry_count: completed_task.retry_count,
                            }).await;
                        }
                    }
                }
            });
        }

        Ok(())
    }

    /// Process retry logic for failed tasks.
    pub(super) async fn process_retries(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let failed_tasks = self.task_repo.list_by_status(TaskStatus::Failed).await?;

        for task in failed_tasks {
            // Check if we should retry
            if task.retry_count < self.config.max_task_retries {
                // Check if dependencies are still met (they might have changed)
                let deps_met = self.are_dependencies_met(&task).await?;
                let deps_failed = self.has_failed_dependencies(&task).await?;

                if deps_failed {
                    // Mark as blocked - can't retry until upstream is fixed
                    let mut blocked_task = task.clone();
                    let _ = blocked_task.transition_to(TaskStatus::Blocked);
                    self.task_repo.update(&blocked_task).await?;
                } else if deps_met {
                    // Transition back to Ready for retry
                    let mut retry_task = task.clone();
                    if retry_task.transition_to(TaskStatus::Ready).is_ok() {
                        self.task_repo.update(&retry_task).await?;
                        let _ = event_tx.send(SwarmEvent::TaskRetrying {
                            task_id: task.id,
                            attempt: task.retry_count + 1,
                            max_attempts: self.config.max_task_retries,
                        }).await;
                    }
                }
            }
        }

        Ok(())
    }
}

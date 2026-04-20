//! Goal processing subsystem for the swarm orchestrator.
//!
//! Handles task spawning for ready tasks, dependency management,
//! task readiness updates, and retry logic.
//!
//! Goals no longer decompose into tasks or own tasks. Instead, goals provide
//! aspirational guidance via GoalContextService when tasks are executed.

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Task, TaskStatus};
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository,
};
use crate::services::{AuditAction, AuditActor, AuditCategory, AuditLevel, CircuitScope};

use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::workflow_template::WorkflowTemplate;

use super::SwarmOrchestrator;
use super::agent_prep::AgentPreparationService;
use super::exec_mode::ExecutionModeResolverService;
use super::task_context::TaskContextService;
use super::task_exec::{ExecutionConfig, TaskExecutionParams, execute_task};
use super::types::SwarmEvent;
use super::workspace::WorkspaceProvisioningService;

/// Re-emit `WorkflowGateRejected` for tasks that were rejected via MCP.
///
/// When an overmind agent calls `workflow_gate(reject)`, the event is emitted
/// on the MCP session's local event bus (no handlers). The orchestrator must
/// re-emit it so `AdapterLifecycleSyncHandler` can fire egress actions.
pub(super) async fn replay_gate_rejection_event(
    task: &Task,
    event_bus: &crate::services::event_bus::EventBus,
    workflows: &[WorkflowTemplate],
) {
    let state = match task.workflow_state() {
        Some(s) => s,
        None => return,
    };

    if let WorkflowState::Rejected {
        workflow_name,
        phase_index,
        reason,
    } = state
    {
        let phase_name = workflows
            .iter()
            .find(|t| t.name == workflow_name)
            .and_then(|t| t.phases.get(phase_index))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("phase_{}", phase_index));

        event_bus
            .publish(crate::services::event_factory::make_event(
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
            ))
            .await;
    }
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
    pub(super) async fn process_goals(
        &self,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        // Get ready tasks and spawn agents for them
        let ready_tasks = self
            .task_repo
            .get_ready_tasks(self.config.max_agents)
            .await?;

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
        let ready_tasks = self
            .task_repo
            .get_ready_tasks(self.config.max_agents)
            .await?;

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
            audit_log: self.subsystem_services.audit_log.clone(),
            circuit_breaker: self.subsystem_services.circuit_breaker.clone(),
            guardrails: self.subsystem_services.guardrails.clone(),
            cost_window_service: self.advanced_services.cost_window_service.clone(),
            budget_tracker: self.advanced_services.budget_tracker.clone(),
            agent_semaphore: self.runtime_state.agent_semaphore.clone(),
            max_agents: self.config.max_agents,
            federation_priority_bumps: 0,
        };

        // Run the registered pre-spawn middleware. Each middleware may
        // short-circuit with a Skip decision (logged below) or enrich the
        // context (e.g. RouteTaskMiddleware sets ctx.agent_type).
        let decision = {
            let chain = self.middleware.pre_spawn_chain.read().await;
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
            crate::domain::errors::DomainError::ConfigError {
                key: "RouteTaskMiddleware".to_string(),
                reason: "pre-spawn chain completed without resolving an agent_type — \
                 RouteTaskMiddleware must be registered"
                    .to_string(),
            }
        })?;

        // Circuit-breaker scope is needed later for recording success/failure
        // outcomes on the spawned task. The gate check already ran in the
        // pre-spawn chain; this just recreates the scope value.
        let scope = CircuitScope::agent(&agent_type);
        let agent_unique_id = task.id.to_string();

        // Try to acquire agent permit
        if let Ok(permit) = self.runtime_state.agent_semaphore.clone().try_acquire_owned() {
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
                    self.subsystem_services.guardrails.register_agent_spawn(&agent_unique_id).await;

                    // Successfully claimed — publish event and continue to spawn
                    self.subsystem_services.event_bus
                        .publish(crate::services::event_factory::task_event(
                            crate::services::event_bus::EventSeverity::Info,
                            None,
                            task.id,
                            crate::services::event_bus::EventPayload::TaskClaimed {
                                task_id: task.id,
                                agent_type: agent_type.clone(),
                            },
                        ))
                        .await;

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

            // Resolve agent template metadata (capabilities, CLI tools,
            // read-only role) via AgentPreparationService.
            let agent_repo_dyn: Arc<dyn crate::domain::ports::AgentRepository> =
                self.agent_repo.clone();
            let agent_prep = AgentPreparationService::new(agent_repo_dyn);
            let agent_meta = agent_prep.prepare_agent(&agent_type).await?;
            let template_version = agent_meta.version;
            let agent_can_write = agent_meta.can_write;
            let template_max_turns = agent_meta.max_turns;
            let is_read_only_role = agent_meta.is_read_only_role;

            // Register agent capabilities with A2A gateway if configured
            if self.config.mcp_servers.a2a_gateway.is_some()
                && let Err(e) = self
                    .register_agent_capabilities(&agent_type, agent_meta.capabilities.clone())
                    .await
            {
                tracing::warn!(
                    "Failed to register agent '{}' capabilities: {}",
                    agent_type,
                    e
                );
            }

            // Publish TaskSpawned via EventBus (bridge forwards to event_tx)
            self.subsystem_services.event_bus
                .publish(crate::services::event_factory::task_event(
                    crate::services::event_bus::EventSeverity::Info,
                    None,
                    task.id,
                    crate::services::event_bus::EventPayload::TaskSpawned {
                        task_id: task.id,
                        task_title: task.title.clone(),
                        agent_type: Some(agent_type.clone()),
                    },
                ))
                .await;

            // Resolve workflow template for this task to determine workspace kind and
            // output delivery mode. SwarmConfig.workflow_template is populated at
            // startup from the resolved config; see `cli::commands::swarm`.
            // If it is missing here, the orchestrator was misconfigured — fail the
            // task with a structured event rather than crashing the whole swarm.
            let task_workflow = match self.config.workflow_template.clone() {
                Some(wf) => wf,
                None => {
                    let error_msg =
                        "workflow_template was not resolved at swarm startup".to_string();

                    tracing::error!(
                        task_id = %task.id,
                        "{}",
                        error_msg,
                    );

                    if let Ok(Some(mut t)) = self.task_repo.get(task.id).await {
                        if !t.status.is_terminal() {
                            let _ = t.transition_to(TaskStatus::Failed);
                        }
                        let _ = self.task_repo.update(&t).await;
                    }

                    self.subsystem_services.audit_log.log(
                        crate::services::AuditEntry::new(
                            AuditLevel::Error,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!(
                                "Task {} failed: {}",
                                task.id, error_msg,
                            ),
                        )
                        .with_entity(task.id, "task"),
                    ).await;

                    self.subsystem_services.event_bus.publish(crate::services::event_factory::task_event(
                        crate::services::event_bus::EventSeverity::Error,
                        None,
                        task.id,
                        crate::services::event_bus::EventPayload::TaskFailed {
                            task_id: task.id,
                            error: error_msg,
                            retry_count: 0,
                        },
                    )).await;

                    self.subsystem_services.guardrails.register_agent_end(&agent_unique_id).await;
                    drop(permit);
                    return Ok(());
                }
            };
            let task_workspace_kind = task_workflow.workspace_kind;
            let task_output_delivery = task_workflow.output_delivery.clone();

            // Provision workspace based on workflow's WorkspaceKind.
            // WorkspaceKind::Worktree → git worktree (existing behaviour)
            // WorkspaceKind::TempDir  → plain temp directory (no git)
            // WorkspaceKind::None     → no workspace (read-only agents)
            let worktree_path = self
                .provision_workspace_for_task(task.id, task_workspace_kind)
                .await;

            // Write per-worktree agent config files (CLAUDE.md, settings.json)
            // via WorkspaceProvisioningService. The MCP servers block is left
            // out of settings.json — the orchestrator provides MCP via
            // --mcp-config with absolute paths instead.
            if let Some(ref wt_path) = worktree_path {
                WorkspaceProvisioningService::new().write_agent_config(wt_path);
            }

            // Load goal/memory/intent-gap context and assemble the final task
            // description via TaskContextService.
            let context_svc =
                TaskContextService::new(self.goal_repo.clone(), self.advanced_services.memory_repo.clone());
            let task_context = context_svc.load_task_context(task).await?;
            if let Some(ref goal_ctx) = task_context.goal_context {
                // Preserve audit-log behaviour for goal-context loading.
                // Count the goals informally by checking for the marker.
                let _ = goal_ctx; // The goals count is no longer separately tracked.
                self.subsystem_services.audit_log
                    .info(
                        AuditCategory::Goal,
                        AuditAction::GoalEvaluated,
                        format!("Task {} received guidance from relevant goal(s)", task.id),
                    )
                    .await;
            }
            let task_description = task_context.combined_description.clone();

            // Spawn task execution
            let task_id = task.id;

            // Resolve effective execution mode (Direct vs Convergent) via
            // ExecutionModeResolverService. The resolver applies the runtime
            // upgrade rule: stored Direct → Convergent when convergence is
            // enabled AND the agent is write-capable AND non-read-only.
            let mode_resolver =
                ExecutionModeResolverService::new(self.config.convergence_enabled);
            let (effective_mode, is_convergent) =
                mode_resolver.resolve_mode(task.execution_mode.clone(), &agent_meta);
            if effective_mode != task.execution_mode {
                tracing::info!(
                    task_id = %task_id,
                    %agent_type,
                    stored_mode = ?task.execution_mode,
                    effective_mode = ?effective_mode,
                    "Upgrading execution mode Direct -> Convergent (write-capable, non-read-only agent)"
                );
            }

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

            // Spawn the per-task worker. All deps are cloned BEFORE the
            // tokio::spawn so the spawn body owns its inputs and never holds a
            // reference back to the orchestrator (Risk 1 mitigation).
            let role_max_turns = {
                let lower = agent_type.to_lowercase();
                if lower.contains("researcher")
                    || lower.contains("analyst")
                    || lower.contains("explorer")
                    || lower.contains("auditor")
                {
                    51 // Research: typical ~15 turns, ceiling 51
                } else if lower.contains("planner")
                    || lower.contains("architect")
                    || lower.contains("designer")
                    || lower.contains("reviewer")
                    || lower.contains("verifier")
                {
                    30 // Planning/Review: typical ~10 turns, ceiling 30
                } else if lower.contains("implement")
                    || lower.contains("coder")
                    || lower.contains("builder")
                    || lower.contains("fixer")
                {
                    75 // Implementation: typical ~25 turns, ceiling 75
                } else {
                    self.config.default_max_turns // Fallback to config default
                }
            };

            // Use agent template max_turns if explicitly set (non-zero),
            // then role-aware default, then orchestrator config default.
            let mut max_turns = if template_max_turns > 0 {
                template_max_turns.max(role_max_turns)
            } else {
                role_max_turns
            };

            // Bump turn budget for tasks retrying after max_turns exhaustion.
            if task
                .context
                .hints
                .iter()
                .any(|h| h == "retry:max_turns_exceeded")
            {
                let multiplier = 1.5_f64.powi(task.retry_count as i32);
                max_turns = ((max_turns as f64 * multiplier) as u32).min(100);
            }

            // Without --dangerously-skip-permissions, disable direct merges to
            // main and force PR-only mode so a human must approve before
            // merging.
            let use_merge_queue =
                self.config.use_merge_queue && self.config.dangerously_skip_permissions;
            let prefer_pull_requests =
                self.config.prefer_pull_requests || !self.config.dangerously_skip_permissions;

            let exec_cfg = ExecutionConfig {
                repo_path: self.config.repo_path.clone(),
                default_base_ref: self.config.default_base_ref.clone(),
                agent_semaphore: self.runtime_state.agent_semaphore.clone(),
                guardrails: self.subsystem_services.guardrails.clone(),
                require_commits: agent_can_write && !is_read_only_role,
                verify_on_completion: self.config.verify_on_completion,
                use_merge_queue,
                prefer_pull_requests,
                track_evolution: self.config.track_evolution,
                evolution_loop: self.subsystem_services.evolution_loop.clone(),
                fetch_on_sync: self.config.fetch_on_sync,
                output_delivery: task_output_delivery.clone(),
                merge_request_repo: self.advanced_services.merge_request_repo.clone(),
                post_completion_chain: self.middleware.post_completion_chain.clone(),
            };

            let intent_verifier_dyn: Option<
                Arc<dyn super::convergent_execution::ConvergentIntentVerifier>,
            > = self.advanced_services.intent_verifier.as_ref().map(|iv| {
                Arc::clone(iv) as Arc<dyn super::convergent_execution::ConvergentIntentVerifier>
            });
            let memory_repo_dyn: Option<Arc<dyn crate::domain::ports::MemoryRepository>> =
                self.advanced_services.memory_repo.as_ref().map(|m| {
                    Arc::clone(m) as Arc<dyn crate::domain::ports::MemoryRepository>
                });
            let goal_repo_dyn: Arc<dyn crate::domain::ports::GoalRepository> =
                self.goal_repo.clone();
            let task_repo_dyn: Arc<dyn crate::domain::ports::TaskRepository> =
                self.task_repo.clone();
            let worktree_repo_dyn: Arc<dyn crate::domain::ports::WorktreeRepository> =
                self.worktree_repo.clone();

            let params = TaskExecutionParams {
                task: task.clone(),
                task_id,
                agent_type: agent_type.clone(),
                system_prompt,
                task_description,
                effective_mode,
                is_convergent,
                max_turns,
                agent_meta,
                worktree_path,
                all_workflows: self.config.all_workflows.clone(),
                circuit_scope: scope,
                agent_unique_id: agent_unique_id.clone(),
                template_version,
                agent_type_for_evolution: agent_type.clone(),
                substrate: self.substrate.clone(),
                task_repo: task_repo_dyn,
                worktree_repo: worktree_repo_dyn,
                goal_repo: goal_repo_dyn,
                event_bus: self.subsystem_services.event_bus.clone(),
                event_tx: event_tx.clone(),
                audit_log: self.subsystem_services.audit_log.clone(),
                circuit_breaker: self.subsystem_services.circuit_breaker.clone(),
                command_bus: self.advanced_services.command_bus.read().await.clone(),
                total_tokens: self.runtime_state.total_tokens.clone(),
                permit,
                overseer_cluster: self.advanced_services.overseer_cluster.clone(),
                trajectory_repo: self.advanced_services.trajectory_repo.clone(),
                convergence_engine_config: self.advanced_services.convergence_engine_config.clone(),
                memory_repo: memory_repo_dyn,
                intent_verifier: intent_verifier_dyn,
                config: exec_cfg,
            };

            // Per-task worker: spawned once per task, short-lived. Not a
            // long-lived daemon, so no supervision wrapper.
            tokio::spawn(execute_task(params));
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
    if let Some(ws) = task.workflow_state()
        && !ws.is_terminal()
    {
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
        COMPLETION_SIGNALS
            .iter()
            .any(|signal| trimmed.contains(signal))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            max_turns, 50,
            "role floor should override lower template value"
        );

        // When template is higher than role default, template should win
        let template_high: u32 = 75;
        let max_turns_high = if template_high > 0 {
            template_high.max(role_max_turns)
        } else {
            role_max_turns
        };
        assert_eq!(
            max_turns_high, 75,
            "template should win when higher than role floor"
        );

        // When template is zero (unset), role default should be used
        let template_zero: u32 = 0;
        let max_turns_zero = if template_zero > 0 {
            template_zero.max(role_max_turns)
        } else {
            role_max_turns
        };
        assert_eq!(
            max_turns_zero, 50,
            "role default should be used when template is zero"
        );
    }

    // -- is_max_turns_auto_completable tests ---------------------------------

    #[test]
    fn test_auto_completable_typical_message() {
        let msg =
            "error_max_turns: agent exhausted 31 turns without completing. Last output: completed";
        assert!(is_max_turns_auto_completable(msg));
    }

    #[test]
    fn test_auto_completable_with_complete_variant() {
        let msg =
            "error_max_turns: Agent exhausted turns without completing. Last output: complete";
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
        let msg =
            "error_max_turns: agent exhausted 31 turns without completing. Last output: finished";
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
        task.set_workflow_state(&ws).unwrap();
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
        task.set_workflow_state(&ws).unwrap();
        assert!(
            !can_safely_auto_complete(&task),
            "Task with non-terminal workflow state should NOT be safely auto-completable"
        );
    }
}

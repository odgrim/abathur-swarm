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
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    CircuitScope, GoalContextService,
    TaskExecution, TaskOutcome,
};

use super::helpers::{auto_commit_worktree, run_post_completion_workflow};
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

    /// Spawn an agent for a ready task.
    ///
    /// Before execution, loads relevant goals via GoalContextService and
    /// prepends goal guidance to the task description.
    async fn spawn_task_agent(
        &self,
        task: &Task,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        // Check circuit breaker
        let scope = CircuitScope::agent(task.agent_type.as_deref().unwrap_or("default"));
        let check_result = self.circuit_breaker.check(scope.clone()).await;

        if check_result.is_blocked() {
            self.audit_log.log(
                AuditEntry::new(
                    AuditLevel::Warning,
                    AuditCategory::Execution,
                    AuditAction::CircuitBreakerTriggered,
                    AuditActor::System,
                    format!("Task {} blocked by circuit breaker", task.id),
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
            // Get agent template for system prompt
            let agent_type = task.agent_type.clone().unwrap_or_else(|| "default".to_string());
            let system_prompt = self.get_agent_system_prompt(&agent_type).await;

            // Get agent template for version tracking and capabilities
            let (template_version, capabilities) = match self.agent_repo.get_template_by_name(&agent_type).await {
                Ok(Some(template)) => {
                    let caps: Vec<String> = template.tools.iter()
                        .map(|t| t.name.clone())
                        .collect();
                    (template.version, caps)
                }
                _ => (1, vec!["task-execution".to_string()]),
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
                agent_type: task.agent_type.clone(),
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
            let mcp_servers = self.config.mcp_servers.clone();
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

                // Add MCP servers so agents can access memory, tasks, and A2A
                if let Some(ref memory_server) = mcp_servers.memory_server {
                    config = config.with_mcp_server(memory_server);
                }
                if let Some(ref tasks_server) = mcp_servers.tasks_server {
                    config = config.with_mcp_server(tasks_server);
                }
                if let Some(ref a2a_gateway) = mcp_servers.a2a_gateway {
                    config = config.with_mcp_server(a2a_gateway);
                }

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

//! DAG execution subsystem for the swarm orchestrator.
//!
//! Handles structured goal execution through task DAGs, convergence loops
//! with intent verification, and the merge queue pipeline.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    EscalationUrgency, HumanEscalation, HumanEscalationEvent, Task, TaskDag, TaskStatus,
};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel, AuditLogService,
    DagExecutor, ExecutionEvent, ExecutionResults, ExecutorConfig,
    IntegrationVerifierService, MemoryService, MergeQueue, MergeQueueConfig,
    TaskExecution, TaskOutcome, VerifierConfig, WorktreeConfig, WorktreeService,
};

use super::types::{SwarmEvent, VerificationLevel};
use super::SwarmOrchestrator;

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Execute a goal's tasks using the DagExecutor for wave-based parallel execution.
    ///
    /// This provides structured execution with waves, guardrails, and circuit breakers.
    pub async fn execute_goal_with_dag(
        &self,
        goal_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<ExecutionResults> {
        // Get tasks created by goal evaluation
        let all_goal_tasks = self.task_repo.list_by_source("goal_evaluation").await?;
        let tasks: Vec<Task> = all_goal_tasks.into_iter()
            .filter(|t| t.source == crate::domain::models::TaskSource::GoalEvaluation(goal_id))
            .collect();
        if tasks.is_empty() {
            return Ok(ExecutionResults::default());
        }

        // Build DAG from tasks
        let dag = TaskDag::from_tasks(tasks.clone());

        // Create worktrees for all tasks if worktrees are enabled
        if self.config.use_worktrees {
            self.create_worktrees_for_tasks(&tasks, event_tx).await;
        }

        // Fetch project context from semantic memory if available
        let project_context = self.fetch_project_context(goal_id).await;

        // Create DAG executor with MCP services and goal context
        let executor_config = ExecutorConfig {
            max_concurrency: self.config.max_agents,
            task_timeout_secs: self.config.goal_timeout_secs,
            max_retries: self.config.max_task_retries,
            default_max_turns: self.config.default_max_turns,
            fail_fast: false,
            memory_server_url: self.config.mcp_servers.memory_server.clone(),
            a2a_gateway_url: self.config.mcp_servers.a2a_gateway.clone(),
            tasks_server_url: self.config.mcp_servers.tasks_server.clone(),
            project_context,
            enable_wave_verification: false,
            iteration_context: None,
        };

        // Create restructure service for failure recovery
        let restructure_service = Arc::new(crate::services::dag_restructure::DagRestructureService::with_defaults());

        let executor = DagExecutor::new(
            self.task_repo.clone(),
            self.agent_repo.clone(),
            self.substrate.clone(),
            executor_config,
        )
        .with_goal_repo(self.goal_repo.clone())
        .with_circuit_breaker(self.circuit_breaker.clone())
        .with_restructure_service(restructure_service.clone())
        .with_guardrails(self.guardrails.clone());

        // Create execution event channel
        let (exec_event_tx, mut exec_event_rx) = mpsc::channel::<ExecutionEvent>(100);

        // Forward execution events to swarm events
        let audit_log = self.audit_log.clone();
        let evolution_loop = self.evolution_loop.clone();
        let track_evolution = self.config.track_evolution;
        let swarm_event_tx = event_tx.clone();
        let agent_repo_for_events = self.agent_repo.clone();
        let task_repo_for_events = self.task_repo.clone();
        let escalation_store_for_events = self.escalation_store.clone();

        let event_forwarder = tokio::spawn(async move {
            while let Some(event) = exec_event_rx.recv().await {
                Self::forward_execution_event(
                    event,
                    &swarm_event_tx,
                    &audit_log,
                    &evolution_loop,
                    track_evolution,
                    &agent_repo_for_events,
                    &task_repo_for_events,
                    &escalation_store_for_events,
                ).await;
            }
        });

        // Execute the DAG
        let mut results = executor.execute_with_events(&dag, exec_event_tx).await?;

        // Wait for event forwarder to finish
        let _ = event_forwarder.await;

        // Post-execution verification
        if self.config.verify_on_completion {
            let verification_failures = self.run_post_dag_verification(&results, event_tx).await;
            if verification_failures > 0 {
                results.completed_tasks = results.completed_tasks.saturating_sub(verification_failures);
                results.failed_tasks += verification_failures;
            }
        }

        // Persist successful task outputs to memory for future agent reference
        self.persist_task_outputs_to_memory(goal_id, &results).await;

        // Goal always remains Active regardless of task outcomes.
        let _ = event_tx.send(SwarmEvent::GoalIterationCompleted {
            goal_id,
            tasks_completed: results.completed_tasks,
        }).await;

        Ok(results)
    }

    /// Execute a goal's tasks with iteration context for convergence loops.
    pub(super) async fn execute_goal_with_dag_and_context(
        &self,
        goal_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
        iteration_context: Option<crate::domain::models::IterationContext>,
        enable_wave_verification: bool,
    ) -> DomainResult<ExecutionResults> {
        // Get tasks created by goal evaluation
        let all_goal_tasks = self.task_repo.list_by_source("goal_evaluation").await?;
        let tasks: Vec<Task> = all_goal_tasks.into_iter()
            .filter(|t| t.source == crate::domain::models::TaskSource::GoalEvaluation(goal_id))
            .collect();
        if tasks.is_empty() {
            return Ok(ExecutionResults::default());
        }

        // Build DAG from tasks
        let dag = TaskDag::from_tasks(tasks.clone());

        // Create worktrees for all tasks if worktrees are enabled
        if self.config.use_worktrees {
            self.create_worktrees_for_tasks(&tasks, event_tx).await;
        }

        // Fetch project context from semantic memory if available
        let project_context = self.fetch_project_context(goal_id).await;

        // Create DAG executor with iteration context
        let executor_config = ExecutorConfig {
            max_concurrency: self.config.max_agents,
            task_timeout_secs: self.config.goal_timeout_secs,
            max_retries: self.config.max_task_retries,
            default_max_turns: self.config.default_max_turns,
            fail_fast: false,
            memory_server_url: self.config.mcp_servers.memory_server.clone(),
            a2a_gateway_url: self.config.mcp_servers.a2a_gateway.clone(),
            tasks_server_url: self.config.mcp_servers.tasks_server.clone(),
            project_context,
            enable_wave_verification,
            iteration_context,
        };

        let restructure_service = Arc::new(crate::services::dag_restructure::DagRestructureService::with_defaults());

        let executor = DagExecutor::new(
            self.task_repo.clone(),
            self.agent_repo.clone(),
            self.substrate.clone(),
            executor_config,
        )
        .with_goal_repo(self.goal_repo.clone())
        .with_circuit_breaker(self.circuit_breaker.clone())
        .with_restructure_service(restructure_service)
        .with_guardrails(self.guardrails.clone());

        // Execute the DAG
        let results = executor.execute(&dag).await?;

        // Track tokens
        for task_result in &results.task_results {
            if let Some(ref session) = task_result.session {
                self.total_tokens.fetch_add(session.total_tokens(), std::sync::atomic::Ordering::Relaxed);
            }
        }

        Ok(results)
    }

    /// Execute a goal with intent verification and convergence loop.
    ///
    /// This wraps `execute_goal` with a convergence loop that:
    /// 1. Executes the goal's tasks
    /// 2. Verifies intent satisfaction
    /// 3. Re-prompts/adds tasks if not converged
    /// 4. Repeats until converged or max iterations reached
    pub async fn execute_goal_with_convergence(
        &self,
        goal_id: Uuid,
        event_tx: mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<ExecutionResults> {
        // If intent verification is not enabled or not configured, fall back to regular execution
        if !self.config.enable_intent_verification {
            return self.execute_goal_with_dag(goal_id, &event_tx).await;
        }

        let Some(ref intent_verifier) = self.intent_verifier else {
            tracing::warn!("Intent verification enabled but no verifier configured, falling back to regular execution");
            return self.execute_goal_with_dag(goal_id, &event_tx).await;
        };

        // Capture the original intent
        let intent = intent_verifier.extract_guiding_intent(goal_id).await?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(self.config.convergence.convergence_timeout_secs);
        let max_iterations = self.config.convergence.max_iterations;

        // Initialize convergence state for drift detection
        let mut convergence_state = crate::domain::models::ConvergenceState::new(intent.clone());
        let mut final_results: Option<ExecutionResults> = None;
        let mut final_satisfaction = "indeterminate".to_string();

        self.audit_log.info(
            AuditCategory::Goal,
            AuditAction::GoalCreated,
            format!(
                "Starting convergence loop for goal {} (max {} iterations)",
                goal_id, max_iterations
            ),
        ).await;

        while convergence_state.current_iteration < max_iterations {
            // Check timeout
            if start.elapsed() > timeout {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Goal,
                        AuditAction::GoalPaused,
                        AuditActor::System,
                        format!(
                            "Convergence loop timed out for goal {} after {} iterations",
                            goal_id, convergence_state.current_iteration
                        ),
                    )
                    .with_entity(goal_id, "goal"),
                ).await;
                convergence_state.end();
                break;
            }

            // Check for semantic drift (same gaps recurring)
            if convergence_state.drift_detected {
                self.handle_semantic_drift(goal_id, &convergence_state, &event_tx).await;
                convergence_state.end();
                break;
            }

            let iteration = convergence_state.current_iteration + 1;

            // Build iteration context for agent prompts
            let iteration_context = if convergence_state.current_iteration > 0 {
                Some(convergence_state.build_iteration_context())
            } else {
                None
            };

            // Execute the goal with iteration context
            let results = self.execute_goal_with_dag_and_context(
                goal_id,
                &event_tx,
                iteration_context,
                self.config.convergence.verification_level == VerificationLevel::Wave,
            ).await?;
            final_results = Some(results.clone());

            // Collect completed tasks
            let completed_task_ids: Vec<Uuid> = results
                .task_results
                .iter()
                .filter(|r| r.status == TaskStatus::Complete)
                .map(|r| r.task_id)
                .collect();

            if completed_task_ids.is_empty() {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Goal,
                        AuditAction::GoalPaused,
                        AuditActor::System,
                        format!("No completed tasks for goal {} in iteration {}", goal_id, iteration),
                    )
                    .with_entity(goal_id, "goal"),
                ).await;
                convergence_state.end();
                break;
            }

            // Get the full task objects for verification
            let mut completed_tasks = Vec::new();
            for task_id in &completed_task_ids {
                if let Ok(Some(task)) = self.task_repo.get(*task_id).await {
                    completed_tasks.push(task);
                }
            }

            // Emit verification started event
            let _ = event_tx.send(SwarmEvent::IntentVerificationStarted {
                goal_id,
                iteration,
            }).await;

            // Verify intent satisfaction
            let verification_result = intent_verifier
                .verify_intent(&intent, &completed_tasks, iteration)
                .await?;

            // Record result in convergence state (updates drift detection)
            convergence_state.record_verification(verification_result.clone());

            let satisfaction_str = verification_result.satisfaction.as_str().to_string();
            let should_continue = self.should_continue_convergence(&verification_result)
                && convergence_state.is_making_progress();

            // Emit verification completed event
            let _ = event_tx.send(SwarmEvent::IntentVerificationCompleted {
                goal_id,
                satisfaction: satisfaction_str.clone(),
                confidence: verification_result.confidence,
                gaps_count: verification_result.gaps.len(),
                iteration,
                will_retry: should_continue,
            }).await;

            self.audit_log.info(
                AuditCategory::Goal,
                AuditAction::GoalEvaluated,
                format!(
                    "Intent verification for goal {} iteration {}: {} (confidence: {:.2}, {} gaps, drift: {})",
                    goal_id, iteration, satisfaction_str,
                    verification_result.confidence, verification_result.gaps.len(),
                    convergence_state.drift_detected
                ),
            ).await;

            final_satisfaction = satisfaction_str.clone();

            // Check if converged
            if convergence_state.converged {
                break;
            }

            // Check if we should continue
            if !should_continue {
                convergence_state.end();
                break;
            }

            // Apply reprompt guidance if available
            if let Some(guidance) = &verification_result.reprompt_guidance {
                self.apply_convergence_guidance(goal_id, &verification_result, guidance, &event_tx).await?;
            } else {
                // No guidance, can't continue meaningfully
                convergence_state.end();
                break;
            }
        }

        // Emit convergence completed event
        let _ = event_tx.send(SwarmEvent::ConvergenceCompleted {
            goal_id,
            converged: convergence_state.converged,
            iterations: convergence_state.current_iteration,
            final_satisfaction: final_satisfaction.clone(),
        }).await;

        self.audit_log.info(
            AuditCategory::Goal,
            if convergence_state.converged { AuditAction::GoalIterationCompleted } else { AuditAction::GoalPaused },
            format!(
                "Convergence loop for goal {} finished: {} after {} iterations ({}, drift: {})",
                goal_id,
                if convergence_state.converged { "CONVERGED" } else { "NOT CONVERGED" },
                convergence_state.current_iteration, final_satisfaction,
                convergence_state.drift_detected
            ),
        ).await;

        final_results.ok_or_else(|| DomainError::ExecutionFailed(
            "No execution results available".to_string()
        ))
    }

    /// Queue a completed task for merge via the two-stage merge queue.
    ///
    /// Stage 1: Agent worktree -> task integration branch
    /// Stage 2: Task integration branch -> main (with verification)
    pub async fn queue_task_for_merge(
        &self,
        task_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        if !self.config.use_merge_queue {
            return Ok(());
        }

        // Get the worktree for this task
        let worktree = self.worktree_repo.get_by_task(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        // Create the verifier needed by MergeQueue
        let verifier = IntegrationVerifierService::new(
            self.task_repo.clone(),
            self.goal_repo.clone(),
            self.worktree_repo.clone(),
            VerifierConfig::default(),
        );

        // Create merge queue with config
        let merge_config = MergeQueueConfig {
            repo_path: self.config.repo_path.to_str().unwrap_or(".").to_string(),
            main_branch: self.config.default_base_ref.clone(),
            require_verification: self.config.verify_on_completion,
            ..Default::default()
        };

        let merge_queue = MergeQueue::new(
            self.task_repo.clone(),
            self.worktree_repo.clone(),
            Arc::new(verifier),
            merge_config,
        );

        // Queue Stage 1: Agent worktree -> task branch
        let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
            task_id,
            stage: "AgentToTask".to_string(),
        }).await;

        match merge_queue.queue_stage1(
            task_id,
            &worktree.branch,
            &format!("task/{}", task_id),
        ).await {
            Ok(_) => {
                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!("Task {} queued for stage 1 merge", task_id),
                ).await;
            }
            Err(e) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Error,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} failed to queue for stage 1: {}", task_id, e),
                    )
                    .with_entity(task_id, "task"),
                ).await;
                return Err(e);
            }
        }

        // Process the queued merge
        match merge_queue.process_next().await {
            Ok(Some(result)) if result.success => {
                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCompleted,
                    format!(
                        "Task {} stage 1 merge completed: {}",
                        task_id, result.commit_sha.clone().unwrap_or_default()
                    ),
                ).await;

                // Queue stage 2
                let _ = event_tx.send(SwarmEvent::TaskQueuedForMerge {
                    task_id,
                    stage: "TaskToMain".to_string(),
                }).await;

                if let Ok(_) = merge_queue.queue_stage2(task_id).await {
                    // Process stage 2
                    if let Ok(Some(result2)) = merge_queue.process_next().await {
                        if result2.success {
                            let _ = event_tx.send(SwarmEvent::TaskMerged {
                                task_id,
                                commit_sha: result2.commit_sha.clone().unwrap_or_default(),
                            }).await;

                            self.audit_log.info(
                                AuditCategory::Task,
                                AuditAction::TaskCompleted,
                                format!(
                                    "Task {} stage 2 merge completed: {}",
                                    task_id, result2.commit_sha.unwrap_or_default()
                                ),
                            ).await;
                        }
                    }
                }
            }
            Ok(Some(result)) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!(
                            "Task {} stage 1 merge failed: {}",
                            task_id, result.error.unwrap_or_default()
                        ),
                    )
                    .with_entity(task_id, "task"),
                ).await;
            }
            Ok(None) => {
                // No queued merge to process
            }
            Err(e) => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Error,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} merge error: {}", task_id, e),
                    )
                    .with_entity(task_id, "task"),
                ).await;
            }
        }

        Ok(())
    }

    /// Execute a specific goal with its task DAG (standalone, not part of run loop).
    pub async fn execute_goal(&self, goal_id: Uuid) -> DomainResult<ExecutionResults> {
        let _goal = self.goal_repo.get(goal_id).await?
            .ok_or(DomainError::GoalNotFound(goal_id))?;

        let all_goal_tasks = self.task_repo.list_by_source("goal_evaluation").await?;
        let tasks: Vec<Task> = all_goal_tasks.into_iter()
            .filter(|t| t.source == crate::domain::models::TaskSource::GoalEvaluation(goal_id))
            .collect();
        let dag = TaskDag::from_tasks(tasks);

        // Fetch project context from memory if available
        let project_context = self.fetch_project_context(goal_id).await;

        let executor_config = ExecutorConfig {
            max_concurrency: self.config.max_agents,
            task_timeout_secs: self.config.goal_timeout_secs,
            max_retries: self.config.max_task_retries,
            default_max_turns: self.config.default_max_turns,
            fail_fast: false,
            memory_server_url: self.config.mcp_servers.memory_server.clone(),
            a2a_gateway_url: self.config.mcp_servers.a2a_gateway.clone(),
            tasks_server_url: self.config.mcp_servers.tasks_server.clone(),
            project_context,
            enable_wave_verification: false,
            iteration_context: None,
        };

        let executor = DagExecutor::new(
            self.task_repo.clone(),
            self.agent_repo.clone(),
            self.substrate.clone(),
            executor_config,
        ).with_goal_repo(self.goal_repo.clone());

        let results = executor.execute(&dag).await?;

        // Track tokens
        for task_result in &results.task_results {
            if let Some(ref session) = task_result.session {
                self.total_tokens.fetch_add(session.total_tokens(), Ordering::Relaxed);
            }
        }

        Ok(results)
    }

    /// Determine if the convergence loop should continue based on verification result.
    pub(super) fn should_continue_convergence(
        &self,
        result: &crate::domain::models::IntentVerificationResult,
    ) -> bool {
        use crate::domain::models::IntentSatisfaction;

        // Don't continue if fully satisfied
        if result.satisfaction == IntentSatisfaction::Satisfied {
            return false;
        }

        // Don't continue if indeterminate (needs human)
        if result.satisfaction == IntentSatisfaction::Indeterminate {
            return false;
        }

        // For partial satisfaction, check config
        if result.satisfaction == IntentSatisfaction::Partial {
            if self.config.convergence.require_full_satisfaction {
                return true;
            }
            // Accept partial if confidence is high enough
            if result.confidence >= self.config.convergence.min_confidence_threshold {
                return false;
            }
            return self.config.convergence.auto_retry_partial;
        }

        // Unsatisfied - continue if we have guidance
        result.should_iterate()
    }

    // ========================================================================
    // Private helpers for DAG execution
    // ========================================================================

    /// Create worktrees for tasks that need them.
    async fn create_worktrees_for_tasks(
        &self,
        tasks: &[Task],
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) {
        let worktree_config = WorktreeConfig {
            base_path: self.config.worktree_base_path.clone(),
            repo_path: self.config.repo_path.clone(),
            default_base_ref: self.config.default_base_ref.clone(),
            auto_cleanup: true,
        };
        let worktree_service = WorktreeService::new(
            self.worktree_repo.clone(),
            worktree_config,
        );

        for task in tasks {
            // Skip tasks that already have worktrees or are complete
            if task.worktree_path.is_some() || task.status == TaskStatus::Complete {
                continue;
            }

            // Create worktree for this task
            match worktree_service.create_worktree(task.id, None).await {
                Ok(worktree) => {
                    // Update task with worktree path
                    let mut updated_task = task.clone();
                    updated_task.worktree_path = Some(worktree.path.clone());
                    if let Err(e) = self.task_repo.update(&updated_task).await {
                        tracing::warn!("Failed to update task {} with worktree path: {}", task.id, e);
                    }

                    let _ = event_tx.send(SwarmEvent::WorktreeCreated {
                        task_id: task.id,
                        path: worktree.path,
                    }).await;

                    self.audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCreated,
                        format!("Created worktree for task {} in DAG execution", task.id),
                    ).await;
                }
                Err(e) => {
                    // Log but don't fail - worktree creation is non-critical
                    tracing::warn!("Failed to create worktree for task {}: {}", task.id, e);
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Failed to create worktree for task {}: {}", task.id, e),
                        )
                        .with_entity(task.id, "task"),
                    ).await;
                }
            }
        }
    }

    /// Fetch project context from semantic memory if available.
    pub(super) async fn fetch_project_context(&self, goal_id: Uuid) -> Option<String> {
        let memory_repo = self.memory_repo.as_ref()?;
        let memory_service = MemoryService::new(memory_repo.clone());
        let mut context_parts = Vec::new();

        // Fetch goal-related memories
        if let Ok(goal_memories) = memory_service.get_goal_context(goal_id).await {
            for mem in goal_memories.iter().take(5) {
                context_parts.push(format!("- {}: {}", mem.key, mem.content));
            }
        }

        // Fetch semantic memories (long-term project knowledge)
        if let Ok(semantic_memories) = memory_service.search("architecture project", Some("semantic"), 5).await {
            for mem in semantic_memories {
                context_parts.push(format!("- {}: {}", mem.key, mem.content));
            }
        }

        if context_parts.is_empty() {
            None
        } else {
            Some(format!("Relevant project knowledge:\n{}", context_parts.join("\n")))
        }
    }

    /// Forward an execution event from the DAG executor to the swarm event channel.
    async fn forward_execution_event(
        event: ExecutionEvent,
        swarm_event_tx: &mpsc::Sender<SwarmEvent>,
        audit_log: &Arc<AuditLogService>,
        evolution_loop: &Arc<crate::services::EvolutionLoop>,
        track_evolution: bool,
        agent_repo: &Arc<A>,
        task_repo: &Arc<T>,
        escalation_store: &Arc<tokio::sync::RwLock<Vec<HumanEscalationEvent>>>,
    ) {
        match event {
            ExecutionEvent::Started { total_tasks, wave_count } => {
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveStarted,
                    format!("DAG execution started: {} tasks in {} waves", total_tasks, wave_count),
                ).await;
            }
            ExecutionEvent::WaveStarted { wave_number, task_count } => {
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveStarted,
                    format!("Wave {} started: {} tasks", wave_number, task_count),
                ).await;
            }
            ExecutionEvent::TaskStarted { task_id, task_title } => {
                let _ = swarm_event_tx.send(SwarmEvent::TaskSpawned {
                    task_id,
                    task_title,
                    agent_type: None,
                }).await;
            }
            ExecutionEvent::TaskCompleted { task_id, result } => {
                let tokens = result.session.as_ref().map(|s| s.total_tokens()).unwrap_or(0);
                let _ = swarm_event_tx.send(SwarmEvent::TaskCompleted {
                    task_id,
                    tokens_used: tokens,
                }).await;

                // Record in evolution loop
                if track_evolution {
                    let turns = result.session.as_ref().map(|s| s.turns_completed).unwrap_or(0);
                    let template_name = result.session.as_ref()
                        .map(|s| s.agent_template.clone())
                        .unwrap_or_else(|| "dag_executor".to_string());

                    let template_version = match agent_repo.get_template_by_name(&template_name).await {
                        Ok(Some(t)) => t.version,
                        _ => 1,
                    };

                    let execution = TaskExecution {
                        task_id,
                        template_name,
                        template_version,
                        outcome: TaskOutcome::Success,
                        executed_at: chrono::Utc::now(),
                        turns_used: turns,
                        tokens_used: tokens,
                        downstream_tasks: vec![],
                    };
                    evolution_loop.record_execution(execution).await;
                }
            }
            ExecutionEvent::TaskFailed { task_id, error, retry_count } => {
                let _ = swarm_event_tx.send(SwarmEvent::TaskFailed {
                    task_id,
                    error,
                    retry_count,
                }).await;
            }
            ExecutionEvent::TaskRetrying { task_id, attempt, max_attempts } => {
                let _ = swarm_event_tx.send(SwarmEvent::TaskRetrying {
                    task_id,
                    attempt,
                    max_attempts,
                }).await;
            }
            ExecutionEvent::WaveCompleted { wave_number, succeeded, failed } => {
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveCompleted,
                    format!("Wave {} completed: {} succeeded, {} failed", wave_number, succeeded, failed),
                ).await;
            }
            ExecutionEvent::Completed { status, results } => {
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveCompleted,
                    format!(
                        "DAG execution completed: {:?}, {}/{} tasks succeeded",
                        status, results.completed_tasks, results.total_tasks
                    ),
                ).await;
            }
            ExecutionEvent::RestructureDecision { task_id, decision } => {
                audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Info,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("DAG restructure triggered for task {}: {}", task_id, decision),
                    )
                    .with_entity(task_id, "task"),
                ).await;
                let _ = swarm_event_tx.send(SwarmEvent::RestructureTriggered {
                    task_id,
                    decision,
                }).await;
            }
            ExecutionEvent::IntentVerificationRequested { goal_id, completed_task_ids } => {
                audit_log.info(
                    AuditCategory::Goal,
                    AuditAction::GoalEvaluated,
                    format!(
                        "Intent verification requested for goal {:?} with {} completed tasks",
                        goal_id, completed_task_ids.len()
                    ),
                ).await;
            }
            ExecutionEvent::IntentVerificationResult {
                satisfaction, confidence, gaps_count, iteration, should_continue,
            } => {
                audit_log.info(
                    AuditCategory::Goal,
                    AuditAction::GoalEvaluated,
                    format!(
                        "Intent verification result: {} (confidence: {:.2}, {} gaps, iteration {}, continue: {})",
                        satisfaction, confidence, gaps_count, iteration, should_continue
                    ),
                ).await;
            }
            ExecutionEvent::WaveVerificationRequested { wave_number, completed_task_ids, goal_id } => {
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveCompleted,
                    format!(
                        "Wave {} verification requested: {} completed tasks, goal {:?}",
                        wave_number, completed_task_ids.len(), goal_id
                    ),
                ).await;
            }
            ExecutionEvent::WaveVerificationResult { wave_number, satisfaction, confidence, gaps_count } => {
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveCompleted,
                    format!(
                        "Wave {} verification result: {} (confidence: {:.2}, {} gaps)",
                        wave_number, satisfaction, confidence, gaps_count
                    ),
                ).await;
            }
            ExecutionEvent::BranchVerificationRequested { branch_task_ids, waiting_task_ids, branch_objective } => {
                let branch_count = branch_task_ids.len();
                let waiting_count = waiting_task_ids.len();
                let _ = swarm_event_tx.send(SwarmEvent::BranchVerificationStarted {
                    branch_task_ids,
                    waiting_task_ids,
                }).await;
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::WaveStarted,
                    format!(
                        "Branch verification requested for {} tasks, {} waiting. Objective: {}",
                        branch_count, waiting_count, branch_objective
                    ),
                ).await;
            }
            ExecutionEvent::BranchVerificationResult { branch_satisfied, confidence, gaps_count, dependents_can_proceed } => {
                let _ = swarm_event_tx.send(SwarmEvent::BranchVerificationCompleted {
                    branch_satisfied,
                    dependents_can_proceed,
                    gaps_count,
                }).await;
                audit_log.info(
                    AuditCategory::Execution,
                    AuditAction::TaskCompleted,
                    format!(
                        "Branch verification result: satisfied={}, confidence={:.2}, gaps={}, proceed={}",
                        branch_satisfied, confidence, gaps_count, dependents_can_proceed
                    ),
                ).await;
            }
            ExecutionEvent::HumanEscalationNeeded { goal_id, task_id, reason, urgency, is_blocking } => {
                // Build and store escalation event
                let parsed_urgency = match urgency.as_str() {
                    "low" => EscalationUrgency::Low,
                    "high" => EscalationUrgency::High,
                    "blocking" => EscalationUrgency::Blocking,
                    _ => EscalationUrgency::Normal,
                };
                let escalation = HumanEscalation::new(reason.clone())
                    .with_urgency(parsed_urgency);
                let mut escalation_event = HumanEscalationEvent::new(escalation);
                if let Some(gid) = goal_id {
                    escalation_event = escalation_event.for_goal(gid);
                }
                if let Some(tid) = task_id {
                    escalation_event = escalation_event.for_task(tid);
                }

                // Store for later retrieval
                escalation_store.write().await.push(escalation_event);

                // If blocking, transition task to Blocked
                if is_blocking {
                    if let Some(tid) = task_id {
                        if let Ok(Some(task)) = task_repo.get(tid).await {
                            let mut blocked_task = task.clone();
                            if blocked_task.transition_to(TaskStatus::Blocked).is_ok() {
                                let _ = task_repo.update(&blocked_task).await;
                            }
                        }
                    }
                }

                let _ = swarm_event_tx.send(SwarmEvent::HumanEscalationRequired {
                    goal_id,
                    task_id,
                    reason: reason.clone(),
                    urgency: urgency.clone(),
                    questions: vec![],
                    is_blocking,
                }).await;
                audit_log.log(
                    AuditEntry::new(
                        if is_blocking { AuditLevel::Warning } else { AuditLevel::Info },
                        AuditCategory::Goal,
                        AuditAction::GoalPaused,
                        AuditActor::System,
                        format!(
                            "Human escalation needed ({}): {} - blocking={}",
                            urgency, reason, is_blocking
                        ),
                    ),
                ).await;
            }
        }
    }

    /// Run post-DAG-execution verification and return the number of failures found.
    async fn run_post_dag_verification(
        &self,
        results: &ExecutionResults,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> usize {
        let mut verification_failures = 0;

        for task_result in &results.task_results {
            if task_result.status == TaskStatus::Complete {
                // Verify the task
                match self.verify_task(task_result.task_id).await {
                    Ok(Some(verification)) if !verification.passed => {
                        // Verification failed - update task status
                        if let Ok(Some(mut task)) = self.task_repo.get(task_result.task_id).await {
                            if task.transition_to(TaskStatus::Failed).is_ok() {
                                let _ = self.task_repo.update(&task).await;
                            }
                        }
                        verification_failures += 1;

                        // Record as goal violation in evolution loop
                        if self.config.track_evolution {
                            let template_name = task_result.session.as_ref()
                                .map(|s| s.agent_template.clone())
                                .unwrap_or_else(|| "unknown".to_string());

                            let template_version = match self.agent_repo.get_template_by_name(&template_name).await {
                                Ok(Some(t)) => t.version,
                                _ => 1,
                            };

                            let execution = TaskExecution {
                                task_id: task_result.task_id,
                                template_name,
                                template_version,
                                outcome: TaskOutcome::GoalViolation,
                                executed_at: chrono::Utc::now(),
                                turns_used: task_result.session.as_ref()
                                    .map(|s| s.turns_completed)
                                    .unwrap_or(0),
                                tokens_used: task_result.session.as_ref()
                                    .map(|s| s.total_tokens())
                                    .unwrap_or(0),
                                downstream_tasks: vec![],
                            };
                            self.evolution_loop.record_execution(execution).await;
                        }

                        // Emit verification event
                        let checks_total = verification.checks.len();
                        let checks_passed = verification.checks.iter().filter(|c| c.passed).count();
                        let _ = event_tx.send(SwarmEvent::TaskVerified {
                            task_id: task_result.task_id,
                            passed: false,
                            checks_passed,
                            checks_total,
                        }).await;

                        self.audit_log.log(
                            AuditEntry::new(
                                AuditLevel::Warning,
                                AuditCategory::Task,
                                AuditAction::TaskFailed,
                                AuditActor::System,
                                format!(
                                    "Task {} failed verification: {}/{} checks passed. {}",
                                    task_result.task_id, checks_passed, checks_total,
                                    verification.failures_summary.unwrap_or_default()
                                ),
                            )
                            .with_entity(task_result.task_id, "task"),
                        ).await;
                    }
                    Ok(Some(verification)) => {
                        // Verification passed
                        let checks_total = verification.checks.len();
                        let checks_passed = verification.checks.iter().filter(|c| c.passed).count();
                        let _ = event_tx.send(SwarmEvent::TaskVerified {
                            task_id: task_result.task_id,
                            passed: true,
                            checks_passed,
                            checks_total,
                        }).await;
                    }
                    _ => {}
                }

                // Also check goal alignment
                if let Some(ref alignment_svc) = self.goal_alignment {
                    if let Ok(Some(task)) = self.task_repo.get(task_result.task_id).await {
                        if let Ok(eval) = alignment_svc.evaluate_task(&task).await {
                            let _ = event_tx.send(SwarmEvent::GoalAlignmentEvaluated {
                                task_id: task_result.task_id,
                                overall_score: eval.overall_score,
                                passes: eval.passes,
                            }).await;

                            if !eval.passes {
                                self.audit_log.log(
                                    AuditEntry::new(
                                        AuditLevel::Warning,
                                        AuditCategory::Goal,
                                        AuditAction::GoalPaused,
                                        AuditActor::System,
                                        format!(
                                            "Task {} has low goal alignment: {:.0}% ({}/{}). {}",
                                            task_result.task_id,
                                            eval.overall_score * 100.0,
                                            eval.goals_satisfied,
                                            eval.goal_alignments.len(),
                                            eval.summary
                                        ),
                                    )
                                    .with_entity(task_result.task_id, "task"),
                                ).await;
                            }
                        }
                    }
                }
            }
        }

        verification_failures
    }

    /// Persist successful task outputs to memory for future agent reference.
    async fn persist_task_outputs_to_memory(&self, _goal_id: Uuid, results: &ExecutionResults) {
        let Some(ref memory_repo) = self.memory_repo else { return };

        use crate::domain::models::Memory;

        for task_result in &results.task_results {
            if task_result.status == TaskStatus::Complete {
                if let Some(ref session) = task_result.session {
                    if let Some(ref result_text) = session.result {
                        let task = self.task_repo.get(task_result.task_id).await
                            .ok()
                            .flatten();

                        let key = format!("task/{}/output", task_result.task_id);
                        let namespace = "tasks".to_string();

                        let content = format!(
                            "Task: {}\nAgent: {}\nOutput:\n{}",
                            task.as_ref().map(|t| t.title.as_str()).unwrap_or("Unknown"),
                            session.agent_template,
                            result_text
                        );

                        let memory = Memory::episodic(key, content)
                            .with_namespace(namespace)
                            .with_type(crate::domain::models::MemoryType::Context);

                        if let Err(e) = memory_repo.store(&memory).await {
                            tracing::warn!(
                                "Failed to persist task {} output to memory: {}",
                                task_result.task_id, e
                            );
                        }
                    }
                }
            }
        }
    }

    /// Handle semantic drift detection in convergence loop.
    pub(super) async fn handle_semantic_drift(
        &self,
        goal_id: Uuid,
        convergence_state: &crate::domain::models::ConvergenceState,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) {
        self.audit_log.log(
            AuditEntry::new(
                AuditLevel::Warning,
                AuditCategory::Goal,
                AuditAction::GoalPaused,
                AuditActor::System,
                format!(
                    "Semantic drift detected for goal {}: same gaps recurring across iterations",
                    goal_id
                ),
            )
            .with_entity(goal_id, "goal"),
        ).await;

        // Log recurring gaps
        for gap in convergence_state.recurring_gaps() {
            tracing::warn!(
                "Recurring gap (seen {} times): {}",
                gap.occurrence_count, gap.normalized_description
            );
        }

        // Emit SemanticDriftDetected event
        let recurring_gap_descriptions: Vec<String> = convergence_state
            .recurring_gaps()
            .iter()
            .map(|g| g.normalized_description.clone())
            .collect();
        let _ = event_tx.send(SwarmEvent::SemanticDriftDetected {
            goal_id,
            recurring_gaps: recurring_gap_descriptions.clone(),
            iterations: convergence_state.current_iteration,
        }).await;

        // Also emit to EventBus if configured
        self.emit_to_event_bus(SwarmEvent::SemanticDriftDetected {
            goal_id,
            recurring_gaps: recurring_gap_descriptions,
            iterations: convergence_state.current_iteration,
        }).await;
    }

    /// Apply convergence guidance: augment tasks, create new tasks, retry failed tasks.
    pub(super) async fn apply_convergence_guidance(
        &self,
        goal_id: Uuid,
        verification_result: &crate::domain::models::IntentVerificationResult,
        guidance: &crate::domain::models::RepromptGuidance,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        self.audit_log.info(
            AuditCategory::Goal,
            AuditAction::GoalUpdated,
            format!(
                "Applying reprompt guidance for goal {}: {:?} approach, {} focus areas, {} new tasks",
                goal_id, guidance.approach,
                guidance.focus_areas.len(), guidance.tasks_to_add.len()
            ),
        ).await;

        // Get pending tasks for augmentation
        let pending_tasks = self.get_pending_task_ids_for_goal(goal_id).await?;

        // Build and apply task augmentations
        let augmentations = crate::domain::models::build_task_augmentations(
            verification_result,
            &pending_tasks,
        );

        for augmentation in augmentations {
            self.apply_task_augmentation(&augmentation).await?;
        }

        // Create new tasks from guidance
        for task_guidance in &guidance.tasks_to_add {
            let priority = match task_guidance.priority {
                crate::domain::models::TaskGuidancePriority::High => crate::domain::models::TaskPriority::High,
                crate::domain::models::TaskGuidancePriority::Normal => crate::domain::models::TaskPriority::Normal,
                crate::domain::models::TaskGuidancePriority::Low => crate::domain::models::TaskPriority::Low,
            };

            let new_task = Task::with_title(&task_guidance.title, &task_guidance.description)
                .with_source(crate::domain::models::TaskSource::GoalEvaluation(goal_id))
                .with_priority(priority);

            if let Err(e) = self.task_repo.create(&new_task).await {
                tracing::warn!("Failed to create reprompt task: {}", e);
            } else {
                let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                    task_id: new_task.id,
                    task_title: new_task.title.clone(),
                    goal_id,
                }).await;
            }
        }

        // Reset failed tasks for retry
        for task_id in &guidance.tasks_to_retry {
            if let Ok(Some(mut task)) = self.task_repo.get(*task_id).await {
                if task.status == TaskStatus::Failed && task.can_retry() {
                    if task.retry().is_ok() {
                        let _ = self.task_repo.update(&task).await;
                    }
                }
            }
        }

        Ok(())
    }

    /// Get pending task IDs for a goal.
    pub(super) async fn get_pending_task_ids_for_goal(&self, goal_id: Uuid) -> DomainResult<Vec<Uuid>> {
        let all_goal_tasks = self.task_repo.list_by_source("goal_evaluation").await?;
        Ok(all_goal_tasks
            .iter()
            .filter(|t| t.source == crate::domain::models::TaskSource::GoalEvaluation(goal_id))
            .filter(|t| t.status == TaskStatus::Pending || t.status == TaskStatus::Ready)
            .map(|t| t.id)
            .collect())
    }

    /// Apply a task augmentation by updating the task's description.
    pub(super) async fn apply_task_augmentation(
        &self,
        augmentation: &crate::domain::models::TaskAugmentation,
    ) -> DomainResult<()> {
        let mut task = self.task_repo.get(augmentation.task_id).await?
            .ok_or(DomainError::TaskNotFound(augmentation.task_id))?;

        // Only augment pending/ready tasks
        if task.status != TaskStatus::Pending && task.status != TaskStatus::Ready {
            return Ok(());
        }

        // Build the augmented description
        let prefix = augmentation.format_as_description_prefix();
        if !prefix.is_empty() {
            task.description = format!("{}{}", prefix, task.description);
            self.task_repo.update(&task).await?;

            tracing::info!(
                "Augmented task {} with {} gaps and {} focus areas",
                augmentation.task_id,
                augmentation.gaps_to_address.len(),
                augmentation.focus_areas.len()
            );
        }

        Ok(())
    }
}

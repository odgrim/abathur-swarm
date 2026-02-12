//! Specialist trigger subsystem for the swarm orchestrator.
//!
//! Handles DAG restructuring for failed tasks, diagnostic analyst spawning,
//! merge conflict specialists, spawn limit evaluation, and overmind integration.

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Task, TaskPriority, TaskSource, TaskStatus};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    IntegrationVerifierService, MergeQueue, MergeQueueConfig, VerifierConfig,
    command_bus::{CommandEnvelope, CommandSource, DomainCommand, TaskCommand},
    dag_restructure::{RestructureContext, RestructureDecision, RestructureTrigger, TaskPriorityModifier},
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
    /// Process specialist agent triggers.
    ///
    /// Checks for conditions that should spawn specialist agents:
    /// - DAG restructuring for recoverable failures -> New decomposition/alternative path
    /// - Merge conflicts -> Merge Conflict Specialist
    /// - Persistent failures (max retries exceeded, restructuring exhausted) -> Diagnostic Analyst
    pub(super) async fn process_specialist_triggers(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        // Check for persistent failures that need restructuring or diagnostic analysis
        let failed_tasks = self.task_repo.list_by_status(TaskStatus::Failed).await?;
        let permanently_failed: Vec<_> = failed_tasks
            .iter()
            .filter(|t| t.retry_count >= self.config.max_task_retries)
            .collect();

        for task in permanently_failed {
            // First, try DAG restructuring before falling back to diagnostic analyst
            let restructure_result = self.try_restructure_for_failure(task, event_tx).await;

            match restructure_result {
                Ok(true) => {
                    // Restructuring created new tasks
                    continue;
                }
                Ok(false) => {
                    // Restructuring not possible, fall through to diagnostic
                }
                Err(e) => {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Task,
                            AuditAction::TaskFailed,
                            AuditActor::System,
                            format!("Restructure attempt failed for task {}: {}", task.id, e),
                        )
                        .with_entity(task.id, "task"),
                    ).await;
                }
            }

            // Check if we haven't already created a diagnostic task
            let id_prefix = &task.id.to_string()[..8];
            let diagnostic_exists = self.task_repo
                .list_by_status(TaskStatus::Ready)
                .await?
                .iter()
                .chain(self.task_repo.list_by_status(TaskStatus::Pending).await?.iter())
                .chain(self.task_repo.list_by_status(TaskStatus::Running).await?.iter())
                .any(|t| t.title.contains("Diagnostic:") && t.title.contains(id_prefix));

            if !diagnostic_exists {
                if let Err(e) = self.spawn_specialist_for_failure(task, event_tx).await {
                    self.audit_log.log(
                        AuditEntry::new(
                            AuditLevel::Warning,
                            AuditCategory::Agent,
                            AuditAction::AgentSpawned,
                            AuditActor::System,
                            format!("Failed to spawn diagnostic specialist for task {}: {}", task.id, e),
                        )
                        .with_entity(task.id, "task"),
                    ).await;
                }
            }
        }

        // Check for merge conflicts needing specialist resolution
        if self.config.use_merge_queue {
            if let Err(e) = self.process_merge_conflict_specialists(event_tx).await {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        AuditActor::System,
                        format!("Failed to process merge conflict specialists: {}", e),
                    ),
                ).await;
            }
        }

        Ok(())
    }

    /// Try to restructure the DAG for a permanently failed task.
    /// Returns Ok(true) if restructuring was applied, Ok(false) if not possible/exhausted.
    async fn try_restructure_for_failure(
        &self,
        failed_task: &Task,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<bool> {
        let trigger = RestructureTrigger::PermanentFailure {
            task_id: failed_task.id,
            retries_exhausted: failed_task.retry_count,
        };

        // Check if restructuring should be attempted
        let mut restructure_svc = self.restructure_service.lock().await;

        if !restructure_svc.should_restructure(&trigger) {
            return Ok(false);
        }

        // Get related failures
        let all_failed = self.task_repo.list_by_status(TaskStatus::Failed).await?;
        let related_failures: Vec<Task> = all_failed
            .into_iter()
            .filter(|t| t.id != failed_task.id)
            .collect();

        // Build restructure context
        let context = RestructureContext {
            goal: None,
            failed_task: failed_task.clone(),
            failure_reason: format!("Task failed after {} retries", failed_task.retry_count),
            previous_attempts: vec![],
            related_failures,
            available_approaches: vec![],
            attempt_number: restructure_svc.attempt_count(failed_task.id) + 1,
            time_since_last: None,
        };

        // Get restructure decision
        let decision = restructure_svc.analyze_and_decide(&context).await?;

        // Log the decision
        self.audit_log.info(
            AuditCategory::Task,
            AuditAction::TaskCreated,
            format!(
                "DAG restructure decision for task {}: {:?}",
                failed_task.id, decision
            ),
        ).await;

        // Emit event via EventBus (journaled)
        self.event_bus.publish(crate::services::event_factory::make_event(
            crate::services::event_bus::EventSeverity::Warning,
            crate::services::event_bus::EventCategory::Execution,
            None,
            Some(failed_task.id),
            crate::services::event_bus::EventPayload::RestructureTriggered {
                task_id: failed_task.id,
                decision: format!("{:?}", decision),
            },
        )).await;
        // (Bridge forwards EventBus→event_tx automatically)

        // Apply the decision
        match decision {
            RestructureDecision::RetryDifferentApproach { new_approach, new_agent_type } => {
                // Update description directly (no command for description updates)
                let mut updated_task = failed_task.clone();
                updated_task.description = format!(
                    "{}\n\n## Restructure Note\nPrevious approach failed. Try: {}",
                    updated_task.description, new_approach
                );
                if let Some(agent_type) = new_agent_type {
                    updated_task.agent_type = Some(agent_type);
                }
                updated_task.retry_count = 0;
                self.task_repo.update(&updated_task).await?;

                // Emit description update event via EventBus
                self.event_bus.publish(crate::services::event_factory::task_event(
                    crate::services::event_bus::EventSeverity::Info,
                    None,
                    failed_task.id,
                    crate::services::event_bus::EventPayload::TaskDescriptionUpdated {
                        task_id: failed_task.id,
                        reason: format!("DAG restructure: retry with different approach — {}", new_approach),
                    },
                )).await;

                // Transition to Ready via CommandBus
                if let Some(cb) = self.command_bus.read().await.as_ref() {
                    let envelope = CommandEnvelope::new(
                        CommandSource::System,
                        DomainCommand::Task(TaskCommand::Transition {
                            task_id: failed_task.id,
                            new_status: TaskStatus::Ready,
                        }),
                    );
                    let _ = cb.dispatch(envelope).await;
                }
                Ok(true)
            }
            RestructureDecision::DecomposeDifferently { new_subtasks, remove_original } => {
                self.create_restructure_subtasks(failed_task, &new_subtasks, remove_original, event_tx).await?;
                Ok(true)
            }
            RestructureDecision::AlternativePath { description, new_tasks } => {
                self.create_restructure_subtasks(failed_task, &new_tasks, false, event_tx).await?;

                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskCreated,
                    format!("Created alternative path: {}", description),
                ).await;

                Ok(true)
            }
            RestructureDecision::WaitAndRetry { delay, reason } => {
                self.audit_log.info(
                    AuditCategory::Task,
                    AuditAction::TaskFailed,
                    format!("Restructure suggests waiting {} seconds: {}", delay.as_secs(), reason),
                ).await;
                Ok(false)
            }
            RestructureDecision::Escalate { reason, context } => {
                // Try federation delegation before falling through
                if let Some(ref federation) = self.federation_client {
                    let peers = federation.list_available_peers();
                    for peer in peers {
                        let task_desc = format!(
                            "Delegated task: {}\nReason: {}\nContext: {}",
                            failed_task.title, reason, context
                        );
                        match federation.delegate_task(&peer.id, &task_desc).await {
                            Ok(a2a_task) => {
                                self.audit_log.info(
                                    AuditCategory::Task,
                                    AuditAction::TaskCompleted,
                                    format!(
                                        "Task {} delegated to peer '{}' as A2A task {}",
                                        failed_task.id, peer.name, a2a_task.id
                                    ),
                                ).await;
                                return Ok(true);
                            }
                            Err(e) => {
                                tracing::warn!("Federation delegation to '{}' failed: {}", peer.name, e);
                            }
                        }
                    }
                }

                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Warning,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} escalated: {} - {}", failed_task.id, reason, context),
                    )
                    .with_entity(failed_task.id, "task"),
                ).await;
                Ok(false)
            }
            RestructureDecision::AcceptFailure { reason } => {
                self.audit_log.log(
                    AuditEntry::new(
                        AuditLevel::Error,
                        AuditCategory::Task,
                        AuditAction::TaskFailed,
                        AuditActor::System,
                        format!("Task {} failure accepted: {}", failed_task.id, reason),
                    )
                    .with_entity(failed_task.id, "task"),
                ).await;
                Ok(false)
            }
        }
    }

    /// Create subtasks from a restructure decision (shared between DecomposeDifferently and AlternativePath).
    async fn create_restructure_subtasks(
        &self,
        failed_task: &Task,
        new_tasks: &[crate::services::dag_restructure::NewTaskSpec],
        remove_original: bool,
        _event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let cb = self.command_bus.read().await.clone();

        let mut title_to_id: Vec<(String, Uuid)> = Vec::new();
        for spec in new_tasks {
            let priority = match spec.priority {
                TaskPriorityModifier::Same => failed_task.priority.clone(),
                TaskPriorityModifier::Higher => TaskPriority::High,
                TaskPriorityModifier::Lower => TaskPriority::Low,
            };

            // Resolve depends_on titles to UUIDs from already-created tasks
            let mut depends_on = Vec::new();
            for dep_title in &spec.depends_on {
                if let Some((_, dep_id)) = title_to_id.iter().find(|(t, _)| t == dep_title) {
                    depends_on.push(*dep_id);
                }
            }

            // Submit task via CommandBus (journals TaskSubmitted event)
            if let Some(ref cb) = cb {
                let envelope = CommandEnvelope::new(
                    CommandSource::System,
                    DomainCommand::Task(TaskCommand::Submit {
                        title: Some(spec.title.clone()),
                        description: spec.description.clone(),
                        parent_id: None,
                        priority: priority.clone(),
                        agent_type: spec.agent_type.clone(),
                        depends_on: depends_on.clone(),
                        context: Box::new(None),
                        idempotency_key: None,
                        source: TaskSource::System,
                        deadline: None,
                    }),
                );
                match cb.dispatch(envelope).await {
                    Ok(crate::services::command_bus::CommandResult::Task(task)) => {
                        title_to_id.push((spec.title.clone(), task.id));
                    }
                    Ok(other) => {
                        tracing::warn!(
                            "CommandBus returned unexpected result type for restructure subtask '{}': {:?}",
                            spec.title, other
                        );
                    }
                    Err(e) => {
                        tracing::warn!("CommandBus submit failed for restructure subtask '{}': {}", spec.title, e);
                    }
                }
            } else {
                tracing::warn!("CommandBus not available — cannot create restructure subtask '{}'", spec.title);
            }
        }

        // Cancel the original task via CommandBus
        if remove_original {
            if let Some(ref cb) = cb {
                let envelope = CommandEnvelope::new(
                    CommandSource::System,
                    DomainCommand::Task(TaskCommand::Cancel {
                        task_id: failed_task.id,
                        reason: "Replaced by restructure subtasks".to_string(),
                    }),
                );
                if let Err(e) = cb.dispatch(envelope).await {
                    tracing::warn!("CommandBus cancel failed for task {}: {}", failed_task.id, e);
                }
            } else {
                tracing::warn!("CommandBus not available — cannot cancel task {}", failed_task.id);
            }
        }

        Ok(())
    }

    /// Spawn a diagnostic analyst for a permanently failed task.
    async fn spawn_specialist_for_failure(
        &self,
        failed_task: &Task,
        _event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let title = format!("Diagnostic: Investigate failure of task {}", &failed_task.id.to_string()[..8]);
        let description = format!(
            "The following task has permanently failed after {} retries:\n\n\
            Title: {}\n\
            Description: {}\n\n\
            Please investigate the root cause of failure and suggest remediation.\n\
            Consider:\n\
            - Are the task requirements achievable?\n\
            - Are there missing dependencies or prerequisites?\n\
            - Is the agent type appropriate for this task?\n\
            - Are there external blockers (permissions, resources, etc.)?",
            failed_task.retry_count,
            failed_task.title,
            failed_task.description
        );

        // Submit diagnostic task via CommandBus (journals TaskSubmitted event)
        let diagnostic_task_id = if let Some(cb) = self.command_bus.read().await.as_ref() {
            let envelope = CommandEnvelope::new(
                CommandSource::System,
                DomainCommand::Task(TaskCommand::Submit {
                    title: Some(title.clone()),
                    description: description.clone(),
                    parent_id: None,
                    priority: TaskPriority::Normal,
                    agent_type: Some("diagnostic-analyst".to_string()),
                    depends_on: vec![],
                    context: Box::new(None),
                    idempotency_key: None,
                    source: TaskSource::System,
                    deadline: None,
                }),
            );
            match cb.dispatch(envelope).await {
                Ok(crate::services::command_bus::CommandResult::Task(task)) => task.id,
                Ok(other) => {
                    tracing::warn!("CommandBus returned unexpected result for diagnostic task: {:?}", other);
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("CommandBus submit failed for diagnostic task: {}", e);
                    return Ok(());
                }
            }
        } else {
            tracing::warn!("CommandBus not available — cannot create diagnostic task for {}", failed_task.id);
            return Ok(());
        };

        // Publish specialist spawned event via EventBus
        self.event_bus.publish(crate::services::event_factory::agent_event(
            crate::services::event_bus::EventSeverity::Info,
            Some(diagnostic_task_id),
            crate::services::event_bus::EventPayload::SpecialistSpawned {
                specialist_type: "diagnostic-analyst".to_string(),
                trigger: format!("Task {} permanently failed", failed_task.id),
                task_id: Some(diagnostic_task_id),
            },
        )).await;
        // (Bridge forwards EventBus→event_tx automatically)

        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Spawned Diagnostic Analyst for permanently failed task {}",
                failed_task.id
            ),
        ).await;

        Ok(())
    }

    /// Check spawn limits for a parent task and trigger limit evaluation specialist if exceeded.
    ///
    /// Returns Ok(true) if task creation should proceed, Ok(false) if limits exceeded.
    pub async fn check_spawn_limits_and_handle(
        &self,
        parent_id: Option<Uuid>,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<bool> {
        use crate::services::task_service::SpawnLimitConfig;

        let Some(parent_id) = parent_id else {
            return Ok(true);
        };

        let parent_task = match self.task_repo.get(parent_id).await? {
            Some(t) => t,
            None => return Ok(true),
        };

        let spawn_limits = SpawnLimitConfig {
            max_subtask_depth: self.config.spawn_limits.max_subtask_depth,
            max_subtasks_per_task: self.config.spawn_limits.max_subtasks_per_task,
            max_total_descendants: self.config.spawn_limits.max_total_descendants,
            allow_limit_extensions: self.config.spawn_limits.allow_limit_extensions,
        };

        // Check subtask depth by traversing up the tree
        let mut depth = 0u32;
        let mut current = parent_task.clone();
        while let Some(pid) = current.parent_id {
            depth += 1;
            if depth > 100 { break; }
            match self.task_repo.get(pid).await? {
                Some(p) => current = p,
                None => break,
            }
        }

        if depth >= spawn_limits.max_subtask_depth {
            if spawn_limits.allow_limit_extensions {
                let id_prefix = &parent_task.id.to_string()[..8];
                let specialist_exists = self.task_repo
                    .list_by_status(TaskStatus::Ready)
                    .await?
                    .iter()
                    .chain(self.task_repo.list_by_status(TaskStatus::Pending).await?.iter())
                    .chain(self.task_repo.list_by_status(TaskStatus::Running).await?.iter())
                    .any(|t| t.title.contains("Limit Evaluation:") && t.title.contains(id_prefix));

                if !specialist_exists {
                    self.spawn_limit_evaluation_specialist(
                        &parent_task,
                        "subtask_depth",
                        depth,
                        spawn_limits.max_subtask_depth,
                        event_tx,
                    ).await?;
                }
            }
            return Ok(false);
        }

        // Check direct subtasks count
        let filter = crate::domain::ports::TaskFilter {
            parent_id: Some(parent_id),
            ..Default::default()
        };
        let direct_subtasks = self.task_repo.list(filter).await?.len() as u32;

        if direct_subtasks >= spawn_limits.max_subtasks_per_task {
            if spawn_limits.allow_limit_extensions {
                let id_prefix = &parent_task.id.to_string()[..8];
                let specialist_exists = self.task_repo
                    .list_by_status(TaskStatus::Ready)
                    .await?
                    .iter()
                    .chain(self.task_repo.list_by_status(TaskStatus::Pending).await?.iter())
                    .chain(self.task_repo.list_by_status(TaskStatus::Running).await?.iter())
                    .any(|t| t.title.contains("Limit Evaluation:") && t.title.contains(id_prefix));

                if !specialist_exists {
                    self.spawn_limit_evaluation_specialist(
                        &parent_task,
                        "subtasks_per_task",
                        direct_subtasks,
                        spawn_limits.max_subtasks_per_task,
                        event_tx,
                    ).await?;
                }
            }
            return Ok(false);
        }

        Ok(true)
    }

    /// Spawn a limit evaluation specialist when spawn limits are exceeded.
    async fn spawn_limit_evaluation_specialist(
        &self,
        parent_task: &Task,
        limit_type: &str,
        current_value: u32,
        limit_value: u32,
        _event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let title = format!("Limit Evaluation: {} exceeded for task {}", limit_type, &parent_task.id.to_string()[..8]);
        let description = format!(
            "Spawn limit exceeded while creating subtasks:\n\n\
            Parent Task: {}\n\
            Limit Type: {}\n\
            Current Value: {}\n\
            Limit Value: {}\n\n\
            Please evaluate whether:\n\
            1. An extension should be granted (the decomposition is genuinely necessary)\n\
            2. The task should be restructured (different approach needed)\n\
            3. The agent is inefficient (template refinement needed)\n\n\
            Your decision should include:\n\
            - GRANT_EXTENSION: Allow additional subtasks with a new limit\n\
            - RESTRUCTURE: Recommend a different decomposition approach\n\
            - REJECT: Task tree is too complex, simplification required",
            parent_task.title,
            limit_type,
            current_value,
            limit_value
        );

        // Submit evaluation task via CommandBus
        let eval_task_id = if let Some(cb) = self.command_bus.read().await.as_ref() {
            let envelope = CommandEnvelope::new(
                CommandSource::System,
                DomainCommand::Task(TaskCommand::Submit {
                    title: Some(title.clone()),
                    description: description.clone(),
                    parent_id: None,
                    priority: TaskPriority::Normal,
                    agent_type: Some("limit-evaluation-specialist".to_string()),
                    depends_on: vec![],
                    context: Box::new(None),
                    idempotency_key: None,
                    source: TaskSource::System,
                    deadline: None,
                }),
            );
            match cb.dispatch(envelope).await {
                Ok(crate::services::command_bus::CommandResult::Task(task)) => task.id,
                Ok(other) => {
                    tracing::warn!("CommandBus returned unexpected result for limit evaluation task: {:?}", other);
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("CommandBus submit failed for limit evaluation task: {}", e);
                    return Ok(());
                }
            }
        } else {
            tracing::warn!("CommandBus not available — cannot create limit evaluation task for {}", parent_task.id);
            return Ok(());
        };

        // Publish events via EventBus
        self.event_bus.publish(crate::services::event_factory::agent_event(
            crate::services::event_bus::EventSeverity::Info,
            Some(eval_task_id),
            crate::services::event_bus::EventPayload::SpecialistSpawned {
                specialist_type: "limit-evaluation-specialist".to_string(),
                trigger: format!("{} limit exceeded ({}/{})", limit_type, current_value, limit_value),
                task_id: Some(eval_task_id),
            },
        )).await;

        self.event_bus.publish(crate::services::event_factory::agent_event(
            crate::services::event_bus::EventSeverity::Warning,
            Some(parent_task.id),
            crate::services::event_bus::EventPayload::SpawnLimitExceeded {
                parent_task_id: parent_task.id,
                limit_type: limit_type.to_string(),
                current_value,
                limit_value,
            },
        )).await;

        // (Bridge forwards EventBus→event_tx automatically)

        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned,
            format!(
                "Spawned Limit Evaluation Specialist for task {} ({} limit: {}/{})",
                parent_task.id, limit_type, current_value, limit_value
            ),
        ).await;

        Ok(())
    }

    /// Process merge conflicts and spawn conflict resolution specialists.
    async fn process_merge_conflict_specialists(
        &self,
        _event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let verifier = IntegrationVerifierService::new(
            self.task_repo.clone(),
            self.goal_repo.clone(),
            self.worktree_repo.clone(),
            VerifierConfig::default(),
        );

        let merge_config = MergeQueueConfig {
            repo_path: self.config.repo_path.to_str().unwrap_or(".").to_string(),
            main_branch: self.config.default_base_ref.clone(),
            require_verification: self.config.verify_on_completion,
            route_conflicts_to_specialist: true,
            ..Default::default()
        };

        let merge_queue = MergeQueue::new(
            self.task_repo.clone(),
            self.worktree_repo.clone(),
            Arc::new(verifier),
            merge_config,
        );

        let conflicts = merge_queue.get_conflicts_needing_resolution().await;

        for conflict in conflicts {
            // Check if a resolution task already exists for this conflict
            let resolution_exists = self.task_repo
                .list_by_status(TaskStatus::Ready)
                .await
                .map(|tasks| {
                    tasks.iter().any(|t| {
                        t.title.contains("Resolve merge conflict") &&
                        t.title.contains(&conflict.source_branch)
                    })
                })
                .unwrap_or(false)
                || self.task_repo
                    .list_by_status(TaskStatus::Running)
                    .await
                    .map(|tasks| {
                        tasks.iter().any(|t| {
                            t.title.contains("Resolve merge conflict") &&
                            t.title.contains(&conflict.source_branch)
                        })
                    })
                    .unwrap_or(false)
                || self.task_repo
                    .list_by_status(TaskStatus::Pending)
                    .await
                    .map(|tasks| {
                        tasks.iter().any(|t| {
                            t.title.contains("Resolve merge conflict") &&
                            t.title.contains(&conflict.source_branch)
                        })
                    })
                    .unwrap_or(false);

            if !resolution_exists {
                // Determine parent_id and context for the specialist task.
                // If the conflict's task has a parent, this is a feature branch merge-back.
                let (specialist_parent_id, conflict_context) = {
                    let conflict_task = self.task_repo.get(conflict.task_id).await.ok().flatten();
                    if let Some(ref ct) = conflict_task {
                        if ct.parent_id.is_some() {
                            let root_id = self.find_root_ancestor(conflict.task_id).await;
                            let mut custom = std::collections::HashMap::new();
                            custom.insert(
                                "feature_branch_conflict".to_string(),
                                serde_json::json!(true),
                            );
                            custom.insert(
                                "original_subtask_id".to_string(),
                                serde_json::json!(conflict.task_id.to_string()),
                            );
                            custom.insert(
                                "merge_request_id".to_string(),
                                serde_json::json!(conflict.merge_request_id.to_string()),
                            );
                            let ctx = crate::domain::models::TaskContext {
                                input: String::new(),
                                hints: vec![],
                                relevant_files: conflict.conflict_files.clone(),
                                custom,
                            };
                            (Some(root_id), Some(ctx))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    }
                };

                let title = format!("Resolve merge conflict: {} → {}", conflict.source_branch, conflict.target_branch);
                let description = format!(
                    "A merge conflict was detected when trying to merge branch '{}' into '{}'.\n\n\
                    Conflicting files:\n{}\n\n\
                    Working directory: {}\n\n\
                    Please resolve the conflicts by:\n\
                    1. Analyzing the conflicting changes\n\
                    2. Understanding the intent of each change\n\
                    3. Merging the changes in a way that preserves both intents\n\
                    4. Testing the merged result\n\
                    5. Completing the merge commit",
                    conflict.source_branch,
                    conflict.target_branch,
                    conflict.conflict_files.iter()
                        .map(|f| format!("  - {}", f))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    conflict.workdir
                );

                // Submit via CommandBus
                let task_id = if let Some(cb) = self.command_bus.read().await.as_ref() {
                    let envelope = CommandEnvelope::new(
                        CommandSource::System,
                        DomainCommand::Task(TaskCommand::Submit {
                            title: Some(title.clone()),
                            description: description.clone(),
                            parent_id: specialist_parent_id,
                            priority: TaskPriority::High,
                            agent_type: Some("merge-conflict-specialist".to_string()),
                            depends_on: vec![],
                            context: Box::new(conflict_context),
                            idempotency_key: None,
                            source: TaskSource::System,
                            deadline: None,
                        }),
                    );
                    match cb.dispatch(envelope).await {
                        Ok(crate::services::command_bus::CommandResult::Task(task)) => Some(task.id),
                        Ok(other) => {
                            tracing::warn!("CommandBus returned unexpected result for merge conflict task: {:?}", other);
                            None
                        }
                        Err(e) => {
                            tracing::warn!("CommandBus submit failed for merge conflict task: {}", e);
                            None
                        }
                    }
                } else {
                    tracing::warn!("CommandBus not available — cannot create merge conflict specialist task");
                    None
                };

                if let Some(task_id) = task_id {
                    // Publish via EventBus
                    self.event_bus.publish(crate::services::event_factory::agent_event(
                        crate::services::event_bus::EventSeverity::Info,
                        Some(task_id),
                        crate::services::event_bus::EventPayload::SpecialistSpawned {
                            specialist_type: "merge-conflict-specialist".to_string(),
                            trigger: format!("Merge conflict in {} files", conflict.conflict_files.len()),
                            task_id: Some(task_id),
                        },
                    )).await;
                    // (Bridge forwards EventBus→event_tx automatically)

                    self.audit_log.info(
                        AuditCategory::Agent,
                        AuditAction::AgentSpawned,
                        format!(
                            "Spawned Merge Conflict Specialist for {} → {}",
                            conflict.source_branch, conflict.target_branch
                        ),
                    ).await;
                }
            }
        }

        Ok(())
    }

    /// Resolve a conflict using the Overmind for intelligent decision-making.
    ///
    /// Falls back to priority-based heuristics if Overmind is not available.
    pub async fn resolve_conflict_with_overmind(
        &self,
        conflict_type: crate::domain::models::overmind::ConflictType,
        parties: Vec<crate::domain::models::overmind::ConflictParty>,
        context: &str,
    ) -> DomainResult<crate::domain::models::overmind::ConflictResolutionDecision> {
        use crate::domain::models::overmind::{
            ConflictResolutionRequest, ConflictResolutionDecision, ConflictResolutionApproach,
            DecisionMetadata,
        };

        let fallback_winner = parties.first().map(|p| p.id);

        if let Some(ref overmind) = self.overmind {
            let request = ConflictResolutionRequest {
                conflict_type,
                parties,
                context: context.to_string(),
                previous_attempts: vec![],
            };

            match overmind.resolve_conflict(request).await {
                Ok(decision) => {
                    self.audit_log.info(
                        AuditCategory::System,
                        AuditAction::TaskCompleted,
                        format!(
                            "Overmind resolved conflict with approach: {:?} (confidence: {:.2})",
                            decision.approach, decision.metadata.confidence
                        ),
                    ).await;
                    return Ok(decision);
                }
                Err(e) => {
                    tracing::warn!("Overmind conflict resolution failed, using fallback: {}", e);
                }
            }
        }

        Ok(ConflictResolutionDecision {
            metadata: DecisionMetadata::new(
                0.5,
                "Fallback: priority-based resolution (Overmind unavailable)",
            ),
            approach: match fallback_winner {
                Some(w) => ConflictResolutionApproach::PriorityBased { winner: w },
                None => ConflictResolutionApproach::Escalate,
            },
            task_modifications: vec![],
            notifications: vec!["Conflict resolved using priority-based fallback".to_string()],
        })
    }

    /// Evaluate whether to escalate to human using the Overmind.
    ///
    /// Falls back to conservative escalation if Overmind is not available.
    pub async fn evaluate_escalation_with_overmind(
        &self,
        context: crate::domain::models::overmind::EscalationContext,
        trigger: crate::domain::models::overmind::EscalationTrigger,
    ) -> DomainResult<crate::domain::models::overmind::OvermindEscalationDecision> {
        use crate::domain::models::overmind::{
            EscalationRequest, OvermindEscalationDecision, EscalationPreferences, OvermindEscalationUrgency,
            DecisionMetadata,
        };

        if let Some(ref overmind) = self.overmind {
            let request = EscalationRequest {
                context: context.clone(),
                trigger: trigger.clone(),
                previous_escalations: vec![],
                escalation_preferences: EscalationPreferences::default(),
            };

            match overmind.evaluate_escalation(request).await {
                Ok(decision) => {
                    self.audit_log.info(
                        AuditCategory::System,
                        AuditAction::TaskCompleted,
                        format!(
                            "Overmind escalation decision: should_escalate={} (confidence: {:.2})",
                            decision.should_escalate, decision.metadata.confidence
                        ),
                    ).await;
                    return Ok(decision);
                }
                Err(e) => {
                    tracing::warn!("Overmind escalation evaluation failed, using fallback: {}", e);
                }
            }
        }

        Ok(OvermindEscalationDecision {
            metadata: DecisionMetadata::new(
                0.6,
                "Fallback: conservative escalation (Overmind unavailable)",
            ),
            should_escalate: true,
            urgency: Some(OvermindEscalationUrgency::Medium),
            questions: vec![context.situation.clone()],
            context_for_human: format!(
                "Trigger: {:?}\nSituation: {}\nAttempts: {:?}",
                trigger, context.situation, context.attempts_made
            ),
            alternatives_if_unavailable: vec!["Wait and retry later".to_string()],
            is_blocking: true,
        })
    }
}

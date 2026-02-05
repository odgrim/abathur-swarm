//! Specialist trigger subsystem for the swarm orchestrator.
//!
//! Handles DAG restructuring for failed tasks, diagnostic analyst spawning,
//! merge conflict specialists, spawn limit evaluation, and overmind integration.

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, Task, TaskStatus};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditLevel,
    IntegrationVerifierService, MergeQueue, MergeQueueConfig, VerifierConfig,
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
            // Skip tasks without a goal_id
            let Some(goal_id) = task.goal_id else {
                continue;
            };

            // Get the goal for context
            let goal = match self.goal_repo.get(goal_id).await? {
                Some(g) => g,
                None => continue,
            };

            // First, try DAG restructuring before falling back to diagnostic analyst
            let restructure_result = self.try_restructure_for_failure(task, &goal, event_tx).await;

            match restructure_result {
                Ok(true) => {
                    // Restructuring created new tasks - reactivate the goal
                    let mut updated_goal = goal.clone();
                    updated_goal.resume();
                    let _ = self.goal_repo.update(&updated_goal).await;
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
            let diagnostic_exists = self.task_repo
                .list_by_goal(goal_id)
                .await?
                .iter()
                .any(|t| t.title.contains("Diagnostic:") && t.title.contains(&task.id.to_string()[..8]));

            if !diagnostic_exists {
                if let Err(e) = self.spawn_specialist_for_failure(task, goal_id, event_tx).await {
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
        goal: &Goal,
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

        // Get related failures in the same goal
        let goal_tasks = self.task_repo.list_by_goal(goal.id).await?;
        let related_failures: Vec<Task> = goal_tasks
            .into_iter()
            .filter(|t| t.status == TaskStatus::Failed && t.id != failed_task.id)
            .collect();

        // Build restructure context
        let context = RestructureContext {
            goal: goal.clone(),
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

        // Emit event
        let _ = event_tx.send(SwarmEvent::RestructureTriggered {
            task_id: failed_task.id,
            decision: format!("{:?}", decision),
        }).await;

        // Apply the decision
        match decision {
            RestructureDecision::RetryDifferentApproach { new_approach, new_agent_type } => {
                let mut updated_task = failed_task.clone();
                updated_task.description = format!(
                    "{}\n\n## Restructure Note\nPrevious approach failed. Try: {}",
                    updated_task.description, new_approach
                );
                if let Some(agent_type) = new_agent_type {
                    updated_task.agent_type = Some(agent_type);
                }
                updated_task.retry_count = 0;
                let _ = updated_task.transition_to(TaskStatus::Ready);
                self.task_repo.update(&updated_task).await?;
                Ok(true)
            }
            RestructureDecision::DecomposeDifferently { new_subtasks, remove_original } => {
                self.create_restructure_subtasks(goal, failed_task, &new_subtasks, remove_original, event_tx).await?;
                Ok(true)
            }
            RestructureDecision::AlternativePath { description, new_tasks } => {
                self.create_restructure_subtasks(goal, failed_task, &new_tasks, false, event_tx).await?;

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
        goal: &Goal,
        failed_task: &Task,
        new_tasks: &[crate::services::dag_restructure::NewTaskSpec],
        remove_original: bool,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let mut title_to_id: Vec<(String, Uuid)> = Vec::new();
        for spec in new_tasks {
            let priority = match spec.priority {
                TaskPriorityModifier::Same => failed_task.priority.clone(),
                TaskPriorityModifier::Higher => crate::domain::models::TaskPriority::High,
                TaskPriorityModifier::Lower => crate::domain::models::TaskPriority::Low,
            };

            let mut new_task = Task::new(&spec.title, &spec.description)
                .with_goal(goal.id)
                .with_priority(priority);

            if let Some(ref agent_type) = spec.agent_type {
                new_task = new_task.with_agent(agent_type);
            }

            // Resolve depends_on titles to UUIDs from already-created tasks
            for dep_title in &spec.depends_on {
                if let Some((_, dep_id)) = title_to_id.iter().find(|(t, _)| t == dep_title) {
                    new_task = new_task.with_dependency(*dep_id);
                }
            }

            if new_task.validate().is_ok() {
                title_to_id.push((spec.title.clone(), new_task.id));
                self.task_repo.create(&new_task).await?;
                let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                    task_id: new_task.id,
                    task_title: new_task.title.clone(),
                    goal_id: goal.id,
                }).await;
            }
        }

        // Cancel the original task if specified
        if remove_original {
            let mut canceled_task = failed_task.clone();
            let _ = canceled_task.transition_to(TaskStatus::Canceled);
            self.task_repo.update(&canceled_task).await?;
        }

        Ok(())
    }

    /// Spawn a diagnostic analyst for a permanently failed task.
    async fn spawn_specialist_for_failure(
        &self,
        failed_task: &Task,
        goal_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let diagnostic_task = Task::new(
            &format!("Diagnostic: Investigate failure of task {}", &failed_task.id.to_string()[..8]),
            &format!(
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
            ),
        )
        .with_goal(goal_id)
        .with_agent("diagnostic-analyst");

        diagnostic_task.validate().map_err(crate::domain::errors::DomainError::ValidationFailed)?;
        self.task_repo.create(&diagnostic_task).await?;

        let _ = event_tx.send(SwarmEvent::SpecialistSpawned {
            specialist_type: "diagnostic-analyst".to_string(),
            trigger: format!("Task {} permanently failed", failed_task.id),
            task_id: Some(diagnostic_task.id),
        }).await;

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
        goal_id: Uuid,
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
                let specialist_exists = self.task_repo
                    .list_by_goal(goal_id)
                    .await?
                    .iter()
                    .any(|t| t.title.contains("Limit Evaluation:") && t.title.contains(&parent_task.id.to_string()[..8]));

                if !specialist_exists {
                    self.spawn_limit_evaluation_specialist(
                        &parent_task,
                        "subtask_depth",
                        depth,
                        spawn_limits.max_subtask_depth,
                        goal_id,
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
                let specialist_exists = self.task_repo
                    .list_by_goal(goal_id)
                    .await?
                    .iter()
                    .any(|t| t.title.contains("Limit Evaluation:") && t.title.contains(&parent_task.id.to_string()[..8]));

                if !specialist_exists {
                    self.spawn_limit_evaluation_specialist(
                        &parent_task,
                        "subtasks_per_task",
                        direct_subtasks,
                        spawn_limits.max_subtasks_per_task,
                        goal_id,
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
        goal_id: Uuid,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let evaluation_task = Task::new(
            &format!("Limit Evaluation: {} exceeded for task {}", limit_type, &parent_task.id.to_string()[..8]),
            &format!(
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
            ),
        )
        .with_goal(goal_id)
        .with_agent("limit-evaluation-specialist");

        evaluation_task.validate().map_err(crate::domain::errors::DomainError::ValidationFailed)?;
        self.task_repo.create(&evaluation_task).await?;

        let _ = event_tx.send(SwarmEvent::SpecialistSpawned {
            specialist_type: "limit-evaluation-specialist".to_string(),
            trigger: format!("{} limit exceeded ({}/{})", limit_type, current_value, limit_value),
            task_id: Some(evaluation_task.id),
        }).await;

        let _ = event_tx.send(SwarmEvent::SpawnLimitExceeded {
            parent_task_id: parent_task.id,
            limit_type: limit_type.to_string(),
            current_value,
            limit_value,
        }).await;

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
        event_tx: &mpsc::Sender<SwarmEvent>,
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
            let resolution_exists = self.task_repo
                .list_by_goal(conflict.task_id)
                .await
                .map(|tasks| {
                    tasks.iter().any(|t| {
                        t.title.contains("Resolve merge conflict") &&
                        t.title.contains(&conflict.source_branch)
                    })
                })
                .unwrap_or(false);

            if !resolution_exists {
                if let Ok(Some(task)) = self.task_repo.get(conflict.task_id).await {
                    let Some(goal_id) = task.goal_id else {
                        continue;
                    };

                    let resolution_task = Task::new(
                        &format!("Resolve merge conflict: {} → {}", conflict.source_branch, conflict.target_branch),
                        &format!(
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
                        ),
                    )
                    .with_goal(goal_id)
                    .with_agent("merge-conflict-specialist");

                    if resolution_task.validate().is_ok() {
                        if let Ok(()) = self.task_repo.create(&resolution_task).await {
                            let _ = event_tx.send(SwarmEvent::SpecialistSpawned {
                                specialist_type: "merge-conflict-specialist".to_string(),
                                trigger: format!("Merge conflict in {} files", conflict.conflict_files.len()),
                                task_id: Some(resolution_task.id),
                            }).await;

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

//! Event handling subsystem for the swarm orchestrator.
//!
//! Manages human escalation events, A2A messaging, and event bus integration.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Max age for an escalation entry before eviction (7 days).
/// Matches the retention window used for the processed_commands dedup cache.
const ESCALATION_STORE_TTL_SECS: i64 = 7 * 24 * 3600;
/// Hard ceiling on resident escalation entries. If exceeded after TTL eviction,
/// the oldest entries by `created_at` are evicted until size <= this value.
const ESCALATION_STORE_MAX_SIZE: usize = 1024;

/// Prune the escalation store under the held write lock.
///
/// Runs on every mutation path (respond, defer, check_deadlines) to bound
/// memory. Evicts:
/// 1. Entries older than `ESCALATION_STORE_TTL_SECS`.
/// 2. If still over `ESCALATION_STORE_MAX_SIZE`, the oldest remaining entries
///    by `created_at` until size <= max.
fn prune_escalation_store_locked(
    store: &mut HashMap<Uuid, HumanEscalationEvent>,
    now: chrono::DateTime<chrono::Utc>,
) {
    let cutoff = now - chrono::Duration::seconds(ESCALATION_STORE_TTL_SECS);
    store.retain(|_, e| e.created_at >= cutoff);

    if store.len() > ESCALATION_STORE_MAX_SIZE {
        let mut ids_by_age: Vec<(Uuid, chrono::DateTime<chrono::Utc>)> =
            store.iter().map(|(id, e)| (*id, e.created_at)).collect();
        ids_by_age.sort_by_key(|(_, ts)| *ts);
        let excess = store.len() - ESCALATION_STORE_MAX_SIZE;
        for (id, _) in ids_by_age.into_iter().take(excess) {
            store.remove(&id);
        }
    }
}

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    EscalationDecision, GoalStatus, HumanEscalationEvent, HumanEscalationResponse, TaskStatus,
};
use crate::domain::ports::{
    AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository,
};
use crate::services::{AuditAction, AuditCategory};

use super::SwarmOrchestrator;
use super::types::SwarmEvent;

impl<G, T, W, A, M> SwarmOrchestrator<G, T, W, A, M>
where
    G: GoalRepository + 'static,
    T: TaskRepository + 'static,
    W: WorktreeRepository + 'static,
    A: AgentRepository + 'static,
    M: MemoryRepository + 'static,
{
    /// Get the EventBus.
    pub fn event_bus(&self) -> &Arc<crate::services::event_bus::EventBus> {
        &self.event_bus
    }

    // ========================================================================
    // Human Escalation Management
    // ========================================================================

    /// List pending (unresponded) escalation events.
    pub async fn list_pending_escalations(&self) -> Vec<HumanEscalationEvent> {
        self.runtime_state.escalation_store.read().await.values().cloned().collect()
    }

    /// Get the stored CommandBus, falling back to building one if not yet initialized.
    async fn get_command_bus(&self) -> Arc<crate::services::command_bus::CommandBus> {
        // Try the stored bus first (set during register_builtin_handlers)
        {
            let stored = self.command_bus.read().await;
            if let Some(ref bus) = *stored {
                return bus.clone();
            }
        }

        // Fallback: build one on the fly (should not happen in normal operation)
        use crate::domain::ports::NullMemoryRepository;
        use crate::services::command_bus::CommandBus;
        use crate::services::goal_service::GoalService;
        use crate::services::memory_maintenance_service::MemoryMaintenanceService;
        use crate::services::memory_service::MemoryService;
        use crate::services::task_service::TaskService;

        tracing::warn!("CommandBus not initialized — building ephemeral instance");
        let task_service = Arc::new(
            TaskService::new(self.task_repo.clone())
                .with_event_bus(self.event_bus.clone())
                .with_default_execution_mode(self.config.default_execution_mode.clone()),
        );
        let goal_service = Arc::new(GoalService::new(self.goal_repo.clone()));

        if let Some(ref memory_repo) = self.memory_repo {
            let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
            let maintenance_service =
                Arc::new(MemoryMaintenanceService::from_memory_service(memory_service));
            let mut bus = CommandBus::new(
                task_service,
                goal_service,
                maintenance_service,
                self.event_bus.clone(),
            );
            if let Some(ref pool) = self.pool {
                bus = bus.with_pool(pool.clone());
            }
            if let Some(ref outbox) = self.outbox_repo {
                bus = bus.with_outbox(outbox.clone());
            }
            Arc::new(bus)
        } else {
            let null_memory = Arc::new(MemoryService::new(Arc::new(NullMemoryRepository::new())));
            let null_maintenance =
                Arc::new(MemoryMaintenanceService::from_memory_service(null_memory));
            let mut bus = CommandBus::new(
                task_service,
                goal_service,
                null_maintenance,
                self.event_bus.clone(),
            );
            if let Some(ref pool) = self.pool {
                bus = bus.with_pool(pool.clone());
            }
            if let Some(ref outbox) = self.outbox_repo {
                bus = bus.with_outbox(outbox.clone());
            }
            Arc::new(bus)
        }
    }

    /// Respond to a human escalation event.
    pub async fn respond_to_escalation(
        &self,
        response: HumanEscalationResponse,
        _event_tx: Option<&mpsc::Sender<SwarmEvent>>,
    ) -> DomainResult<()> {
        // Find and remove the escalation from the store atomically.
        let escalation = {
            let mut store = self.runtime_state.escalation_store.write().await;
            let removed = store.remove(&response.event_id);
            // Opportunistically prune while we hold the write lock.
            prune_escalation_store_locked(&mut store, chrono::Utc::now());
            match removed {
                Some(e) => e,
                None => {
                    return Err(DomainError::ValidationFailed(format!(
                        "Escalation {} not found",
                        response.event_id
                    )));
                }
            }
        };

        self.apply_escalation_response(escalation, response).await
    }

    /// Apply an escalation response to an already-owned `HumanEscalationEvent`.
    ///
    /// This is the decision-application logic factored out of
    /// `respond_to_escalation` so callers that have already atomically drained
    /// entries from the store (e.g. `check_escalation_deadlines`) can process
    /// owned values without re-running the find-remove step.
    async fn apply_escalation_response(
        &self,
        escalation: HumanEscalationEvent,
        response: HumanEscalationResponse,
    ) -> DomainResult<()> {
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, GoalCommand, TaskCommand,
        };

        let command_bus = self.get_command_bus().await;

        match &response.decision {
            EscalationDecision::Accept => {
                // Unblock associated task if it was blocked
                if let Some(task_id) = escalation.task_id
                    && let Ok(Some(task)) = self.task_repo.get(task_id).await
                    && task.status == TaskStatus::Blocked
                {
                    let envelope = CommandEnvelope::new(
                        CommandSource::Human,
                        DomainCommand::Task(TaskCommand::Transition {
                            task_id,
                            new_status: TaskStatus::Ready,
                        }),
                    );
                    command_bus.dispatch(envelope).await.map_err(|e| {
                        DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e))
                    })?;
                }
            }
            EscalationDecision::Reject => {
                // Fail the associated task
                if let Some(task_id) = escalation.task_id {
                    let envelope = CommandEnvelope::new(
                        CommandSource::Human,
                        DomainCommand::Task(TaskCommand::Fail {
                            task_id,
                            error: Some("Escalation rejected by human".to_string()),
                        }),
                    );
                    command_bus.dispatch(envelope).await.map_err(|e| {
                        DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e))
                    })?;
                }
            }
            EscalationDecision::Clarify { clarification } => {
                // Append clarification to task description, then unblock via CommandBus
                if let Some(task_id) = escalation.task_id
                    && let Ok(Some(task)) = self.task_repo.get(task_id).await
                {
                    // Update description directly (no command for description updates)
                    let mut updated = task.clone();
                    updated.description = format!(
                        "{}\n\n## Human Clarification\n\n{}",
                        updated.description, clarification
                    );
                    self.task_repo.update(&updated).await?;

                    // Emit description update event via EventBus
                    self.event_bus
                        .publish(crate::services::event_factory::task_event(
                            crate::services::event_bus::EventSeverity::Info,
                            None,
                            task_id,
                            crate::services::event_bus::EventPayload::TaskDescriptionUpdated {
                                task_id,
                                reason: "Human clarification appended".to_string(),
                            },
                        ))
                        .await;

                    // Transition via CommandBus for proper event emission
                    if updated.status == TaskStatus::Blocked {
                        let envelope = CommandEnvelope::new(
                            CommandSource::Human,
                            DomainCommand::Task(TaskCommand::Transition {
                                task_id,
                                new_status: TaskStatus::Ready,
                            }),
                        );
                        command_bus.dispatch(envelope).await.map_err(|e| {
                            DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e))
                        })?;
                    }
                }
            }
            EscalationDecision::ModifyIntent {
                new_requirements,
                removed_requirements,
            } => {
                // Update goal description directly (no command for description updates)
                if let Some(goal_id) = escalation.goal_id
                    && let Ok(Some(mut goal)) = self.goal_repo.get(goal_id).await
                {
                    for req in new_requirements {
                        goal.description = format!("{}\n- {}", goal.description, req);
                    }
                    if !removed_requirements.is_empty() {
                        goal.description = format!(
                            "{}\n\n## Removed requirements:\n{}",
                            goal.description,
                            removed_requirements
                                .iter()
                                .map(|r| format!("- {}", r))
                                .collect::<Vec<_>>()
                                .join("\n")
                        );
                    }
                    self.goal_repo.update(&goal).await?;

                    // Emit description update event via EventBus
                    self.event_bus
                        .publish(crate::services::event_factory::goal_event(
                            crate::services::event_bus::EventSeverity::Info,
                            goal_id,
                            crate::services::event_bus::EventPayload::GoalDescriptionUpdated {
                                goal_id,
                                reason: "Human modified intent (requirements changed)".to_string(),
                            },
                        ))
                        .await;
                }
                // Unblock associated task via CommandBus
                if let Some(task_id) = escalation.task_id
                    && let Ok(Some(task)) = self.task_repo.get(task_id).await
                    && task.status == TaskStatus::Blocked
                {
                    let envelope = CommandEnvelope::new(
                        CommandSource::Human,
                        DomainCommand::Task(TaskCommand::Transition {
                            task_id,
                            new_status: TaskStatus::Ready,
                        }),
                    );
                    command_bus.dispatch(envelope).await.map_err(|e| {
                        DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e))
                    })?;
                }
            }
            EscalationDecision::Abort => {
                // Suspend the goal via CommandBus
                if let Some(goal_id) = escalation.goal_id {
                    let envelope = CommandEnvelope::new(
                        CommandSource::Human,
                        DomainCommand::Goal(GoalCommand::TransitionStatus {
                            goal_id,
                            new_status: GoalStatus::Paused,
                        }),
                    );
                    command_bus.dispatch(envelope).await.map_err(|e| {
                        DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e))
                    })?;
                }
            }
            EscalationDecision::Defer { revisit_after } => {
                // Put the escalation back with a deadline (no mutation, no command needed)
                let mut deferred = escalation.clone();
                if let Some(deadline) = revisit_after {
                    deferred.escalation.deadline = Some(*deadline);
                }
                let deferred_id = deferred.id;
                let mut store = self.runtime_state.escalation_store.write().await;
                store.insert(deferred_id, deferred);
                prune_escalation_store_locked(&mut store, chrono::Utc::now());
            }
        }

        // Emit response event
        let allows_continuation = response.decision.allows_continuation();
        let decision_str = response.decision.as_str().to_string();

        // (Bridge forwards EventBus→event_tx automatically)

        self.event_bus
            .publish(crate::services::event_factory::make_event(
                crate::services::event_bus::EventSeverity::Info,
                crate::services::event_bus::EventCategory::Escalation,
                None,
                None,
                crate::services::event_bus::EventPayload::HumanResponseReceived {
                    escalation_id: response.event_id,
                    decision: decision_str,
                    allows_continuation,
                },
            ))
            .await;

        self.audit_log
            .info(
                AuditCategory::Goal,
                AuditAction::GoalEvaluated,
                format!(
                    "Human response to escalation {}: {} (continue={})",
                    response.event_id,
                    response.decision.as_str(),
                    allows_continuation,
                ),
            )
            .await;

        Ok(())
    }

    /// Check escalation deadlines and apply default actions for timed-out escalations.
    pub async fn check_escalation_deadlines(
        &self,
        event_tx: &mpsc::Sender<SwarmEvent>,
    ) -> DomainResult<()> {
        let now = chrono::Utc::now();
        // Atomically REMOVE timed-out entries under the write lock, then
        // process the owned values. This closes the TOCTOU race: between
        // reading the list and issuing the auto-response, another task could
        // otherwise mutate (e.g. human `respond_to_escalation`) the same
        // entry, causing duplicate processing or lost decisions.
        let timed_out: Vec<HumanEscalationEvent> = {
            let mut store = self.runtime_state.escalation_store.write().await;
            let ids: Vec<Uuid> = store
                .iter()
                .filter(|(_, e)| e.escalation.deadline.is_some_and(|d| now > d))
                .map(|(id, _)| *id)
                .collect();
            let drained: Vec<HumanEscalationEvent> =
                ids.iter().filter_map(|id| store.remove(id)).collect();
            prune_escalation_store_locked(&mut store, now);
            drained
        };

        for escalation in timed_out {
            // Apply default action or accept
            let decision = if escalation.escalation.default_action.is_some() {
                EscalationDecision::Accept
            } else {
                EscalationDecision::Defer {
                    revisit_after: None,
                }
            };

            let response = HumanEscalationResponse {
                event_id: escalation.id,
                decision,
                response_text: Some("Auto-response: escalation deadline exceeded".to_string()),
                additional_context: None,
                responded_at: now,
            };

            let escalation_id = escalation.id;
            if let Err(e) = self.apply_escalation_response(escalation, response).await {
                tracing::warn!(
                    "Failed to auto-respond to timed-out escalation {}: {}",
                    escalation_id,
                    e
                );
            }
        }
        // event_tx is still part of the public signature for forward-compat;
        // the EventBus bridge now handles the notification path.
        let _ = event_tx;

        Ok(())
    }

    /// Send a message to another agent via A2A protocol.
    ///
    /// This allows agents to communicate and coordinate work.
    pub async fn send_a2a_message(
        &self,
        from_agent: &str,
        to_agent: &str,
        message_type: crate::domain::models::a2a::MessageType,
        subject: &str,
        content: &str,
    ) -> DomainResult<()> {
        use crate::domain::models::a2a::A2AMessage;

        let message = A2AMessage::new(message_type, from_agent, to_agent, subject, content);

        // Log the A2A message
        self.audit_log
            .info(
                AuditCategory::Agent,
                AuditAction::AgentSpawned, // Could add A2AMessageSent action
                format!(
                    "A2A message from '{}' to '{}': {} ({})",
                    from_agent,
                    to_agent,
                    message_type.as_str(),
                    message.id
                ),
            )
            .await;

        // If A2A gateway is configured, route the message via HTTP
        if let Some(ref gateway_url) = self.config.mcp_servers.a2a_gateway {
            // Build JSON-RPC request for tasks/send
            let request_id = Uuid::new_v4().to_string();
            let json_rpc_request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tasks/send",
                "params": {
                    "id": message.id.to_string(),
                    "message": {
                        "role": "user",
                        "parts": [{
                            "type": "text",
                            "text": format!(
                                "[A2A Message]\nFrom: {}\nTo: {}\nType: {}\nSubject: {}\n\n{}",
                                from_agent, to_agent, message_type.as_str(), subject, content
                            )
                        }]
                    },
                    "metadata": {
                        "from_agent": from_agent,
                        "to_agent": to_agent,
                        "message_type": message_type.as_str(),
                        "subject": subject,
                        "message_id": message.id.to_string(),
                        "task_id": message.task_id.as_ref().map(|t| t.to_string()),
                        "goal_id": message.goal_id.as_ref().map(|g| g.to_string()),
                    }
                }
            });

            // Send HTTP POST to A2A gateway
            let client = reqwest::Client::new();
            let rpc_url = format!("{}/rpc", gateway_url.trim_end_matches('/'));

            match client
                .post(&rpc_url)
                .json(&json_rpc_request)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            "A2A message {} routed successfully via gateway: {} -> {}",
                            message.id,
                            from_agent,
                            to_agent
                        );
                    } else {
                        tracing::warn!(
                            "A2A gateway returned error status {} for message {}",
                            response.status(),
                            message.id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to route A2A message {} via gateway: {}",
                        message.id,
                        e
                    );
                    // Don't fail the operation - message routing is best-effort
                }
            }
        } else {
            tracing::debug!(
                "A2A message {} not routed (no gateway configured): {} -> {}",
                message.id,
                from_agent,
                to_agent
            );
        }

        Ok(())
    }
}

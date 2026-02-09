//! Event handling subsystem for the swarm orchestrator.
//!
//! Manages human escalation events, A2A messaging, and event bus integration.

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    EscalationDecision, GoalStatus, HumanEscalationEvent, HumanEscalationResponse,
    TaskStatus,
};
use crate::domain::ports::{AgentRepository, GoalRepository, MemoryRepository, TaskRepository, WorktreeRepository};
use crate::services::{
    AuditAction, AuditCategory,
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
    /// Get the EventBus.
    pub fn event_bus(&self) -> &Arc<crate::services::event_bus::EventBus> {
        &self.event_bus
    }

    // ========================================================================
    // Human Escalation Management
    // ========================================================================

    /// List pending (unresponded) escalation events.
    pub async fn list_pending_escalations(&self) -> Vec<HumanEscalationEvent> {
        self.escalation_store.read().await.clone()
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
        use crate::services::memory_service::MemoryService;
        use crate::services::task_service::TaskService;

        tracing::warn!("CommandBus not initialized — building ephemeral instance");
        let task_service = Arc::new(TaskService::new(self.task_repo.clone()));
        let goal_service = Arc::new(GoalService::new(self.goal_repo.clone()));

        if let Some(ref memory_repo) = self.memory_repo {
            let memory_service = Arc::new(MemoryService::new(memory_repo.clone()));
            Arc::new(CommandBus::new(task_service, goal_service, memory_service, self.event_bus.clone()))
        } else {
            let null_memory = Arc::new(MemoryService::new(Arc::new(NullMemoryRepository::new())));
            Arc::new(CommandBus::new(task_service, goal_service, null_memory, self.event_bus.clone()))
        }
    }

    /// Respond to a human escalation event.
    pub async fn respond_to_escalation(
        &self,
        response: HumanEscalationResponse,
        _event_tx: Option<&mpsc::Sender<SwarmEvent>>,
    ) -> DomainResult<()> {
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, GoalCommand, TaskCommand,
        };

        // Find and remove the escalation from the store
        let escalation = {
            let mut store = self.escalation_store.write().await;
            let idx = store.iter().position(|e| e.id == response.event_id);
            match idx {
                Some(i) => store.remove(i),
                None => {
                    return Err(DomainError::ValidationFailed(format!(
                        "Escalation {} not found",
                        response.event_id
                    )));
                }
            }
        };

        let command_bus = self.get_command_bus().await;

        match &response.decision {
            EscalationDecision::Accept => {
                // Unblock associated task if it was blocked
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        if task.status == TaskStatus::Blocked {
                            let envelope = CommandEnvelope::new(
                                CommandSource::Human,
                                DomainCommand::Task(TaskCommand::Transition {
                                    task_id,
                                    new_status: TaskStatus::Ready,
                                }),
                            );
                            command_bus.dispatch(envelope).await
                                .map_err(|e| DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e)))?;
                        }
                    }
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
                    command_bus.dispatch(envelope).await
                        .map_err(|e| DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e)))?;
                }
            }
            EscalationDecision::Clarify { clarification } => {
                // Append clarification to task description, then unblock via CommandBus
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        // Update description directly (no command for description updates)
                        let mut updated = task.clone();
                        updated.description = format!(
                            "{}\n\n## Human Clarification\n\n{}",
                            updated.description, clarification
                        );
                        self.task_repo.update(&updated).await?;

                        // Emit description update event via EventBus
                        self.event_bus.publish(crate::services::event_factory::task_event(
                            crate::services::event_bus::EventSeverity::Info,
                            None,
                            task_id,
                            crate::services::event_bus::EventPayload::TaskDescriptionUpdated {
                                task_id,
                                reason: "Human clarification appended".to_string(),
                            },
                        )).await;

                        // Transition via CommandBus for proper event emission
                        if updated.status == TaskStatus::Blocked {
                            let envelope = CommandEnvelope::new(
                                CommandSource::Human,
                                DomainCommand::Task(TaskCommand::Transition {
                                    task_id,
                                    new_status: TaskStatus::Ready,
                                }),
                            );
                            command_bus.dispatch(envelope).await
                                .map_err(|e| DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e)))?;
                        }
                    }
                }
            }
            EscalationDecision::ModifyIntent { new_requirements, removed_requirements } => {
                // Update goal description directly (no command for description updates)
                if let Some(goal_id) = escalation.goal_id {
                    if let Ok(Some(mut goal)) = self.goal_repo.get(goal_id).await {
                        for req in new_requirements {
                            goal.description = format!("{}\n- {}", goal.description, req);
                        }
                        if !removed_requirements.is_empty() {
                            goal.description = format!(
                                "{}\n\n## Removed requirements:\n{}",
                                goal.description,
                                removed_requirements.iter()
                                    .map(|r| format!("- {}", r))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            );
                        }
                        self.goal_repo.update(&goal).await?;

                        // Emit description update event via EventBus
                        self.event_bus.publish(crate::services::event_factory::goal_event(
                            crate::services::event_bus::EventSeverity::Info,
                            goal_id,
                            crate::services::event_bus::EventPayload::GoalDescriptionUpdated {
                                goal_id,
                                reason: "Human modified intent (requirements changed)".to_string(),
                            },
                        )).await;
                    }
                }
                // Unblock associated task via CommandBus
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        if task.status == TaskStatus::Blocked {
                            let envelope = CommandEnvelope::new(
                                CommandSource::Human,
                                DomainCommand::Task(TaskCommand::Transition {
                                    task_id,
                                    new_status: TaskStatus::Ready,
                                }),
                            );
                            command_bus.dispatch(envelope).await
                                .map_err(|e| DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e)))?;
                        }
                    }
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
                    command_bus.dispatch(envelope).await
                        .map_err(|e| DomainError::ExecutionFailed(format!("Command dispatch failed: {}", e)))?;
                }
            }
            EscalationDecision::Defer { revisit_after } => {
                // Put the escalation back with a deadline (no mutation, no command needed)
                let mut deferred = escalation.clone();
                if let Some(deadline) = revisit_after {
                    deferred.escalation.deadline = Some(*deadline);
                }
                self.escalation_store.write().await.push(deferred);
            }
        }

        // Emit response event
        let allows_continuation = response.decision.allows_continuation();
        let decision_str = response.decision.as_str().to_string();

        // (Bridge forwards EventBus→event_tx automatically)

        self.event_bus.publish(crate::services::event_factory::make_event(
            crate::services::event_bus::EventSeverity::Info,
            crate::services::event_bus::EventCategory::Escalation,
            None,
            None,
            crate::services::event_bus::EventPayload::HumanResponseReceived {
                escalation_id: response.event_id,
                decision: decision_str,
                allows_continuation,
            },
        )).await;

        self.audit_log.info(
            AuditCategory::Goal,
            AuditAction::GoalEvaluated,
            format!(
                "Human response to escalation {}: {} (continue={})",
                response.event_id,
                response.decision.as_str(),
                allows_continuation,
            ),
        ).await;

        Ok(())
    }

    /// Check escalation deadlines and apply default actions for timed-out escalations.
    pub async fn check_escalation_deadlines(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let now = chrono::Utc::now();
        let timed_out: Vec<HumanEscalationEvent> = {
            let store = self.escalation_store.read().await;
            store.iter()
                .filter(|e| {
                    e.escalation.deadline.map_or(false, |d| now > d)
                })
                .cloned()
                .collect()
        };

        for escalation in timed_out {
            // Apply default action or accept
            let decision = if escalation.escalation.default_action.is_some() {
                EscalationDecision::Accept
            } else {
                EscalationDecision::Defer { revisit_after: None }
            };

            let response = HumanEscalationResponse {
                event_id: escalation.id,
                decision,
                response_text: Some("Auto-response: escalation deadline exceeded".to_string()),
                additional_context: None,
                responded_at: now,
            };

            if let Err(e) = self.respond_to_escalation(response, Some(event_tx)).await {
                tracing::warn!("Failed to auto-respond to timed-out escalation {}: {}", escalation.id, e);
            }
        }

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
        self.audit_log.info(
            AuditCategory::Agent,
            AuditAction::AgentSpawned, // Could add A2AMessageSent action
            format!(
                "A2A message from '{}' to '{}': {} ({})",
                from_agent, to_agent, message_type.as_str(), message.id
            ),
        ).await;

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

            match client.post(&rpc_url)
                .json(&json_rpc_request)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            "A2A message {} routed successfully via gateway: {} -> {}",
                            message.id, from_agent, to_agent
                        );
                    } else {
                        tracing::warn!(
                            "A2A gateway returned error status {} for message {}",
                            response.status(), message.id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to route A2A message {} via gateway: {}",
                        message.id, e
                    );
                    // Don't fail the operation - message routing is best-effort
                }
            }
        } else {
            tracing::debug!(
                "A2A message {} not routed (no gateway configured): {} -> {}",
                message.id, from_agent, to_agent
            );
        }

        Ok(())
    }

}

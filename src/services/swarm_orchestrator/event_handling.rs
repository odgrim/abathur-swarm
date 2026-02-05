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
    /// Get the EventBus if configured.
    pub fn event_bus(&self) -> Option<&Arc<crate::services::event_bus::EventBus>> {
        self.event_bus.as_ref()
    }

    /// Emit an event to the EventBus if configured.
    /// This is a helper for publishing events without needing the mpsc channel.
    pub async fn emit_to_event_bus(&self, event: SwarmEvent) {
        if let Some(ref bus) = self.event_bus {
            bus.publish_swarm_event(event).await;
        }
    }

    // ========================================================================
    // Human Escalation Management
    // ========================================================================

    /// List pending (unresponded) escalation events.
    pub async fn list_pending_escalations(&self) -> Vec<HumanEscalationEvent> {
        self.escalation_store.read().await.clone()
    }

    /// Respond to a human escalation event.
    pub async fn respond_to_escalation(
        &self,
        response: HumanEscalationResponse,
        event_tx: Option<&mpsc::Sender<SwarmEvent>>,
    ) -> DomainResult<()> {
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

        match &response.decision {
            EscalationDecision::Accept => {
                // Unblock associated task if it was blocked
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        if task.status == TaskStatus::Blocked {
                            let mut unblocked = task.clone();
                            if unblocked.transition_to(TaskStatus::Ready).is_ok() {
                                self.task_repo.update(&unblocked).await?;
                            }
                        }
                    }
                }
            }
            EscalationDecision::Reject => {
                // Fail the associated task
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        let mut failed = task.clone();
                        let _ = failed.transition_to(TaskStatus::Failed);
                        self.task_repo.update(&failed).await?;
                    }
                }
            }
            EscalationDecision::Clarify { clarification } => {
                // Append clarification to task description and unblock
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        let mut updated = task.clone();
                        updated.description = format!(
                            "{}\n\n## Human Clarification\n\n{}",
                            updated.description, clarification
                        );
                        if updated.status == TaskStatus::Blocked {
                            let _ = updated.transition_to(TaskStatus::Ready);
                        }
                        self.task_repo.update(&updated).await?;
                    }
                }
            }
            EscalationDecision::ModifyIntent { new_requirements, removed_requirements } => {
                // Update goal constraints
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
                    }
                }
                // Unblock associated task
                if let Some(task_id) = escalation.task_id {
                    if let Ok(Some(task)) = self.task_repo.get(task_id).await {
                        if task.status == TaskStatus::Blocked {
                            let mut unblocked = task.clone();
                            if unblocked.transition_to(TaskStatus::Ready).is_ok() {
                                self.task_repo.update(&unblocked).await?;
                            }
                        }
                    }
                }
            }
            EscalationDecision::Abort => {
                // Suspend the goal
                if let Some(goal_id) = escalation.goal_id {
                    if let Ok(Some(mut goal)) = self.goal_repo.get(goal_id).await {
                        let _ = goal.transition_to(GoalStatus::Paused);
                        self.goal_repo.update(&goal).await?;
                    }
                }
            }
            EscalationDecision::Defer { revisit_after } => {
                // Put the escalation back with a deadline
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

        if let Some(tx) = event_tx {
            let _ = tx.send(SwarmEvent::HumanResponseReceived {
                escalation_id: response.event_id,
                decision: decision_str.clone(),
                allows_continuation,
            }).await;
        }

        self.emit_to_event_bus(SwarmEvent::HumanResponseReceived {
            escalation_id: response.event_id,
            decision: decision_str,
            allows_continuation,
        }).await;

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

    /// Process A2A delegation requests from agents.
    ///
    /// Polls the A2A gateway for pending delegation messages and creates
    /// corresponding tasks for the target agents.
    pub(super) async fn process_a2a_delegations(&self, event_tx: &mpsc::Sender<SwarmEvent>) -> DomainResult<()> {
        let Some(ref a2a_url) = self.config.mcp_servers.a2a_gateway else {
            return Ok(());
        };

        // Poll A2A gateway for pending delegation messages
        // Using HTTP GET to fetch pending delegations
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/delegations/pending", a2a_url);

        let response = match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) => resp,
            Err(e) => {
                // Non-fatal: A2A gateway may not be running or reachable
                tracing::debug!("Failed to poll A2A gateway for delegations: {}", e);
                return Ok(());
            }
        };

        if !response.status().is_success() {
            return Ok(());
        }

        // Parse pending delegations
        #[derive(serde::Deserialize)]
        struct PendingDelegation {
            id: Uuid,
            sender_id: String,
            target_agent: String,
            task_description: String,
            parent_task_id: Option<Uuid>,
            goal_id: Option<Uuid>,
            priority: String,
        }

        let delegations: Vec<PendingDelegation> = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("Failed to parse A2A delegations: {}", e);
                return Ok(());
            }
        };

        for delegation in delegations {
            // Create a new task for the delegated work
            let priority = match delegation.priority.to_lowercase().as_str() {
                "critical" => crate::domain::models::TaskPriority::Critical,
                "high" => crate::domain::models::TaskPriority::High,
                "low" => crate::domain::models::TaskPriority::Low,
                _ => crate::domain::models::TaskPriority::Normal,
            };

            let mut task = crate::domain::models::Task::new(
                &format!("Delegated: {}", &delegation.task_description.chars().take(50).collect::<String>()),
                &format!(
                    "## A2A Delegation\n\n\
                    Delegated by: {}\n\n\
                    ## Task\n\n{}",
                    delegation.sender_id,
                    delegation.task_description
                ),
            )
            .with_priority(priority)
            .with_agent(&delegation.target_agent);

            if let Some(goal_id) = delegation.goal_id {
                task = task.with_goal(goal_id);
            }

            if let Some(parent_id) = delegation.parent_task_id {
                task.parent_id = Some(parent_id);
            }

            if task.validate().is_ok() {
                if let Ok(()) = self.task_repo.create(&task).await {
                    let _ = event_tx.send(SwarmEvent::TaskSubmitted {
                        task_id: task.id,
                        task_title: task.title.clone(),
                        goal_id: delegation.goal_id.unwrap_or(Uuid::nil()),
                    }).await;

                    self.audit_log.info(
                        AuditCategory::Task,
                        AuditAction::TaskCreated,
                        format!(
                            "Created delegated task {} for agent '{}' (from: {})",
                            task.id, delegation.target_agent, delegation.sender_id
                        ),
                    ).await;

                    // Acknowledge the delegation in A2A gateway
                    let ack_url = format!("{}/api/v1/delegations/{}/ack", a2a_url, delegation.id);
                    let _ = client.post(&ack_url).send().await;
                }
            }
        }

        Ok(())
    }
}

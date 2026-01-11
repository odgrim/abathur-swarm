//! Agent-to-Agent (A2A) protocol domain models.
//!
//! Enables structured communication between agents for handoffs,
//! delegation, and collaboration.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of A2A message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// Request to hand off a task.
    HandoffRequest,
    /// Accept a handoff request.
    HandoffAccept,
    /// Reject a handoff request.
    HandoffReject,
    /// Delegate a subtask.
    DelegateTask,
    /// Report progress on delegated work.
    ProgressReport,
    /// Request assistance.
    AssistanceRequest,
    /// Provide assistance response.
    AssistanceResponse,
    /// Notify of completion.
    CompletionNotify,
    /// Report an error.
    ErrorReport,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HandoffRequest => "handoff_request",
            Self::HandoffAccept => "handoff_accept",
            Self::HandoffReject => "handoff_reject",
            Self::DelegateTask => "delegate_task",
            Self::ProgressReport => "progress_report",
            Self::AssistanceRequest => "assistance_request",
            Self::AssistanceResponse => "assistance_response",
            Self::CompletionNotify => "completion_notify",
            Self::ErrorReport => "error_report",
        }
    }
}

/// Priority of an A2A message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagePriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl Default for MessagePriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// An A2A message between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Message type.
    pub message_type: MessageType,
    /// Priority level.
    pub priority: MessagePriority,
    /// Sender agent ID.
    pub sender_id: String,
    /// Receiver agent ID.
    pub receiver_id: String,
    /// Related task ID (if applicable).
    pub task_id: Option<Uuid>,
    /// Related goal ID (if applicable).
    pub goal_id: Option<Uuid>,
    /// Message subject/title.
    pub subject: String,
    /// Message body/content.
    pub body: String,
    /// Structured payload (JSON).
    pub payload: Option<serde_json::Value>,
    /// ID of message being replied to.
    pub reply_to: Option<Uuid>,
    /// Correlation ID for tracking conversations.
    pub correlation_id: Uuid,
    /// When message was created.
    pub created_at: DateTime<Utc>,
    /// When message expires (if applicable).
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether message has been acknowledged.
    pub acknowledged: bool,
}

impl A2AMessage {
    /// Create a new A2A message.
    pub fn new(
        message_type: MessageType,
        sender_id: impl Into<String>,
        receiver_id: impl Into<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        let id = Uuid::new_v4();
        Self {
            id,
            message_type,
            priority: MessagePriority::default(),
            sender_id: sender_id.into(),
            receiver_id: receiver_id.into(),
            task_id: None,
            goal_id: None,
            subject: subject.into(),
            body: body.into(),
            payload: None,
            reply_to: None,
            correlation_id: id, // Start new conversation by default
            created_at: Utc::now(),
            expires_at: None,
            acknowledged: false,
        }
    }

    /// Create a handoff request.
    pub fn handoff_request(
        sender_id: impl Into<String>,
        receiver_id: impl Into<String>,
        task_id: Uuid,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(
            MessageType::HandoffRequest,
            sender_id,
            receiver_id,
            "Handoff Request",
            reason,
        ).with_task(task_id)
    }

    /// Create a delegation message.
    pub fn delegate(
        sender_id: impl Into<String>,
        receiver_id: impl Into<String>,
        task_id: Uuid,
        instructions: impl Into<String>,
    ) -> Self {
        Self::new(
            MessageType::DelegateTask,
            sender_id,
            receiver_id,
            "Task Delegation",
            instructions,
        ).with_task(task_id)
    }

    /// Create a progress report.
    pub fn progress(
        sender_id: impl Into<String>,
        receiver_id: impl Into<String>,
        task_id: Uuid,
        progress_info: impl Into<String>,
    ) -> Self {
        Self::new(
            MessageType::ProgressReport,
            sender_id,
            receiver_id,
            "Progress Update",
            progress_info,
        ).with_task(task_id)
    }

    /// Create a completion notification.
    pub fn completion(
        sender_id: impl Into<String>,
        receiver_id: impl Into<String>,
        task_id: Uuid,
        summary: impl Into<String>,
    ) -> Self {
        Self::new(
            MessageType::CompletionNotify,
            sender_id,
            receiver_id,
            "Task Completed",
            summary,
        ).with_task(task_id)
    }

    /// Set the task ID.
    pub fn with_task(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Set the goal ID.
    pub fn with_goal(mut self, goal_id: Uuid) -> Self {
        self.goal_id = Some(goal_id);
        self
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: MessagePriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the correlation ID.
    pub fn with_correlation(mut self, correlation_id: Uuid) -> Self {
        self.correlation_id = correlation_id;
        self
    }

    /// Set the reply-to message ID.
    pub fn with_reply_to(mut self, reply_to: Uuid) -> Self {
        self.reply_to = Some(reply_to);
        self.correlation_id = reply_to; // Continue the conversation
        self
    }

    /// Set the payload.
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Set expiration.
    pub fn expires_in_secs(mut self, secs: i64) -> Self {
        self.expires_at = Some(Utc::now() + chrono::Duration::seconds(secs));
        self
    }

    /// Check if message has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at.map(|exp| Utc::now() > exp).unwrap_or(false)
    }

    /// Mark as acknowledged.
    pub fn acknowledge(&mut self) {
        self.acknowledged = true;
    }

    /// Create a reply to this message.
    pub fn reply(&self, message_type: MessageType, body: impl Into<String>) -> Self {
        Self::new(
            message_type,
            &self.receiver_id,
            &self.sender_id,
            format!("Re: {}", self.subject),
            body,
        )
        .with_reply_to(self.id)
        .with_correlation(self.correlation_id)
    }
}

/// An agent card describing an agent's capabilities for A2A discovery.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct A2AAgentCard {
    /// Agent ID/name.
    pub agent_id: String,
    /// Human-readable name.
    pub display_name: String,
    /// Description of capabilities.
    pub description: String,
    /// Agent tier.
    pub tier: String,
    /// List of capabilities/skills.
    pub capabilities: Vec<String>,
    /// Message types this agent accepts.
    pub accepts: Vec<MessageType>,
    /// Agents this agent can hand off to.
    pub handoff_targets: Vec<String>,
    /// Whether agent is currently available.
    pub available: bool,
    /// Current load (0.0 - 1.0).
    pub load: f64,
}

impl A2AAgentCard {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            available: true,
            ..Default::default()
        }
    }

    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = name.into();
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn with_capability(mut self, cap: impl Into<String>) -> Self {
        self.capabilities.push(cap.into());
        self
    }

    pub fn with_handoff_target(mut self, target: impl Into<String>) -> Self {
        self.handoff_targets.push(target.into());
        self
    }

    pub fn accepts_message_type(mut self, msg_type: MessageType) -> Self {
        self.accepts.push(msg_type);
        self
    }

    pub fn can_accept(&self, msg_type: MessageType) -> bool {
        self.accepts.is_empty() || self.accepts.contains(&msg_type)
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c.eq_ignore_ascii_case(cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let msg = A2AMessage::new(
            MessageType::HandoffRequest,
            "sender",
            "receiver",
            "Test Subject",
            "Test body",
        );

        assert_eq!(msg.sender_id, "sender");
        assert_eq!(msg.receiver_id, "receiver");
        assert!(!msg.acknowledged);
    }

    #[test]
    fn test_handoff_request() {
        let task_id = Uuid::new_v4();
        let msg = A2AMessage::handoff_request(
            "architect-1",
            "worker-1",
            task_id,
            "Need specialized handling",
        );

        assert_eq!(msg.message_type, MessageType::HandoffRequest);
        assert_eq!(msg.task_id, Some(task_id));
    }

    #[test]
    fn test_message_reply() {
        let original = A2AMessage::new(
            MessageType::HandoffRequest,
            "sender",
            "receiver",
            "Request",
            "Body",
        );

        let reply = original.reply(MessageType::HandoffAccept, "Accepted");

        assert_eq!(reply.sender_id, "receiver");
        assert_eq!(reply.receiver_id, "sender");
        assert_eq!(reply.reply_to, Some(original.id));
        assert_eq!(reply.correlation_id, original.correlation_id);
    }

    #[test]
    fn test_agent_card() {
        let card = A2AAgentCard::new("test-agent")
            .with_display_name("Test Agent")
            .with_capability("coding")
            .with_capability("testing")
            .with_handoff_target("reviewer");

        assert!(card.has_capability("coding"));
        assert!(card.has_capability("TESTING"));
        assert!(!card.has_capability("unknown"));
        assert_eq!(card.handoff_targets.len(), 1);
    }
}

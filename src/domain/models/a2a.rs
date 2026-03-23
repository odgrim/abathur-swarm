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
    /// Federation: delegate a task to a cerebrate.
    FederationDelegate,
    /// Federation: cerebrate accepts a delegated task.
    FederationAccept,
    /// Federation: cerebrate rejects a delegated task.
    FederationReject,
    /// Federation: progress update from cerebrate.
    FederationProgress,
    /// Federation: final result from cerebrate.
    FederationResult,
    /// Federation: heartbeat ping/pong.
    FederationHeartbeat,
    /// Federation: discover a cerebrate's capabilities.
    FederationDiscover,
    /// Federation: register with a parent/cerebrate.
    FederationRegister,
    /// Federation: disconnect from federation.
    FederationDisconnect,
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
            Self::FederationDelegate => "federation_delegate",
            Self::FederationAccept => "federation_accept",
            Self::FederationReject => "federation_reject",
            Self::FederationProgress => "federation_progress",
            Self::FederationResult => "federation_result",
            Self::FederationHeartbeat => "federation_heartbeat",
            Self::FederationDiscover => "federation_discover",
            Self::FederationRegister => "federation_register",
            Self::FederationDisconnect => "federation_disconnect",
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

// ============================================================================
// Federation Domain Models
// ============================================================================

/// Role of a swarm in a federation topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederationRole {
    /// Parent swarm that delegates work.
    Overmind,
    /// Child swarm that accepts delegated work.
    Cerebrate,
}

impl std::fmt::Display for FederationRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overmind => write!(f, "overmind"),
            Self::Cerebrate => write!(f, "cerebrate"),
        }
    }
}

/// Connection state of a cerebrate in the federation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Unreachable,
    Reconnecting,
    Disconnecting,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "disconnected"),
            Self::Connecting => write!(f, "connecting"),
            Self::Connected => write!(f, "connected"),
            Self::Unreachable => write!(f, "unreachable"),
            Self::Reconnecting => write!(f, "reconnecting"),
            Self::Disconnecting => write!(f, "disconnecting"),
        }
    }
}

/// Status of a federation task result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederationTaskStatus {
    Completed,
    Failed,
    Partial,
}

impl std::fmt::Display for FederationTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Partial => write!(f, "partial"),
        }
    }
}

/// An artifact reference in a federation result (PR URL, commit SHA, doc link, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// Type of artifact (e.g., "pr_url", "commit_sha", "doc_link").
    pub artifact_type: String,
    /// Value of the artifact reference.
    pub value: String,
}

impl Artifact {
    pub fn new(artifact_type: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            artifact_type: artifact_type.into(),
            value: value.into(),
        }
    }
}

/// Context passed alongside a delegated task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FederationTaskContext {
    /// Summary of the parent goal this task serves.
    pub parent_goal_summary: Option<String>,
    /// Related artifact references for context.
    pub related_artifacts: Vec<Artifact>,
    /// Free-form hints for the cerebrate.
    pub hints: Vec<String>,
}

/// Envelope wrapping a task delegated to a cerebrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationTaskEnvelope {
    /// ID of the delegated task.
    pub task_id: Uuid,
    /// Parent goal this task belongs to.
    pub parent_goal_id: Option<Uuid>,
    /// Correlation ID for tracking the delegation conversation.
    pub correlation_id: Uuid,
    /// Short title.
    pub title: String,
    /// Detailed description / instructions.
    pub description: String,
    /// Constraints the cerebrate must follow.
    pub constraints: Vec<String>,
    /// Priority level.
    pub priority: MessagePriority,
    /// Contextual information from the parent swarm.
    pub context: FederationTaskContext,
    /// Seconds before the cerebrate must accept or reject (0 = no timeout).
    pub accept_timeout_secs: u64,
    /// Optional schema ID for the expected result format.
    pub result_schema: Option<String>,
}

impl FederationTaskEnvelope {
    pub fn new(
        task_id: Uuid,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            parent_goal_id: None,
            correlation_id: Uuid::new_v4(),
            title: title.into(),
            description: description.into(),
            constraints: Vec::new(),
            priority: MessagePriority::Normal,
            context: FederationTaskContext::default(),
            accept_timeout_secs: 300,
            result_schema: None,
        }
    }

    pub fn with_parent_goal(mut self, goal_id: Uuid) -> Self {
        self.parent_goal_id = Some(goal_id);
        self
    }

    pub fn with_priority(mut self, priority: MessagePriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraints.push(constraint.into());
        self
    }

    pub fn with_context(mut self, context: FederationTaskContext) -> Self {
        self.context = context;
        self
    }

    pub fn with_result_schema(mut self, schema_id: impl Into<String>) -> Self {
        self.result_schema = Some(schema_id.into());
        self
    }
}

/// Structured result returned by a cerebrate after completing a delegated task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationResult {
    /// ID of the delegated task.
    pub task_id: Uuid,
    /// Correlation ID matching the delegation envelope.
    pub correlation_id: Uuid,
    /// Outcome status.
    pub status: FederationTaskStatus,
    /// Human-readable summary.
    pub summary: String,
    /// Artifact references produced.
    pub artifacts: Vec<Artifact>,
    /// Metrics (e.g., tokens used, duration).
    pub metrics: std::collections::HashMap<String, serde_json::Value>,
    /// Optional notes for the parent swarm.
    pub notes: Option<String>,
    /// If failed, the reason.
    pub failure_reason: Option<String>,
    /// Suggestions for follow-up work.
    pub suggestions: Vec<String>,
}

impl FederationResult {
    pub fn completed(task_id: Uuid, correlation_id: Uuid, summary: impl Into<String>) -> Self {
        Self {
            task_id,
            correlation_id,
            status: FederationTaskStatus::Completed,
            summary: summary.into(),
            artifacts: Vec::new(),
            metrics: std::collections::HashMap::new(),
            notes: None,
            failure_reason: None,
            suggestions: Vec::new(),
        }
    }

    pub fn failed(
        task_id: Uuid,
        correlation_id: Uuid,
        summary: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            correlation_id,
            status: FederationTaskStatus::Failed,
            summary: summary.into(),
            artifacts: Vec::new(),
            metrics: std::collections::HashMap::new(),
            notes: None,
            failure_reason: Some(reason.into()),
            suggestions: Vec::new(),
        }
    }

    pub fn with_artifact(mut self, artifact: Artifact) -> Self {
        self.artifacts.push(artifact);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes = Some(note.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestions.push(suggestion.into());
        self
    }
}

/// Extended agent card for federation — includes federation-specific metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationCard {
    /// Base agent card.
    #[serde(flatten)]
    pub card: A2AAgentCard,
    /// Parent swarm ID (if this is a cerebrate).
    pub parent_id: Option<String>,
    /// Hive/federation cluster ID.
    pub hive_id: Option<String>,
    /// Role in the federation.
    pub federation_role: FederationRole,
    /// Maximum tasks this swarm will accept concurrently.
    pub max_accepted_tasks: u32,
    /// Current heartbeat state.
    pub heartbeat_ok: bool,
}

impl FederationCard {
    pub fn new(card: A2AAgentCard, role: FederationRole) -> Self {
        Self {
            card,
            parent_id: None,
            hive_id: None,
            federation_role: role,
            max_accepted_tasks: 10,
            heartbeat_ok: true,
        }
    }

    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    pub fn with_hive(mut self, hive_id: impl Into<String>) -> Self {
        self.hive_id = Some(hive_id.into());
        self
    }

    /// Build a FederationCard from an A2A standard agent card.
    ///
    /// Used when the A2A discovery path succeeds and we need to convert
    /// the wire-format card into the internal federation representation.
    pub fn from_a2a_agent_card(
        a2a_card: &super::a2a_protocol::A2AStandardAgentCard,
    ) -> Self {
        let card = A2AAgentCard {
            agent_id: a2a_card.id.clone(),
            display_name: a2a_card.name.clone(),
            description: a2a_card.description.clone(),
            tier: "federation".to_string(),
            capabilities: a2a_card
                .skills
                .iter()
                .map(|s| s.name.clone())
                .collect(),
            accepts: vec![],
            handoff_targets: vec![],
            available: true,
            load: 0.0,
        };
        Self {
            card,
            parent_id: None,
            hive_id: None,
            federation_role: FederationRole::Cerebrate,
            max_accepted_tasks: 10,
            heartbeat_ok: true,
        }
    }
}

/// Status of a connected cerebrate as tracked by the overmind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CerebrateStatus {
    /// Unique cerebrate ID.
    pub id: String,
    /// Human-readable name.
    pub display_name: String,
    /// Current connection state.
    pub connection_state: ConnectionState,
    /// Capabilities advertised by the cerebrate.
    pub capabilities: Vec<String>,
    /// Current load (0.0–1.0).
    pub load: f64,
    /// Number of tasks currently delegated to this cerebrate.
    pub active_delegations: u32,
    /// Maximum concurrent delegations.
    pub max_concurrent_delegations: u32,
    /// Timestamp of last successful heartbeat.
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    /// URL of the cerebrate.
    pub url: Option<String>,
    /// Number of consecutive missed heartbeats.
    pub missed_heartbeats: u32,
}

impl CerebrateStatus {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            connection_state: ConnectionState::Disconnected,
            capabilities: Vec::new(),
            load: 0.0,
            active_delegations: 0,
            max_concurrent_delegations: 10,
            last_heartbeat_at: None,
            url: None,
            missed_heartbeats: 0,
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_max_delegations(mut self, max: u32) -> Self {
        self.max_concurrent_delegations = max;
        self
    }

    /// Whether this cerebrate can accept another task.
    pub fn can_accept_task(&self) -> bool {
        self.connection_state == ConnectionState::Connected
            && self.active_delegations < self.max_concurrent_delegations
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

    // -- Federation domain model tests --

    #[test]
    fn test_federation_task_envelope_roundtrip() {
        let task_id = Uuid::new_v4();
        let envelope = FederationTaskEnvelope::new(task_id, "Test task", "Do the thing")
            .with_parent_goal(Uuid::new_v4())
            .with_priority(MessagePriority::High)
            .with_constraint("Must not break CI")
            .with_result_schema("standard_v1");

        let json = serde_json::to_string(&envelope).unwrap();
        let roundtrip: FederationTaskEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.task_id, task_id);
        assert_eq!(roundtrip.title, "Test task");
        assert_eq!(roundtrip.priority, MessagePriority::High);
        assert_eq!(roundtrip.constraints.len(), 1);
        assert_eq!(roundtrip.result_schema.as_deref(), Some("standard_v1"));
    }

    #[test]
    fn test_federation_result_roundtrip() {
        let task_id = Uuid::new_v4();
        let corr_id = Uuid::new_v4();
        let result = FederationResult::completed(task_id, corr_id, "All done")
            .with_artifact(Artifact::new("pr_url", "https://github.com/org/repo/pull/42"))
            .with_note("Took 5 minutes")
            .with_suggestion("Consider adding more tests");

        let json = serde_json::to_string(&result).unwrap();
        let roundtrip: FederationResult = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.task_id, task_id);
        assert_eq!(roundtrip.status, FederationTaskStatus::Completed);
        assert_eq!(roundtrip.artifacts.len(), 1);
        assert_eq!(roundtrip.artifacts[0].artifact_type, "pr_url");
        assert!(roundtrip.failure_reason.is_none());
        assert_eq!(roundtrip.suggestions.len(), 1);
    }

    #[test]
    fn test_federation_result_failed_roundtrip() {
        let task_id = Uuid::new_v4();
        let corr_id = Uuid::new_v4();
        let result = FederationResult::failed(task_id, corr_id, "Could not complete", "CI failed");

        let json = serde_json::to_string(&result).unwrap();
        let roundtrip: FederationResult = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.status, FederationTaskStatus::Failed);
        assert_eq!(roundtrip.failure_reason.as_deref(), Some("CI failed"));
    }

    #[test]
    fn test_federation_card_roundtrip() {
        let card = FederationCard::new(
            A2AAgentCard::new("cerebrate-1").with_display_name("My Cerebrate"),
            FederationRole::Cerebrate,
        )
        .with_parent("overmind-1")
        .with_hive("hive-alpha");

        let json = serde_json::to_string(&card).unwrap();
        let roundtrip: FederationCard = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.federation_role, FederationRole::Cerebrate);
        assert_eq!(roundtrip.parent_id.as_deref(), Some("overmind-1"));
        assert_eq!(roundtrip.hive_id.as_deref(), Some("hive-alpha"));
        assert_eq!(roundtrip.card.display_name, "My Cerebrate");
    }

    #[test]
    fn test_cerebrate_status_can_accept_task() {
        let mut status = CerebrateStatus::new("c1", "Cerebrate 1")
            .with_max_delegations(2);
        status.connection_state = ConnectionState::Connected;

        assert!(status.can_accept_task());

        status.active_delegations = 2;
        assert!(!status.can_accept_task());

        status.active_delegations = 1;
        status.connection_state = ConnectionState::Unreachable;
        assert!(!status.can_accept_task());
    }

    #[test]
    fn test_connection_state_transitions() {
        let mut status = CerebrateStatus::new("c1", "Cerebrate 1");
        assert_eq!(status.connection_state, ConnectionState::Disconnected);

        status.connection_state = ConnectionState::Connecting;
        assert_eq!(status.connection_state, ConnectionState::Connecting);

        status.connection_state = ConnectionState::Connected;
        assert_eq!(status.connection_state, ConnectionState::Connected);
    }

    #[test]
    fn test_federation_role_display() {
        assert_eq!(FederationRole::Overmind.to_string(), "overmind");
        assert_eq!(FederationRole::Cerebrate.to_string(), "cerebrate");
    }

    #[test]
    fn test_federation_message_types() {
        assert_eq!(MessageType::FederationDelegate.as_str(), "federation_delegate");
        assert_eq!(MessageType::FederationResult.as_str(), "federation_result");
        assert_eq!(MessageType::FederationHeartbeat.as_str(), "federation_heartbeat");

        // Verify roundtrip through serde
        let json = serde_json::to_string(&MessageType::FederationDelegate).unwrap();
        let roundtrip: MessageType = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, MessageType::FederationDelegate);
    }

    #[test]
    fn test_artifact_creation() {
        let artifact = Artifact::new("commit_sha", "abc123");
        assert_eq!(artifact.artifact_type, "commit_sha");
        assert_eq!(artifact.value, "abc123");

        let json = serde_json::to_string(&artifact).unwrap();
        let roundtrip: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, artifact);
    }
}

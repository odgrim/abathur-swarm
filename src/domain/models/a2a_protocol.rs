//! Standard A2A (Agent2Agent) protocol types per the A2A specification v0.3.
//!
//! These types represent the wire format for inter-swarm communication.
//! Internal domain types (in `a2a.rs`) are converted to/from these for transport.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::a2a::{
    FederationCard, FederationResult, FederationTaskEnvelope, FederationTaskStatus,
    MessagePriority,
};

// ---------------------------------------------------------------------------
// Task lifecycle (A2A spec §4.1.3)
// ---------------------------------------------------------------------------

/// A2A task lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum A2ATaskState {
    Submitted,
    Working,
    InputRequired,
    AuthRequired,
    Completed,
    Failed,
    Canceled,
    Rejected,
}

impl A2ATaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input-required",
            Self::AuthRequired => "auth-required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Rejected => "rejected",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Canceled | Self::Rejected
        )
    }
}

// ---------------------------------------------------------------------------
// Message types (A2A spec §4.1.4, §4.1.6)
// ---------------------------------------------------------------------------

/// Role of a message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum A2ARole {
    User,
    Agent,
}

/// File reference within a Part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AFileRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
}

/// A Part — the atomic content unit within messages and artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum A2APart {
    Text {
        text: String,
    },
    File {
        file: A2AFileRef,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
    Data {
        data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
}

/// A message in the A2A protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AProtocolMessage {
    pub role: A2ARole,
    pub parts: Vec<A2APart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Task status and task (A2A spec §4.1.1, §4.1.3)
// ---------------------------------------------------------------------------

/// Status of an A2A task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATaskStatus {
    pub state: A2ATaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<A2AProtocolMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

/// An artifact produced by a task (A2A spec §4.1.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AProtocolArtifact {
    pub artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parts: Vec<A2APart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_chunk: Option<bool>,
}

/// An A2A task (A2A spec §4.1.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: A2ATaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<A2AProtocolMessage>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<A2AProtocolArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Agent card (A2A spec §4.4)
// ---------------------------------------------------------------------------

/// A skill declared in an agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ASkill {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

/// Capabilities declared in an agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ACapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub push_notifications: bool,
    #[serde(default)]
    pub state_transition_history: bool,
}

impl Default for A2ACapabilities {
    fn default() -> Self {
        Self {
            streaming: true,
            push_notifications: false,
            state_transition_history: false,
        }
    }
}

/// Provider info in an agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AProvider {
    pub organization: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Standard A2A agent card (published at `/.well-known/agent.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AStandardAgentCard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<A2AProvider>,
    pub capabilities: A2ACapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<A2ASkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_schemes: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_input_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_output_modes: Vec<String>,
}

// ---------------------------------------------------------------------------
// SSE streaming events
// ---------------------------------------------------------------------------

/// Status update event in an SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATaskStatusUpdateEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: A2ATaskStatus,
    #[serde(rename = "final")]
    pub final_flag: bool,
}

/// Artifact update event in an SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATaskArtifactUpdateEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub artifact: A2AProtocolArtifact,
}

/// A stream event from an A2A SSE connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum A2AStreamEvent {
    Task(A2ATask),
    StatusUpdate(A2ATaskStatusUpdateEvent),
    ArtifactUpdate(A2ATaskArtifactUpdateEvent),
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

/// A2A JSON-RPC error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AJsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A2A JSON-RPC request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AJsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl A2AJsonRpcRequest {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            id: serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
            params: Some(params),
        }
    }
}

/// A2A JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AJsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<A2AJsonRpcError>,
}

// ---------------------------------------------------------------------------
// Request parameter types
// ---------------------------------------------------------------------------

/// Parameters for `tasks/send` and `tasks/sendSubscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSendParams {
    pub message: A2AProtocolMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_notification_config: Option<serde_json::Value>,
}

/// Parameters for `tasks/get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueryParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

// ---------------------------------------------------------------------------
// From conversions: bespoke federation types <-> A2A wire types
// ---------------------------------------------------------------------------

impl From<FederationTaskStatus> for A2ATaskState {
    fn from(status: FederationTaskStatus) -> Self {
        match status {
            FederationTaskStatus::Completed => Self::Completed,
            FederationTaskStatus::Failed => Self::Failed,
            FederationTaskStatus::Partial => Self::Working,
        }
    }
}

impl From<A2ATaskState> for Option<FederationTaskStatus> {
    fn from(state: A2ATaskState) -> Self {
        match state {
            A2ATaskState::Completed => Some(FederationTaskStatus::Completed),
            A2ATaskState::Failed => Some(FederationTaskStatus::Failed),
            A2ATaskState::Canceled | A2ATaskState::Rejected => Some(FederationTaskStatus::Failed),
            A2ATaskState::Working => Some(FederationTaskStatus::Partial),
            _ => None,
        }
    }
}

impl From<&FederationCard> for A2AStandardAgentCard {
    fn from(card: &FederationCard) -> Self {
        Self {
            id: card.card.agent_id.clone(),
            name: card.card.display_name.clone(),
            description: card.card.description.clone(),
            url: String::new(),
            version: Some("0.3".to_string()),
            provider: Some(A2AProvider {
                organization: "Abathur Swarm".to_string(),
                url: None,
            }),
            capabilities: A2ACapabilities {
                streaming: true,
                push_notifications: false,
                state_transition_history: false,
            },
            skills: card
                .card
                .capabilities
                .iter()
                .map(|cap| A2ASkill {
                    id: cap.to_lowercase().replace(' ', "-"),
                    name: cap.clone(),
                    description: None,
                    tags: vec![],
                    examples: vec![],
                })
                .collect(),
            security_schemes: vec![],
            default_input_modes: vec!["application/json".to_string()],
            default_output_modes: vec!["application/json".to_string()],
        }
    }
}

impl From<&FederationTaskEnvelope> for TaskSendParams {
    fn from(envelope: &FederationTaskEnvelope) -> Self {
        let mut federation_data = serde_json::Map::new();
        federation_data.insert(
            "intent".to_string(),
            serde_json::Value::String("delegate".to_string()),
        );
        federation_data.insert(
            "task_id".to_string(),
            serde_json::Value::String(envelope.task_id.to_string()),
        );
        if let Some(goal_id) = envelope.parent_goal_id {
            federation_data.insert(
                "parent_goal_id".to_string(),
                serde_json::Value::String(goal_id.to_string()),
            );
        }
        federation_data.insert(
            "correlation_id".to_string(),
            serde_json::Value::String(envelope.correlation_id.to_string()),
        );
        if !envelope.constraints.is_empty() {
            federation_data.insert(
                "constraints".to_string(),
                // Safety: Vec<String> serialization is infallible; unwrap_or_default
                // is a defensive fallback that can never actually trigger.
                serde_json::to_value(&envelope.constraints).unwrap_or_default(),
            );
        }
        if let Some(ref schema) = envelope.result_schema {
            federation_data.insert(
                "result_schema".to_string(),
                serde_json::Value::String(schema.clone()),
            );
        }
        if !envelope.required_capabilities.is_empty() {
            federation_data.insert(
                "required_capabilities".to_string(),
                serde_json::to_value(&envelope.required_capabilities).unwrap_or_default(),
            );
        }

        let mut metadata = HashMap::new();
        metadata.insert(
            "abathur:federation".to_string(),
            serde_json::Value::Object(federation_data),
        );

        let message = A2AProtocolMessage {
            role: A2ARole::User,
            parts: vec![
                A2APart::Text {
                    text: format!("{}\n\n{}", envelope.title, envelope.description),
                },
                A2APart::Data {
                    data: serde_json::json!({
                        "title": envelope.title,
                        "description": envelope.description,
                        "priority": envelope.priority.as_str(),
                        "context": envelope.context,
                    }),
                    metadata: None,
                },
            ],
            metadata: None,
        };

        Self {
            message,
            metadata: Some(metadata),
            history_length: None,
            push_notification_config: None,
        }
    }
}

impl From<&FederationResult> for A2ATask {
    fn from(result: &FederationResult) -> Self {
        let state = match result.status {
            FederationTaskStatus::Completed => A2ATaskState::Completed,
            FederationTaskStatus::Failed => A2ATaskState::Failed,
            FederationTaskStatus::Partial => A2ATaskState::Working,
        };

        let status_message = A2AProtocolMessage {
            role: A2ARole::Agent,
            parts: vec![A2APart::Text {
                text: result.summary.clone(),
            }],
            metadata: None,
        };

        let artifacts: Vec<A2AProtocolArtifact> = result
            .artifacts
            .iter()
            .enumerate()
            .map(|(i, a)| A2AProtocolArtifact {
                artifact_id: format!("artifact-{}", i),
                name: Some(a.artifact_type.clone()),
                description: None,
                parts: vec![A2APart::Text {
                    text: a.value.clone(),
                }],
                metadata: None,
                index: Some(i as u32),
                append: None,
                last_chunk: Some(true),
            })
            .collect();

        let mut metadata = HashMap::new();
        metadata.insert(
            "correlation_id".to_string(),
            serde_json::Value::String(result.correlation_id.to_string()),
        );
        if !result.metrics.is_empty() {
            metadata.insert(
                "metrics".to_string(),
                // Safety: HashMap<String, f64> serialization is infallible;
                // unwrap_or_default is a defensive fallback that can never actually trigger.
                serde_json::to_value(&result.metrics).unwrap_or_default(),
            );
        }
        if let Some(ref reason) = result.failure_reason {
            metadata.insert(
                "failure_reason".to_string(),
                serde_json::Value::String(reason.clone()),
            );
        }

        Self {
            id: result.task_id.to_string(),
            context_id: None,
            status: A2ATaskStatus {
                state,
                message: Some(status_message),
                timestamp: Some(Utc::now()),
            },
            history: None,
            artifacts,
            metadata: Some(metadata),
        }
    }
}

// Helper for MessagePriority (used by From impls)
impl MessagePriority {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::a2a::Artifact;

    #[test]
    fn test_task_state_is_terminal() {
        assert!(A2ATaskState::Completed.is_terminal());
        assert!(A2ATaskState::Failed.is_terminal());
        assert!(A2ATaskState::Canceled.is_terminal());
        assert!(A2ATaskState::Rejected.is_terminal());
        assert!(!A2ATaskState::Submitted.is_terminal());
        assert!(!A2ATaskState::Working.is_terminal());
        assert!(!A2ATaskState::InputRequired.is_terminal());
        assert!(!A2ATaskState::AuthRequired.is_terminal());
    }

    #[test]
    fn test_task_state_serde_roundtrip() {
        let state = A2ATaskState::InputRequired;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"input-required\"");
        let parsed: A2ATaskState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn test_protocol_message_serde_roundtrip() {
        let msg = A2AProtocolMessage {
            role: A2ARole::User,
            parts: vec![
                A2APart::Text {
                    text: "Hello".to_string(),
                },
                A2APart::Data {
                    data: serde_json::json!({"key": "value"}),
                    metadata: None,
                },
            ],
            metadata: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: A2AProtocolMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, A2ARole::User);
        assert_eq!(parsed.parts.len(), 2);
    }

    #[test]
    fn test_a2a_task_serde_roundtrip() {
        let task = A2ATask {
            id: "task-123".to_string(),
            context_id: None,
            status: A2ATaskStatus {
                state: A2ATaskState::Working,
                message: None,
                timestamp: Some(Utc::now()),
            },
            history: None,
            artifacts: vec![],
            metadata: None,
        };
        let json = serde_json::to_string(&task).unwrap();
        let parsed: A2ATask = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "task-123");
        assert_eq!(parsed.status.state, A2ATaskState::Working);
    }

    #[test]
    fn test_from_federation_task_status() {
        assert_eq!(
            A2ATaskState::from(FederationTaskStatus::Completed),
            A2ATaskState::Completed
        );
        assert_eq!(
            A2ATaskState::from(FederationTaskStatus::Failed),
            A2ATaskState::Failed
        );
        assert_eq!(
            A2ATaskState::from(FederationTaskStatus::Partial),
            A2ATaskState::Working
        );
    }

    #[test]
    fn test_from_a2a_task_state_to_federation_status() {
        let completed: Option<FederationTaskStatus> = A2ATaskState::Completed.into();
        assert_eq!(completed, Some(FederationTaskStatus::Completed));

        let working: Option<FederationTaskStatus> = A2ATaskState::Working.into();
        assert_eq!(working, Some(FederationTaskStatus::Partial));

        let submitted: Option<FederationTaskStatus> = A2ATaskState::Submitted.into();
        assert_eq!(submitted, None);
    }

    #[test]
    fn test_from_federation_task_envelope() {
        let envelope = FederationTaskEnvelope::new(
            uuid::Uuid::new_v4(),
            "Test task",
            "Do something useful",
        )
        .with_constraint("Must not break CI");

        let params = TaskSendParams::from(&envelope);
        assert_eq!(params.message.role, A2ARole::User);
        assert_eq!(params.message.parts.len(), 2);

        let metadata = params.metadata.unwrap();
        let fed = metadata.get("abathur:federation").unwrap();
        assert_eq!(fed["intent"], "delegate");
    }

    #[test]
    fn test_from_federation_result() {
        let result = FederationResult::completed(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            "All done",
        )
        .with_artifact(Artifact::new("pr_url", "https://github.com/org/repo/pull/1"));

        let task = A2ATask::from(&result);
        assert_eq!(task.status.state, A2ATaskState::Completed);
        assert_eq!(task.artifacts.len(), 1);
        assert_eq!(task.artifacts[0].name.as_deref(), Some("pr_url"));
    }

    #[test]
    fn test_agent_card_serde() {
        let card = A2AStandardAgentCard {
            id: "test-swarm".to_string(),
            name: "Test Swarm".to_string(),
            description: "A test swarm".to_string(),
            url: "http://localhost:8080".to_string(),
            version: Some("0.3".to_string()),
            provider: None,
            capabilities: A2ACapabilities::default(),
            skills: vec![A2ASkill {
                id: "coding".to_string(),
                name: "Coding".to_string(),
                description: Some("Write code".to_string()),
                tags: vec!["dev".to_string()],
                examples: vec![],
            }],
            security_schemes: vec![],
            default_input_modes: vec!["application/json".to_string()],
            default_output_modes: vec!["application/json".to_string()],
        };

        let json = serde_json::to_string_pretty(&card).unwrap();
        assert!(json.contains("\"pushNotifications\""));
        let parsed: A2AStandardAgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "test-swarm");
        assert!(parsed.capabilities.streaming);
    }

    #[test]
    fn test_jsonrpc_request() {
        let req = A2AJsonRpcRequest::new(
            "tasks/send",
            serde_json::json!({"message": {"role": "user", "parts": []}}),
        );
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "tasks/send");
        assert!(req.params.is_some());
    }
}

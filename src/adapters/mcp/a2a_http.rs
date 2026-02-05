//! A2A (Agent-to-Agent) HTTP Gateway.
//!
//! Implements JSON-RPC 2.0 over HTTP for agent-to-agent communication
//! following the A2A protocol specification.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::domain::models::a2a::{A2AAgentCard, A2AMessage};

/// A2A-specific JSON-RPC error codes.
#[derive(Debug, Clone, Copy)]
pub enum A2AErrorCode {
    /// Task not found.
    TaskNotFound = -32001,
    /// Task cannot be canceled in current state.
    TaskNotCancelable = -32002,
    /// Agent doesn't support webhooks.
    PushNotificationNotSupported = -32003,
    /// Requested operation not supported.
    UnsupportedOperation = -32004,
    /// Unsupported content/MIME type.
    ContentTypeNotSupported = -32005,
    /// Agent returned malformed response.
    InvalidAgentResponse = -32006,
    /// Agent not found.
    AgentNotFound = -32007,
    /// Invalid request parameters.
    InvalidParams = -32602,
    /// Method not found.
    MethodNotFound = -32601,
    /// Parse error.
    ParseError = -32700,
    /// Internal error.
    InternalError = -32603,
}

impl A2AErrorCode {
    pub fn code(&self) -> i32 {
        *self as i32
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::TaskNotFound => "Task not found",
            Self::TaskNotCancelable => "Task cannot be canceled",
            Self::PushNotificationNotSupported => "Push notifications not supported",
            Self::UnsupportedOperation => "Operation not supported",
            Self::ContentTypeNotSupported => "Content type not supported",
            Self::InvalidAgentResponse => "Invalid agent response",
            Self::AgentNotFound => "Agent not found",
            Self::InvalidParams => "Invalid params",
            Self::MethodNotFound => "Method not found",
            Self::ParseError => "Parse error",
            Self::InternalError => "Internal error",
        }
    }
}

/// A2A task state matching the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum A2ATaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Failed,
    Canceled,
}

impl std::fmt::Display for A2ATaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Submitted => write!(f, "submitted"),
            Self::Working => write!(f, "working"),
            Self::InputRequired => write!(f, "input-required"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Canceled => write!(f, "canceled"),
        }
    }
}

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: A2AErrorCode, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: code.code(),
                message: code.message().to_string(),
                data,
            }),
        }
    }
}

/// JSON-RPC 2.0 error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Message part for multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MessagePart {
    Text {
        text: String,
    },
    Data {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: Value,
    },
    File {
        #[serde(rename = "mimeType")]
        mime_type: String,
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Binary {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: String, // base64 encoded
    },
}

/// A2A protocol message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AProtocolMessage {
    pub role: String,
    pub parts: Vec<MessagePart>,
}

/// Task send parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskSendParams {
    pub id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub message: A2AProtocolMessage,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Task get parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskGetParams {
    pub id: String,
    #[serde(rename = "historyLength")]
    pub history_length: Option<u32>,
}

/// Task cancel parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskCancelParams {
    pub id: String,
}

/// Push notification config parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct PushNotificationConfigParams {
    pub id: String,
    #[serde(rename = "pushNotificationConfig")]
    pub config: PushNotificationConfig,
}

/// Push notification config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushNotificationConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
}

/// Artifact structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AArtifact {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub parts: Vec<MessagePart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub append: bool,
    #[serde(default = "default_true")]
    #[serde(rename = "lastChunk")]
    pub last_chunk: bool,
}

fn default_true() -> bool {
    true
}

/// A2A Task response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATask {
    pub id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub status: A2ATaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<A2AProtocolMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<A2AArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// A2A Task status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATaskStatus {
    pub state: A2ATaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<A2AProtocolMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Skill definition for agent cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ASkill {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

/// Extended agent card with A2A protocol fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AExtendedAgentCard {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<A2AProvider>,
    pub capabilities: A2ACapabilities,
    pub skills: Vec<A2ASkill>,
    #[serde(rename = "defaultInputModes")]
    pub default_input_modes: Vec<String>,
    #[serde(rename = "defaultOutputModes")]
    pub default_output_modes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<A2AAuthentication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abathur: Option<A2AAbathurExtension>,
}

/// Provider info in agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AProvider {
    pub organization: String,
    #[serde(rename = "contactEmail")]
    pub contact_email: Option<String>,
}

/// Capabilities in agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ACapabilities {
    pub streaming: bool,
    #[serde(rename = "pushNotifications")]
    pub push_notifications: bool,
    #[serde(rename = "stateTransitionHistory")]
    pub state_transition_history: bool,
}

/// Authentication info in agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AAuthentication {
    pub schemes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

/// Abathur-specific extensions for agent cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AAbathurExtension {
    pub tier: String,
    #[serde(rename = "agentType")]
    pub agent_type: String,
    #[serde(rename = "maxTurns")]
    pub max_turns: u32,
    #[serde(rename = "allowedTools")]
    pub allowed_tools: Vec<String>,
    #[serde(rename = "handoffTargets")]
    pub handoff_targets: Vec<String>,
    #[serde(rename = "autonomyLevel")]
    pub autonomy_level: String,
}

/// Configuration for the A2A HTTP gateway.
#[derive(Debug, Clone)]
pub struct A2AHttpConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Whether to enable CORS.
    pub enable_cors: bool,
    /// Whether to enable streaming.
    pub enable_streaming: bool,
    /// Whether to enable push notifications.
    pub enable_push_notifications: bool,
    /// Heartbeat interval for SSE streams (milliseconds).
    pub heartbeat_interval_ms: u64,
    /// Maximum stream duration (seconds).
    pub max_stream_duration_s: u64,
}

impl Default for A2AHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            enable_cors: true,
            enable_streaming: true,
            enable_push_notifications: true,
            heartbeat_interval_ms: 30000,
            max_stream_duration_s: 3600,
        }
    }
}

/// In-memory A2A task.
#[derive(Debug, Clone)]
pub struct InMemoryTask {
    pub id: String,
    pub session_id: String,
    pub state: A2ATaskState,
    pub history: Vec<A2AProtocolMessage>,
    pub artifacts: Vec<A2AArtifact>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub push_config: Option<PushNotificationConfig>,
}

/// In-memory session for multi-turn conversations.
#[derive(Debug, Clone)]
pub struct A2ASession {
    pub id: String,
    pub agent_id: String,
    pub task_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
}

/// Shared state for the A2A HTTP gateway.
/// A pending delegation request for orchestrator consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDelegation {
    pub id: Uuid,
    pub sender_id: String,
    pub target_agent: String,
    pub task_description: String,
    pub parent_task_id: Option<Uuid>,
    pub goal_id: Option<Uuid>,
    pub priority: String,
    pub created_at: DateTime<Utc>,
    pub acknowledged: bool,
}

pub struct A2AState {
    /// Registered agent cards.
    pub agent_cards: RwLock<HashMap<String, A2AAgentCard>>,
    /// In-memory tasks.
    pub tasks: RwLock<HashMap<String, InMemoryTask>>,
    /// Sessions.
    pub sessions: RwLock<HashMap<String, A2ASession>>,
    /// Messages between agents.
    pub messages: RwLock<HashMap<Uuid, A2AMessage>>,
    /// Pending delegations for orchestrator consumption.
    pub delegations: RwLock<HashMap<Uuid, PendingDelegation>>,
    /// Configuration.
    pub config: A2AHttpConfig,
}

impl A2AState {
    pub fn new(config: A2AHttpConfig) -> Self {
        Self {
            agent_cards: RwLock::new(HashMap::new()),
            tasks: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            messages: RwLock::new(HashMap::new()),
            delegations: RwLock::new(HashMap::new()),
            config,
        }
    }
}

/// A2A HTTP Gateway.
pub struct A2AHttpGateway {
    config: A2AHttpConfig,
    state: Arc<A2AState>,
}

impl A2AHttpGateway {
    pub fn new(config: A2AHttpConfig) -> Self {
        let state = Arc::new(A2AState::new(config.clone()));
        Self { config, state }
    }

    /// Get a reference to the shared state.
    pub fn state(&self) -> Arc<A2AState> {
        Arc::clone(&self.state)
    }

    /// Register an agent card.
    pub async fn register_agent(&self, card: A2AAgentCard) {
        let mut cards = self.state.agent_cards.write().await;
        cards.insert(card.agent_id.clone(), card);
    }

    /// Register multiple agent cards.
    pub async fn register_agents(&self, cards: Vec<A2AAgentCard>) {
        let mut agent_cards = self.state.agent_cards.write().await;
        for card in cards {
            agent_cards.insert(card.agent_id.clone(), card);
        }
    }

    /// Build the router.
    fn build_router(self) -> Router {
        let state = Arc::clone(&self.state);

        let app = Router::new()
            // JSON-RPC endpoint
            .route("/", post(handle_jsonrpc))
            .route("/rpc", post(handle_jsonrpc))
            // Agent discovery endpoints
            .route("/agents", get(list_agents))
            .route("/agents/{agent_id}", get(get_agent))
            .route("/agents/{agent_id}/card", get(get_agent_card))
            .route("/agents/{agent_id}/skills", get(get_agent_skills))
            // Task endpoints (REST alternative)
            .route("/tasks", post(create_task))
            .route("/tasks/{task_id}", get(get_task))
            .route("/tasks/{task_id}/cancel", post(cancel_task))
            .route("/tasks/{task_id}/stream", get(stream_task))
            // Health check
            .route("/health", get(health_check))
            // Delegation endpoints (for orchestrator integration)
            .route("/api/v1/delegations", post(create_delegation))
            .route("/api/v1/delegations/pending", get(list_pending_delegations))
            .route("/api/v1/delegations/{delegation_id}/ack", post(acknowledge_delegation))
            .with_state(state);

        if self.config.enable_cors {
            app.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .layer(TraceLayer::new_for_http())
        } else {
            app.layer(TraceLayer::new_for_http())
        }
    }

    /// Start the gateway.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("A2A HTTP Gateway listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Start the gateway with a shutdown signal.
    pub async fn serve_with_shutdown<F>(
        self,
        shutdown: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("A2A HTTP Gateway listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }
}

// Handler functions

async fn health_check() -> &'static str {
    "OK"
}

/// Request to create a delegation.
#[derive(Debug, Clone, Deserialize)]
struct CreateDelegationRequest {
    sender_id: String,
    target_agent: String,
    task_description: String,
    parent_task_id: Option<Uuid>,
    goal_id: Option<Uuid>,
    #[serde(default = "default_priority")]
    priority: String,
}

fn default_priority() -> String {
    "normal".to_string()
}

/// Create a new delegation request.
async fn create_delegation(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<CreateDelegationRequest>,
) -> Result<Json<PendingDelegation>, (StatusCode, String)> {
    let delegation = PendingDelegation {
        id: Uuid::new_v4(),
        sender_id: request.sender_id,
        target_agent: request.target_agent,
        task_description: request.task_description,
        parent_task_id: request.parent_task_id,
        goal_id: request.goal_id,
        priority: request.priority,
        created_at: Utc::now(),
        acknowledged: false,
    };

    let mut delegations = state.delegations.write().await;
    let id = delegation.id;
    delegations.insert(id, delegation.clone());

    Ok(Json(delegation))
}

/// List pending (unacknowledged) delegations.
async fn list_pending_delegations(
    State(state): State<Arc<A2AState>>,
) -> Json<Vec<PendingDelegation>> {
    let delegations = state.delegations.read().await;
    let pending: Vec<PendingDelegation> = delegations
        .values()
        .filter(|d| !d.acknowledged)
        .cloned()
        .collect();
    Json(pending)
}

/// Acknowledge a delegation (mark as processed).
async fn acknowledge_delegation(
    State(state): State<Arc<A2AState>>,
    Path(delegation_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut delegations = state.delegations.write().await;
    if let Some(delegation) = delegations.get_mut(&delegation_id) {
        delegation.acknowledged = true;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "Delegation not found".to_string()))
    }
}

async fn handle_jsonrpc(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    if request.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::ParseError,
            Some(json!({"message": "Invalid JSON-RPC version"})),
        ));
    }

    match request.method.as_str() {
        "tasks/send" => handle_tasks_send(state, request).await,
        "tasks/get" => handle_tasks_get(state, request).await,
        "tasks/cancel" => handle_tasks_cancel(state, request).await,
        "tasks/pushNotificationConfig/set" => handle_push_notification_config(state, request).await,
        "agent/card" => handle_agent_card(state, request).await,
        "agent/skills" => handle_agent_skills(state, request).await,
        _ => Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::MethodNotFound,
            Some(json!({"method": request.method})),
        )),
    }
}

async fn handle_tasks_send(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: TaskSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ))
        }
    };

    let task_id = params.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let session_id = params
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let now = Utc::now();

    // Check if task exists (continue existing task)
    let mut tasks = state.tasks.write().await;
    let task = if let Some(existing) = tasks.get_mut(&task_id) {
        existing.history.push(params.message);
        existing.state = A2ATaskState::Working;
        existing.updated_at = now;
        existing.clone()
    } else {
        // Create new task
        let new_task = InMemoryTask {
            id: task_id.clone(),
            session_id: session_id.clone(),
            state: A2ATaskState::Submitted,
            history: vec![params.message],
            artifacts: vec![],
            metadata: params.metadata,
            created_at: now,
            updated_at: now,
            push_config: None,
        };
        tasks.insert(task_id.clone(), new_task.clone());

        // Update session
        drop(tasks);
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            if !session.task_ids.contains(&task_id) {
                session.task_ids.push(task_id.clone());
            }
            session.last_activity = now;
        } else {
            sessions.insert(
                session_id.clone(),
                A2ASession {
                    id: session_id.clone(),
                    agent_id: String::new(),
                    task_ids: vec![task_id.clone()],
                    created_at: now,
                    last_activity: now,
                },
            );
        }

        new_task
    };

    let response = A2ATask {
        id: task.id,
        session_id: task.session_id,
        status: A2ATaskStatus {
            state: task.state,
            message: None,
            timestamp: Some(task.updated_at.to_rfc3339()),
        },
        history: Some(task.history),
        artifacts: if task.artifacts.is_empty() {
            None
        } else {
            Some(task.artifacts)
        },
        metadata: task.metadata,
    };

    Json(JsonRpcResponse::success(
        request.id,
        serde_json::to_value(response).unwrap(),
    ))
}

async fn handle_tasks_get(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: TaskGetParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ))
        }
    };

    let tasks = state.tasks.read().await;
    let task = match tasks.get(&params.id) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotFound,
                Some(json!({"taskId": params.id})),
            ))
        }
    };

    let history = if let Some(len) = params.history_length {
        let start = task.history.len().saturating_sub(len as usize);
        Some(task.history[start..].to_vec())
    } else {
        Some(task.history.clone())
    };

    let response = A2ATask {
        id: task.id.clone(),
        session_id: task.session_id.clone(),
        status: A2ATaskStatus {
            state: task.state,
            message: None,
            timestamp: Some(task.updated_at.to_rfc3339()),
        },
        history,
        artifacts: if task.artifacts.is_empty() {
            None
        } else {
            Some(task.artifacts.clone())
        },
        metadata: task.metadata.clone(),
    };

    Json(JsonRpcResponse::success(
        request.id,
        serde_json::to_value(response).unwrap(),
    ))
}

async fn handle_tasks_cancel(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: TaskCancelParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ))
        }
    };

    let mut tasks = state.tasks.write().await;
    let task = match tasks.get_mut(&params.id) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotFound,
                Some(json!({"taskId": params.id})),
            ))
        }
    };

    // Check if task can be canceled
    match task.state {
        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotCancelable,
                Some(json!({
                    "taskId": params.id,
                    "currentState": task.state.to_string()
                })),
            ))
        }
        _ => {}
    }

    task.state = A2ATaskState::Canceled;
    task.updated_at = Utc::now();

    let response = A2ATask {
        id: task.id.clone(),
        session_id: task.session_id.clone(),
        status: A2ATaskStatus {
            state: task.state,
            message: None,
            timestamp: Some(task.updated_at.to_rfc3339()),
        },
        history: None,
        artifacts: None,
        metadata: task.metadata.clone(),
    };

    Json(JsonRpcResponse::success(
        request.id,
        serde_json::to_value(response).unwrap(),
    ))
}

async fn handle_push_notification_config(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    if !state.config.enable_push_notifications {
        return Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::PushNotificationNotSupported,
            None,
        ));
    }

    let params: PushNotificationConfigParams = match serde_json::from_value(request.params.clone())
    {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ))
        }
    };

    let mut tasks = state.tasks.write().await;
    let task = match tasks.get_mut(&params.id) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotFound,
                Some(json!({"taskId": params.id})),
            ))
        }
    };

    task.push_config = Some(params.config.clone());

    Json(JsonRpcResponse::success(
        request.id,
        json!({"taskId": params.id, "configured": true}),
    ))
}

async fn handle_agent_card(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    // Get agent_id from params if provided
    let agent_id: Option<String> = serde_json::from_value(request.params.clone())
        .ok()
        .and_then(|v: Value| v.get("agentId").and_then(|a| a.as_str()).map(String::from));

    let cards = state.agent_cards.read().await;

    if let Some(id) = agent_id {
        match cards.get(&id) {
            Some(card) => Json(JsonRpcResponse::success(
                request.id,
                serde_json::to_value(card).unwrap(),
            )),
            None => Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::AgentNotFound,
                Some(json!({"agentId": id})),
            )),
        }
    } else {
        // Return first available agent card
        match cards.values().next() {
            Some(card) => Json(JsonRpcResponse::success(
                request.id,
                serde_json::to_value(card).unwrap(),
            )),
            None => Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::AgentNotFound,
                Some(json!({"message": "No agents registered"})),
            )),
        }
    }
}

async fn handle_agent_skills(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let agent_id: Option<String> = serde_json::from_value(request.params.clone())
        .ok()
        .and_then(|v: Value| v.get("agentId").and_then(|a| a.as_str()).map(String::from));

    let cards = state.agent_cards.read().await;

    let card = if let Some(id) = &agent_id {
        cards.get(id)
    } else {
        cards.values().next()
    };

    match card {
        Some(c) => {
            // Convert capabilities to skills
            let skills: Vec<A2ASkill> = c
                .capabilities
                .iter()
                .map(|cap| A2ASkill {
                    id: cap.to_lowercase().replace(' ', "-"),
                    name: cap.clone(),
                    description: None,
                    tags: vec![],
                    examples: vec![],
                })
                .collect();

            Json(JsonRpcResponse::success(
                request.id,
                serde_json::to_value(skills).unwrap(),
            ))
        }
        None => Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::AgentNotFound,
            Some(json!({"agentId": agent_id})),
        )),
    }
}

// REST endpoints for agent discovery

async fn list_agents(
    State(state): State<Arc<A2AState>>,
) -> Result<Json<Vec<A2AAgentCard>>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    Ok(Json(cards.values().cloned().collect()))
}

async fn get_agent(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<A2AAgentCard>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    match cards.get(&agent_id) {
        Some(card) => Ok(Json(card.clone())),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Agent {} not found", agent_id),
                code: "AGENT_NOT_FOUND".to_string(),
            }),
        )),
    }
}

async fn get_agent_card(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<A2AAgentCard>, (StatusCode, Json<ErrorResponse>)> {
    get_agent(State(state), Path(agent_id)).await
}

async fn get_agent_skills(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<A2ASkill>>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    match cards.get(&agent_id) {
        Some(card) => {
            let skills: Vec<A2ASkill> = card
                .capabilities
                .iter()
                .map(|cap| A2ASkill {
                    id: cap.to_lowercase().replace(' ', "-"),
                    name: cap.clone(),
                    description: None,
                    tags: vec![],
                    examples: vec![],
                })
                .collect();
            Ok(Json(skills))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Agent {} not found", agent_id),
                code: "AGENT_NOT_FOUND".to_string(),
            }),
        )),
    }
}

// REST endpoints for task operations

async fn create_task(
    State(state): State<Arc<A2AState>>,
    Json(params): Json<TaskSendParams>,
) -> Result<(StatusCode, Json<A2ATask>), (StatusCode, Json<ErrorResponse>)> {
    let task_id = params.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let session_id = params
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let now = Utc::now();

    let new_task = InMemoryTask {
        id: task_id.clone(),
        session_id: session_id.clone(),
        state: A2ATaskState::Submitted,
        history: vec![params.message],
        artifacts: vec![],
        metadata: params.metadata,
        created_at: now,
        updated_at: now,
        push_config: None,
    };

    let mut tasks = state.tasks.write().await;
    tasks.insert(task_id.clone(), new_task.clone());

    let response = A2ATask {
        id: new_task.id,
        session_id: new_task.session_id,
        status: A2ATaskStatus {
            state: new_task.state,
            message: None,
            timestamp: Some(new_task.created_at.to_rfc3339()),
        },
        history: Some(new_task.history),
        artifacts: None,
        metadata: new_task.metadata,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

async fn get_task(
    State(state): State<Arc<A2AState>>,
    Path(task_id): Path<String>,
) -> Result<Json<A2ATask>, (StatusCode, Json<ErrorResponse>)> {
    let tasks = state.tasks.read().await;
    match tasks.get(&task_id) {
        Some(task) => Ok(Json(A2ATask {
            id: task.id.clone(),
            session_id: task.session_id.clone(),
            status: A2ATaskStatus {
                state: task.state,
                message: None,
                timestamp: Some(task.updated_at.to_rfc3339()),
            },
            history: Some(task.history.clone()),
            artifacts: if task.artifacts.is_empty() {
                None
            } else {
                Some(task.artifacts.clone())
            },
            metadata: task.metadata.clone(),
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Task {} not found", task_id),
                code: "TASK_NOT_FOUND".to_string(),
            }),
        )),
    }
}

async fn cancel_task(
    State(state): State<Arc<A2AState>>,
    Path(task_id): Path<String>,
) -> Result<Json<A2ATask>, (StatusCode, Json<ErrorResponse>)> {
    let mut tasks = state.tasks.write().await;
    let task = match tasks.get_mut(&task_id) {
        Some(t) => t,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Task {} not found", task_id),
                    code: "TASK_NOT_FOUND".to_string(),
                }),
            ))
        }
    };

    match task.state {
        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: format!("Task {} cannot be canceled in state {}", task_id, task.state),
                    code: "TASK_NOT_CANCELABLE".to_string(),
                }),
            ))
        }
        _ => {}
    }

    task.state = A2ATaskState::Canceled;
    task.updated_at = Utc::now();

    Ok(Json(A2ATask {
        id: task.id.clone(),
        session_id: task.session_id.clone(),
        status: A2ATaskStatus {
            state: task.state,
            message: None,
            timestamp: Some(task.updated_at.to_rfc3339()),
        },
        history: None,
        artifacts: None,
        metadata: task.metadata.clone(),
    }))
}

/// Stream task updates via SSE.
async fn stream_task(
    State(state): State<Arc<A2AState>>,
    Path(task_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    if !state.config.enable_streaming {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Streaming not enabled".to_string(),
                code: "STREAMING_NOT_ENABLED".to_string(),
            }),
        ));
    }

    // Verify task exists
    {
        let tasks = state.tasks.read().await;
        if !tasks.contains_key(&task_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Task {} not found", task_id),
                    code: "TASK_NOT_FOUND".to_string(),
                }),
            ));
        }
    }

    let heartbeat_interval = Duration::from_millis(state.config.heartbeat_interval_ms);
    let state_clone = Arc::clone(&state);
    let task_id_clone = task_id.clone();

    // Create a stream that periodically checks task state
    let stream = stream::unfold(
        (state_clone, task_id_clone, None::<A2ATaskState>),
        |(state, task_id, last_state)| async move {
            // Wait a bit before next update
            tokio::time::sleep(Duration::from_millis(500)).await;

            let tasks = state.tasks.read().await;
            if let Some(task) = tasks.get(&task_id) {
                // Only send event if state changed
                if last_state != Some(task.state) {
                    let event = json!({
                        "type": "TaskStatusUpdate",
                        "taskId": task.id,
                        "status": {
                            "state": task.state,
                            "timestamp": task.updated_at.to_rfc3339()
                        }
                    });

                    let sse_event = Event::default()
                        .event("TaskStatusUpdate")
                        .data(event.to_string());

                    // If terminal state, end stream
                    let is_terminal = matches!(
                        task.state,
                        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled
                    );

                    if is_terminal {
                        return Some((Ok(sse_event), (state.clone(), task_id, Some(task.state))));
                    }

                    return Some((Ok(sse_event), (state.clone(), task_id, Some(task.state))));
                }

                // Send heartbeat if no state change
                Some((
                    Ok(Event::default().comment("heartbeat")),
                    (state.clone(), task_id, last_state),
                ))
            } else {
                None
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(heartbeat_interval)))
}

/// Error response for REST endpoints.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// ============================================================================
// Federation Client — Outbound cross-swarm task delegation
// ============================================================================

use crate::services::{A2AFederationConfig, TrustedSwarmConfig};

/// Client for delegating tasks to trusted peer swarms via A2A protocol.
pub struct FederationClient {
    config: A2AFederationConfig,
    http_client: reqwest::Client,
    /// Per-peer request counters for rate limiting (peer_id -> count in current window).
    request_counts: Arc<RwLock<HashMap<String, u32>>>,
}

impl FederationClient {
    pub fn new(config: A2AFederationConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.external_request_timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            config,
            http_client,
            request_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// List available trusted peers that are active.
    pub fn list_available_peers(&self) -> Vec<&TrustedSwarmConfig> {
        self.config.trusted_swarms.iter()
            .filter(|s| s.active)
            .collect()
    }

    /// Get a peer's agent card / capabilities.
    pub async fn get_peer_capabilities(&self, peer_id: &str) -> Result<A2AAgentCard, String> {
        let peer = self.find_peer(peer_id)?;

        let url = format!("{}/.well-known/agent.json", peer.endpoint.trim_end_matches('/'));

        let mut request = self.http_client.get(&url);
        if let Some(ref token) = peer.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await
            .map_err(|e| format!("Failed to reach peer {}: {}", peer_id, e))?;

        if !response.status().is_success() {
            return Err(format!("Peer {} returned status {}", peer_id, response.status()));
        }

        let card: A2AAgentCard = response.json().await
            .map_err(|e| format!("Failed to parse peer {} agent card: {}", peer_id, e))?;

        Ok(card)
    }

    /// Delegate a task to a trusted peer swarm.
    pub async fn delegate_task(
        &self,
        peer_id: &str,
        message: &str,
    ) -> Result<A2ATask, String> {
        let peer = self.find_peer(peer_id)?;

        // Rate limiting
        {
            let mut counts = self.request_counts.write().await;
            let count = counts.entry(peer_id.to_string()).or_insert(0);
            let limit = peer.rate_limit_override.unwrap_or(self.config.rate_limit_per_swarm);
            if *count >= limit {
                return Err(format!("Rate limit exceeded for peer {}", peer_id));
            }
            *count += 1;
        }

        let task_id = Uuid::new_v4().to_string();
        let session_id = Uuid::new_v4().to_string();

        let json_rpc = json!({
            "jsonrpc": "2.0",
            "id": task_id,
            "method": "tasks/send",
            "params": {
                "id": task_id,
                "sessionId": session_id,
                "message": {
                    "role": "user",
                    "parts": [
                        {
                            "type": "text",
                            "text": message,
                        }
                    ]
                }
            }
        });

        let url = format!("{}/a2a", peer.endpoint.trim_end_matches('/'));

        let mut request = self.http_client.post(&url)
            .header("Content-Type", "application/json")
            .json(&json_rpc);

        if let Some(ref token) = peer.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await
            .map_err(|e| format!("Failed to send task to peer {}: {}", peer_id, e))?;

        if !response.status().is_success() {
            return Err(format!("Peer {} returned status {}", peer_id, response.status()));
        }

        let rpc_response: JsonRpcResponse = response.json().await
            .map_err(|e| format!("Failed to parse response from peer {}: {}", peer_id, e))?;

        if let Some(error) = rpc_response.error {
            return Err(format!("Peer {} error: {} ({})", peer_id, error.message, error.code));
        }

        let result = rpc_response.result
            .ok_or_else(|| format!("Peer {} returned no result", peer_id))?;

        let task: A2ATask = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse task from peer {}: {}", peer_id, e))?;

        Ok(task)
    }

    /// Reset rate limit counters (should be called periodically).
    pub async fn reset_rate_limits(&self) {
        self.request_counts.write().await.clear();
    }

    fn find_peer(&self, peer_id: &str) -> Result<&TrustedSwarmConfig, String> {
        self.config.trusted_swarms.iter()
            .find(|s| s.id == peer_id && s.active)
            .ok_or_else(|| format!("Peer {} not found or inactive", peer_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = A2AHttpConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert!(config.enable_cors);
        assert!(config.enable_streaming);
    }

    #[test]
    fn test_jsonrpc_request_parsing() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/send",
            "params": {"message": {"role": "user", "parts": [{"type": "text", "text": "Hello"}]}}
        }"#;
        let request: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.method, "tasks/send");
    }

    #[test]
    fn test_jsonrpc_response_success() {
        let response = JsonRpcResponse::success(Some(json!(1)), json!({"result": "ok"}));
        assert!(response.error.is_none());
        assert!(response.result.is_some());
    }

    #[test]
    fn test_jsonrpc_response_error() {
        let response = JsonRpcResponse::error(Some(json!(1)), A2AErrorCode::TaskNotFound, None);
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32001);
    }

    #[test]
    fn test_task_state_display() {
        assert_eq!(A2ATaskState::Submitted.to_string(), "submitted");
        assert_eq!(A2ATaskState::Working.to_string(), "working");
        assert_eq!(A2ATaskState::Completed.to_string(), "completed");
    }

    #[test]
    fn test_message_part_serialization() {
        let part = MessagePart::Text {
            text: "Hello".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello\""));
    }

    #[test]
    fn test_task_send_params_parsing() {
        let json = r#"{
            "message": {
                "role": "user",
                "parts": [{"type": "text", "text": "Do something"}]
            }
        }"#;
        let params: TaskSendParams = serde_json::from_str(json).unwrap();
        assert!(params.id.is_none());
        assert_eq!(params.message.role, "user");
    }

    #[tokio::test]
    async fn test_state_management() {
        let state = A2AState::new(A2AHttpConfig::default());

        // Register agent
        let card = A2AAgentCard::new("test-agent")
            .with_display_name("Test Agent")
            .with_capability("testing");
        state
            .agent_cards
            .write()
            .await
            .insert("test-agent".to_string(), card);

        let cards = state.agent_cards.read().await;
        assert!(cards.contains_key("test-agent"));
    }

    #[tokio::test]
    async fn test_task_creation() {
        let state = Arc::new(A2AState::new(A2AHttpConfig::default()));

        let task = InMemoryTask {
            id: "task-1".to_string(),
            session_id: "session-1".to_string(),
            state: A2ATaskState::Submitted,
            history: vec![],
            artifacts: vec![],
            metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            push_config: None,
        };

        state
            .tasks
            .write()
            .await
            .insert("task-1".to_string(), task);

        let tasks = state.tasks.read().await;
        assert!(tasks.contains_key("task-1"));
        assert_eq!(tasks.get("task-1").unwrap().state, A2ATaskState::Submitted);
    }
}

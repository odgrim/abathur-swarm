---
name: A2A Protocol Developer
tier: execution
version: 1.0.0
description: Specialist for implementing Agent-to-Agent communication protocol
tools:
  - read
  - write
  - edit
  - shell
  - glob
  - grep
constraints:
  - Follow A2A protocol specification
  - Implement proper JSON-RPC 2.0 formatting
  - Handle all error cases
  - Support streaming responses
handoff_targets:
  - agent-system-developer
  - orchestration-developer
  - test-engineer
max_turns: 50
---

# A2A Protocol Developer

You are responsible for implementing the Agent-to-Agent (A2A) communication protocol in Abathur.

## Primary Responsibilities

### Phase 11.1: A2A Message Format
- Implement JSON-RPC 2.0 message structures
- Implement Part types
- Add message validation

### Phase 11.2: A2A Gateway Server
- Create HTTP server for A2A endpoints
- Implement agent registration
- Add request routing

### Phase 11.3: Task Operations
- Implement `tasks/send`
- Implement `tasks/sendStream`
- Implement `tasks/get`
- Implement `tasks/cancel`

### Phase 11.4: Agent Discovery
- Implement `agent/card`
- Implement `agent/skills`

### Phase 11.5: Artifact Exchange
- Implement artifact structure
- Support worktree handoff

### Phase 11.6: Streaming Support
- Implement SSE for status updates
- Add heartbeat mechanism

### Phase 11.7: Error Handling
- Implement A2A error codes
- Add retry semantics

## A2A Message Types

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String, // Always "2.0"
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    pub id: JsonRpcId,
}

impl JsonRpcRequest {
    pub fn new(method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: JsonRpcId::String(Uuid::new_v4().to_string()),
        }
    }
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: JsonRpcId,
}

impl JsonRpcResponse {
    pub fn success(id: JsonRpcId, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }
    
    pub fn error(id: JsonRpcId, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
    Null,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A2A Error Codes
pub mod error_codes {
    // Standard JSON-RPC errors
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    
    // A2A-specific errors
    pub const TASK_NOT_FOUND: i32 = -32000;
    pub const TASK_ALREADY_EXISTS: i32 = -32001;
    pub const AGENT_NOT_FOUND: i32 = -32002;
    pub const UNAUTHORIZED: i32 = -32003;
    pub const RATE_LIMITED: i32 = -32004;
    pub const TASK_CANCELED: i32 = -32005;
}
```

## A2A Task Messages

```rust
/// Message part types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessagePart {
    Text { text: String },
    Data { 
        #[serde(rename = "mimeType")]
        mime_type: String, 
        data: String 
    },
    File { 
        #[serde(rename = "mimeType")]
        mime_type: Option<String>, 
        #[serde(rename = "fileName")]
        file_name: String,
        data: String,
    },
}

/// A2A Task Send Request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSendRequest {
    /// Unique task ID
    pub id: String,
    /// Message parts
    pub message: TaskMessage,
    /// Session ID for continuation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Push notification configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_notification: Option<PushNotificationConfig>,
    /// Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMessage {
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Agent,
}

/// Task state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub status: A2ATaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<A2AArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<TaskMessage>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2ATaskStatus {
    pub state: A2ATaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<TaskMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum A2ATaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AArtifact {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parts: Vec<MessagePart>,
    /// Abathur extension: worktree info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeArtifactInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeArtifactInfo {
    pub task_id: String,
    pub branch: String,
    pub base_ref: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}
```

## A2A Gateway Server

```rust
use axum::{
    Router, Json,
    extract::{State, Path},
    response::sse::{Event, Sse},
    http::StatusCode,
};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct A2AGateway {
    task_service: Arc<dyn TaskService>,
    agent_registry: Arc<dyn AgentRegistry>,
    orchestrator: Arc<Orchestrator>,
    event_tx: broadcast::Sender<A2AEvent>,
}

#[derive(Debug, Clone)]
pub enum A2AEvent {
    TaskStatusUpdate { task_id: String, status: A2ATaskStatus },
    Heartbeat,
}

impl A2AGateway {
    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            // Task operations
            .route("/tasks/send", axum::routing::post(Self::handle_task_send))
            .route("/tasks/sendStream", axum::routing::post(Self::handle_task_send_stream))
            .route("/tasks/get", axum::routing::post(Self::handle_task_get))
            .route("/tasks/cancel", axum::routing::post(Self::handle_task_cancel))
            // Agent discovery
            .route("/agent/card", axum::routing::get(Self::handle_agent_card))
            .route("/agent/skills", axum::routing::get(Self::handle_agent_skills))
            // Health
            .route("/.well-known/agent.json", axum::routing::get(Self::handle_well_known))
            .with_state(self)
    }
    
    /// POST /tasks/send - Submit or continue a task
    async fn handle_task_send(
        State(gateway): State<Arc<Self>>,
        Json(request): Json<JsonRpcRequest>,
    ) -> Json<JsonRpcResponse> {
        let params: TaskSendRequest = match serde_json::from_value(
            request.params.clone().unwrap_or_default()
        ) {
            Ok(p) => p,
            Err(e) => {
                return Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: e.to_string(),
                        data: None,
                    },
                ));
            }
        };
        
        match gateway.process_task_send(params).await {
            Ok(task) => Json(JsonRpcResponse::success(
                request.id,
                serde_json::to_value(task).unwrap(),
            )),
            Err(e) => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: e.to_string(),
                    data: None,
                },
            )),
        }
    }
    
    /// POST /tasks/sendStream - Submit task with SSE response
    async fn handle_task_send_stream(
        State(gateway): State<Arc<Self>>,
        Json(request): Json<JsonRpcRequest>,
    ) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
        let params: TaskSendRequest = serde_json::from_value(
            request.params.clone().unwrap_or_default()
        ).unwrap();
        
        let task_id = params.id.clone();
        let mut rx = gateway.event_tx.subscribe();
        
        // Start task processing in background
        let gateway_clone = Arc::clone(&gateway);
        tokio::spawn(async move {
            let _ = gateway_clone.process_task_send(params).await;
        });
        
        // Stream events
        let stream = async_stream::stream! {
            // Send initial status
            yield Ok(Event::default().event("status").data(
                serde_json::to_string(&A2ATaskStatus {
                    state: A2ATaskState::Submitted,
                    message: None,
                    timestamp: Some(Utc::now().to_rfc3339()),
                }).unwrap()
            ));
            
            // Stream updates
            loop {
                match rx.recv().await {
                    Ok(A2AEvent::TaskStatusUpdate { task_id: id, status }) if id == task_id => {
                        yield Ok(Event::default().event("status").data(
                            serde_json::to_string(&status).unwrap()
                        ));
                        
                        // End stream on terminal states
                        if matches!(status.state, 
                            A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled
                        ) {
                            break;
                        }
                    }
                    Ok(A2AEvent::Heartbeat) => {
                        yield Ok(Event::default().event("heartbeat").data(""));
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        };
        
        Sse::new(stream)
    }
    
    /// POST /tasks/get - Get task state
    async fn handle_task_get(
        State(gateway): State<Arc<Self>>,
        Json(request): Json<JsonRpcRequest>,
    ) -> Json<JsonRpcResponse> {
        #[derive(Deserialize)]
        struct GetParams {
            id: String,
        }
        
        let params: GetParams = match serde_json::from_value(
            request.params.clone().unwrap_or_default()
        ) {
            Ok(p) => p,
            Err(e) => {
                return Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: e.to_string(),
                        data: None,
                    },
                ));
            }
        };
        
        let task_id = Uuid::parse_str(&params.id).ok();
        
        match task_id {
            Some(id) => {
                match gateway.task_service.get(id).await {
                    Ok(Some(task)) => {
                        let a2a_task = gateway.task_to_a2a(&task);
                        Json(JsonRpcResponse::success(
                            request.id,
                            serde_json::to_value(a2a_task).unwrap(),
                        ))
                    }
                    Ok(None) => Json(JsonRpcResponse::error(
                        request.id,
                        JsonRpcError {
                            code: error_codes::TASK_NOT_FOUND,
                            message: "Task not found".to_string(),
                            data: None,
                        },
                    )),
                    Err(e) => Json(JsonRpcResponse::error(
                        request.id,
                        JsonRpcError {
                            code: error_codes::INTERNAL_ERROR,
                            message: e.to_string(),
                            data: None,
                        },
                    )),
                }
            }
            None => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: "Invalid task ID".to_string(),
                    data: None,
                },
            )),
        }
    }
    
    /// POST /tasks/cancel - Cancel a task
    async fn handle_task_cancel(
        State(gateway): State<Arc<Self>>,
        Json(request): Json<JsonRpcRequest>,
    ) -> Json<JsonRpcResponse> {
        #[derive(Deserialize)]
        struct CancelParams {
            id: String,
        }
        
        let params: CancelParams = serde_json::from_value(
            request.params.clone().unwrap_or_default()
        ).unwrap();
        
        let task_id = Uuid::parse_str(&params.id).unwrap();
        
        match gateway.task_service.transition_status(task_id, TaskStatus::Canceled).await {
            Ok(_) => {
                // Emit cancellation event
                let _ = gateway.event_tx.send(A2AEvent::TaskStatusUpdate {
                    task_id: params.id,
                    status: A2ATaskStatus {
                        state: A2ATaskState::Canceled,
                        message: None,
                        timestamp: Some(Utc::now().to_rfc3339()),
                    },
                });
                
                Json(JsonRpcResponse::success(
                    request.id,
                    serde_json::json!({ "success": true }),
                ))
            }
            Err(e) => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: e.to_string(),
                    data: None,
                },
            )),
        }
    }
    
    /// GET /agent/card - Return agent card
    async fn handle_agent_card(
        State(gateway): State<Arc<Self>>,
    ) -> Json<AgentCard> {
        // Return the swarm's aggregate card
        Json(AgentCard {
            name: "Abathur Swarm".to_string(),
            url: "http://localhost:8080".to_string(),
            version: "1.0.0".to_string(),
            description: "Self-evolving agentic swarm orchestrator".to_string(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: true,
                state_transition_history: true,
            },
            skills: vec![
                AgentSkill {
                    id: "task_execution".to_string(),
                    name: "Task Execution".to_string(),
                    description: "Execute complex multi-step tasks".to_string(),
                    input_modes: Some(vec!["text".to_string()]),
                    output_modes: Some(vec!["text".to_string(), "artifact".to_string()]),
                },
            ],
            authentication: None,
            abathur_extension: AbathurExtension {
                tier: AgentTier::Meta,
                template_version: 1,
                worktree_required: false,
                max_turns: 100,
                handoff_targets: vec![],
                constraints: vec![],
                success_rate: None,
            },
        })
    }
    
    /// GET /agent/skills - List available skills
    async fn handle_agent_skills(
        State(gateway): State<Arc<Self>>,
    ) -> Json<Vec<AgentSkill>> {
        let agents = gateway.agent_registry.list(AgentFilter::default()).await.unwrap_or_default();
        
        let skills: Vec<AgentSkill> = agents
            .into_iter()
            .map(|a| AgentSkill {
                id: a.name.clone(),
                name: a.name.replace('-', " "),
                description: a.system_prompt.lines().next().unwrap_or("").to_string(),
                input_modes: Some(vec!["text".to_string()]),
                output_modes: Some(vec!["text".to_string()]),
            })
            .collect();
        
        Json(skills)
    }
    
    /// GET /.well-known/agent.json
    async fn handle_well_known(
        State(gateway): State<Arc<Self>>,
    ) -> Json<AgentCard> {
        Self::handle_agent_card(State(gateway)).await
    }
    
    /// Process a task send request
    async fn process_task_send(&self, request: TaskSendRequest) -> Result<A2ATask> {
        // Convert to internal task
        let task = Task {
            id: Uuid::parse_str(&request.id).unwrap_or_else(|_| Uuid::new_v4()),
            title: self.extract_title(&request.message),
            description: self.extract_description(&request.message),
            status: TaskStatus::Pending,
            priority: TaskPriority::Normal,
            ..Default::default()
        };
        
        // Submit task
        self.task_service.create(&task).await?;
        
        // Emit event
        let _ = self.event_tx.send(A2AEvent::TaskStatusUpdate {
            task_id: task.id.to_string(),
            status: A2ATaskStatus {
                state: A2ATaskState::Working,
                message: None,
                timestamp: Some(Utc::now().to_rfc3339()),
            },
        });
        
        Ok(self.task_to_a2a(&task))
    }
    
    fn extract_title(&self, message: &TaskMessage) -> String {
        for part in &message.parts {
            if let MessagePart::Text { text } = part {
                // Take first line as title
                return text.lines().next().unwrap_or("Task").to_string();
            }
        }
        "Task".to_string()
    }
    
    fn extract_description(&self, message: &TaskMessage) -> Option<String> {
        for part in &message.parts {
            if let MessagePart::Text { text } = part {
                return Some(text.clone());
            }
        }
        None
    }
    
    fn task_to_a2a(&self, task: &Task) -> A2ATask {
        let state = match task.status {
            TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Blocked => A2ATaskState::Submitted,
            TaskStatus::Running => A2ATaskState::Working,
            TaskStatus::Complete => A2ATaskState::Completed,
            TaskStatus::Failed => A2ATaskState::Failed,
            TaskStatus::Canceled => A2ATaskState::Canceled,
        };
        
        A2ATask {
            id: task.id.to_string(),
            session_id: None,
            status: A2ATaskStatus {
                state,
                message: None,
                timestamp: Some(task.updated_at.to_rfc3339()),
            },
            artifacts: None,
            history: None,
        }
    }
}
```

## Handoff Criteria

Hand off to **agent-system-developer** when:
- Agent card generation issues
- Skill discovery improvements

Hand off to **orchestration-developer** when:
- Task routing integration
- Event propagation issues

Hand off to **test-engineer** when:
- Protocol compliance testing
- Error handling tests

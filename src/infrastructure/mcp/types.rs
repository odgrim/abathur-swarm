//! MCP HTTP server types
//!
//! JSON-RPC 2.0 and MCP-specific request/response types

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl IntoResponse for JsonRpcResponse {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

// ============================================================================
// Memory Request Types
// ============================================================================

/// Request parameters for adding a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAddRequest {
    pub namespace: String,
    pub key: String,
    pub value: Value,
    pub memory_type: String,
    pub created_by: String,
}

/// Request parameters for getting a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryGetRequest {
    pub namespace: String,
    pub key: String,
}

/// Request parameters for searching memories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchRequest {
    pub namespace_prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// Request parameters for updating a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdateRequest {
    pub namespace: String,
    pub key: String,
    pub value: Value,
    pub updated_by: String,
}

/// Request parameters for deleting a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDeleteRequest {
    pub namespace: String,
    pub key: String,
}

// ============================================================================
// Task Request Types
// ============================================================================

/// Request parameters for enqueuing a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEnqueueRequest {
    pub summary: String,
    pub description: String,
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(default = "default_dependency_type")]
    pub dependency_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
}

/// Request parameters for getting a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGetRequest {
    pub task_id: String,
}

/// Request parameters for listing tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskListRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default = "default_list_limit")]
    pub limit: usize,
}

/// Request parameters for canceling a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCancelRequest {
    pub task_id: String,
}

/// Request parameters for execution plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecutionPlanRequest {
    #[serde(default)]
    pub include_completed: bool,
}

// ============================================================================
// Default value functions
// ============================================================================

fn default_limit() -> usize {
    50
}

fn default_agent_type() -> String {
    "requirements-gatherer".to_string()
}

fn default_priority() -> u8 {
    5
}

fn default_dependency_type() -> String {
    "sequential".to_string()
}

fn default_list_limit() -> usize {
    50
}

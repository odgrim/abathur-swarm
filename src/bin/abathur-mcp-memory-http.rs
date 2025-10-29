//! HTTP MCP server for Abathur memory management
//!
//! This MCP server exposes memory management operations via HTTP transport.
//! It listens on port 45678 and handles JSON-RPC requests.

use abathur_cli::{
    domain::models::{Memory, MemoryType},
    infrastructure::database::{connection::DatabaseConnection, memory_repo::MemoryRepositoryImpl},
    services::MemoryService,
};
use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "abathur-mcp-memory-http")]
#[command(about = "HTTP MCP server for Abathur memory management")]
struct Args {
    /// Path to SQLite database file
    #[arg(long, default_value = ".abathur/abathur.db")]
    db_path: String,

    /// Port to listen on
    #[arg(long, default_value = "45678")]
    port: u16,
}

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl IntoResponse for JsonRpcResponse {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

/// Application state
#[derive(Clone)]
struct AppState {
    memory_service: Arc<MemoryService>,
}

/// Request parameters for adding a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryAddRequest {
    namespace: String,
    key: String,
    value: Value,
    memory_type: String,
    created_by: String,
}

/// Request parameters for getting a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryGetRequest {
    namespace: String,
    key: String,
}

/// Request parameters for searching memories
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemorySearchRequest {
    namespace_prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Request parameters for updating a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryUpdateRequest {
    namespace: String,
    key: String,
    value: Value,
    updated_by: String,
}

/// Request parameters for deleting a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryDeleteRequest {
    namespace: String,
    key: String,
}

async fn handle_request(
    State(state): State<AppState>,
    Json(request): Json<JsonRpcRequest>,
) -> JsonRpcResponse {
    debug!("Received request: method={}", request.method);
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => handle_initialize(id),
        "tools/list" => handle_list_tools(id),
        "tools/call" => handle_tool_call(state, request).await,
        _ => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        },
    }
}

fn handle_initialize(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "abathur-memory",
                "version": "1.0.0"
            }
        })),
        error: None,
    }
}

fn handle_list_tools(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(json!({
            "tools": [
                {
                    "name": "memory_add",
                    "description": "Add a new memory entry to the memory system",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "Hierarchical namespace" },
                            "key": { "type": "string", "description": "Unique key within namespace" },
                            "value": { "description": "JSON value to store" },
                            "memory_type": { "type": "string", "description": "Type (semantic/episodic/procedural)" },
                            "created_by": { "type": "string", "description": "Creator identifier" }
                        },
                        "required": ["namespace", "key", "value", "memory_type", "created_by"]
                    }
                },
                {
                    "name": "memory_get",
                    "description": "Get memory by namespace and key",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string" },
                            "key": { "type": "string" }
                        },
                        "required": ["namespace", "key"]
                    }
                },
                {
                    "name": "memory_search",
                    "description": "Search memories by namespace prefix and optional type filter",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace_prefix": { "type": "string" },
                            "memory_type": { "type": "string" },
                            "limit": { "type": "integer", "default": 50 }
                        },
                        "required": ["namespace_prefix"]
                    }
                },
                {
                    "name": "memory_update",
                    "description": "Update an existing memory value",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string" },
                            "key": { "type": "string" },
                            "value": {},
                            "updated_by": { "type": "string" }
                        },
                        "required": ["namespace", "key", "value", "updated_by"]
                    }
                },
                {
                    "name": "memory_delete",
                    "description": "Soft delete a memory entry",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string" },
                            "key": { "type": "string" }
                        },
                        "required": ["namespace", "key"]
                    }
                }
            ]
        })),
        error: None,
    }
}

async fn handle_tool_call(state: AppState, request: JsonRpcRequest) -> JsonRpcResponse {
    let id = request.id.clone();

    let params = match request.params {
        Some(p) => p,
        None => {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32600,
                    message: "Missing params".to_string(),
                    data: None,
                }),
            }
        }
    };

    let tool_name = match params.get("name") {
        Some(Value::String(name)) => name,
        _ => {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32600,
                    message: "Missing tool name".to_string(),
                    data: None,
                }),
            }
        }
    };

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match tool_name.as_str() {
        "memory_add" => memory_add(&state.memory_service, arguments).await,
        "memory_get" => memory_get(&state.memory_service, arguments).await,
        "memory_search" => memory_search(&state.memory_service, arguments).await,
        "memory_update" => memory_update(&state.memory_service, arguments).await,
        "memory_delete" => memory_delete(&state.memory_service, arguments).await,
        _ => Err(format!("Unknown tool: {}", tool_name)),
    };

    match result {
        Ok(content) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(json!({
                "content": [
                    {
                        "type": "text",
                        "text": content
                    }
                ]
            })),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        },
    }
}

async fn memory_add(service: &MemoryService, arguments: Value) -> Result<String, String> {
    let params: MemoryAddRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let memory_type: MemoryType = params
        .memory_type
        .parse()
        .map_err(|e| format!("Invalid memory_type: {}", e))?;

    let memory = Memory::new(
        params.namespace,
        params.key,
        params.value,
        memory_type,
        params.created_by,
    );

    service
        .add(memory)
        .await
        .map(|id| {
            info!("Memory added successfully with ID: {}", id);
            format!("Memory added successfully with ID: {}", id)
        })
        .map_err(|e| {
            error!("Failed to add memory: {}", e);
            format!("Failed to add memory: {}", e)
        })
}

async fn memory_get(service: &MemoryService, arguments: Value) -> Result<String, String> {
    let params: MemoryGetRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    service
        .get(&params.namespace, &params.key)
        .await
        .map_err(|e| {
            error!("Failed to get memory: {}", e);
            format!("Failed to get memory: {}", e)
        })?
        .map(|memory| {
            info!("Memory found: {}", memory.namespace_path());
            serde_json::to_string_pretty(&memory)
                .unwrap_or_else(|_| "Error serializing memory".to_string())
        })
        .ok_or_else(|| format!("Memory not found: {}:{}", params.namespace, params.key))
}

async fn memory_search(service: &MemoryService, arguments: Value) -> Result<String, String> {
    let params: MemorySearchRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let memory_type: Option<MemoryType> = params.memory_type.as_ref().and_then(|s| s.parse().ok());

    service
        .search(&params.namespace_prefix, memory_type, Some(params.limit))
        .await
        .map(|memories| {
            info!("Found {} memories", memories.len());
            json!({
                "count": memories.len(),
                "memories": memories
            })
            .to_string()
        })
        .map_err(|e| {
            error!("Failed to search memories: {}", e);
            format!("Failed to search memories: {}", e)
        })
}

async fn memory_update(service: &MemoryService, arguments: Value) -> Result<String, String> {
    let params: MemoryUpdateRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    service
        .update(
            &params.namespace,
            &params.key,
            params.value,
            &params.updated_by,
        )
        .await
        .map(|_| {
            info!(
                "Memory updated successfully: {}:{}",
                params.namespace, params.key
            );
            format!(
                "Memory updated successfully: {}:{}",
                params.namespace, params.key
            )
        })
        .map_err(|e| {
            error!("Failed to update memory: {}", e);
            format!("Failed to update memory: {}", e)
        })
}

async fn memory_delete(service: &MemoryService, arguments: Value) -> Result<String, String> {
    let params: MemoryDeleteRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    service
        .delete(&params.namespace, &params.key)
        .await
        .map(|_| {
            info!(
                "Memory deleted successfully: {}:{}",
                params.namespace, params.key
            );
            format!(
                "Memory deleted successfully: {}:{}",
                params.namespace, params.key
            )
        })
        .map_err(|e| {
            error!("Failed to delete memory: {}", e);
            format!("Failed to delete memory: {}", e)
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    info!("Starting Abathur Memory HTTP MCP server");
    info!("Database path: {}", args.db_path);
    info!("Port: {}", args.port);

    let database_url = format!("sqlite:{}", args.db_path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to connect to database")?;

    db.migrate()
        .await
        .context("Failed to run database migrations")?;

    let memory_repo = Arc::new(MemoryRepositoryImpl::new(db.pool().clone()));
    let memory_service = Arc::new(MemoryService::new(memory_repo));

    info!("Database initialized successfully");

    let state = AppState { memory_service };

    let app = Router::new()
        .route("/", post(handle_request))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("HTTP MCP server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

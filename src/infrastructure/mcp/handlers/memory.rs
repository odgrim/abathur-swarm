//! Memory MCP tool handlers
//!
//! Implements handlers for memory-related MCP tools

use crate::domain::models::{Memory, MemoryType};
use crate::infrastructure::mcp::types::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, MemoryAddRequest, MemoryDeleteRequest,
    MemoryGetRequest, MemorySearchRequest, MemoryUpdateRequest,
};
use crate::services::{MemoryService, RagService};
use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info};

/// Application state for memory server
#[derive(Clone)]
pub struct MemoryAppState {
    pub memory_service: Arc<MemoryService>,
    pub rag_service: Option<Arc<RagService>>,
}

pub async fn handle_memory_request(
    State(state): State<MemoryAppState>,
    Json(request): Json<JsonRpcRequest>,
) -> JsonRpcResponse {
    debug!("Received request: method={}", request.method);
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => handle_memory_initialize(id),
        "tools/list" => handle_memory_list_tools(id),
        "tools/call" => handle_memory_tool_call(state, request).await,
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

fn handle_memory_initialize(id: Option<Value>) -> JsonRpcResponse {
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

fn handle_memory_list_tools(id: Option<Value>) -> JsonRpcResponse {
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
                },
                {
                    "name": "vector_add_document",
                    "description": "Add a document to vector storage with automatic chunking and embedding for semantic search",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "namespace": { "type": "string", "description": "Namespace for organizing the document (e.g., 'docs:api', 'agent:memory')" },
                            "content": { "type": "string", "description": "The document content to index" },
                            "citation_source": { "type": "string", "description": "Optional source citation (file path or URL)" }
                        },
                        "required": ["namespace", "content"]
                    }
                },
                {
                    "name": "vector_search",
                    "description": "Semantic search across vector memories using natural language queries",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "The search query in natural language" },
                            "limit": { "type": "integer", "default": 10, "description": "Maximum number of results to return" },
                            "namespace_filter": { "type": "string", "description": "Optional namespace prefix to filter results (e.g., 'docs:')" }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "vector_list_namespaces",
                    "description": "List all vector memory namespaces with their document counts",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        })),
        error: None,
    }
}

async fn handle_memory_tool_call(state: MemoryAppState, request: JsonRpcRequest) -> JsonRpcResponse {
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
        "vector_add_document" => vector_add_document(&state, arguments).await,
        "vector_search" => vector_search(&state, arguments).await,
        "vector_list_namespaces" => vector_list_namespaces(&state).await,
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

// Vector search tool handlers

async fn vector_add_document(state: &MemoryAppState, arguments: Value) -> Result<String, String> {
    use crate::domain::models::Citation;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct VectorAddDocumentRequest {
        namespace: String,
        content: String,
        citation_source: Option<String>,
    }

    let rag_service = match &state.rag_service {
        Some(service) => service,
        None => {
            return Err("RAG service not available. Vector search is not enabled.".to_string());
        }
    };

    let params: VectorAddDocumentRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let citation = params.citation_source.map(Citation::from_file);

    rag_service
        .add_document(&params.namespace, &params.content, citation)
        .await
        .map(|chunk_ids| {
            info!(
                "Document added to vector storage: namespace={}, chunks={}",
                params.namespace,
                chunk_ids.len()
            );
            format!(
                "Document added successfully. Created {} chunks in namespace '{}'",
                chunk_ids.len(),
                params.namespace
            )
        })
        .map_err(|e| {
            error!("Failed to add document to vector storage: {}", e);
            format!("Failed to add document: {}", e)
        })
}

async fn vector_search(state: &MemoryAppState, arguments: Value) -> Result<String, String> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct VectorSearchRequest {
        query: String,
        #[serde(default = "default_search_limit")]
        limit: usize,
        namespace_filter: Option<String>,
    }

    fn default_search_limit() -> usize {
        10
    }

    let rag_service = match &state.rag_service {
        Some(service) => service,
        None => {
            return Err("RAG service not available. Vector search is not enabled.".to_string());
        }
    };

    let params: VectorSearchRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let namespace_filter = params.namespace_filter.as_deref();

    rag_service
        .retrieve_context(&params.query, params.limit, namespace_filter)
        .await
        .map(|results| {
            info!(
                "Vector search completed: query='{}', results={}",
                params.query,
                results.len()
            );

            if results.is_empty() {
                return "No results found.".to_string();
            }

            // Format results as a readable string
            let mut output = format!("Found {} results:\n\n", results.len());
            for (i, result) in results.iter().enumerate() {
                output.push_str(&format!(
                    "{}. [Score: {:.2}] {} ({})\n{}\n\n",
                    i + 1,
                    result.score,
                    result.id,
                    result.namespace,
                    result.content
                ));
            }

            output
        })
        .map_err(|e| {
            error!("Vector search failed: {}", e);
            format!("Search failed: {}", e)
        })
}

async fn vector_list_namespaces(state: &MemoryAppState) -> Result<String, String> {
    let rag_service = match &state.rag_service {
        Some(service) => service,
        None => {
            return Err("RAG service not available. Vector search is not enabled.".to_string());
        }
    };

    rag_service
        .list_namespaces()
        .await
        .map(|namespaces| {
            info!("Listed {} vector namespaces", namespaces.len());

            if namespaces.is_empty() {
                return "No vector namespaces found.".to_string();
            }

            let mut output = format!("Vector Memory Namespaces ({} total):\n\n", namespaces.len());
            for (namespace, count) in namespaces {
                output.push_str(&format!("- {}: {} documents\n", namespace, count));
            }

            output
        })
        .map_err(|e| {
            error!("Failed to list namespaces: {}", e);
            format!("Failed to list namespaces: {}", e)
        })
}

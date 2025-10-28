//! Direct MCP Client Implementation
//!
//! Provides MCP tool functionality by directly calling underlying services
//! without spawning separate MCP server processes. This is used by internal
//! agents within the swarm for efficient, in-process communication.
//!
//! For external clients (IDEs, Claude Code), use stdio MCP servers instead.

use crate::domain::ports::{
    McpClient, McpError, McpToolRequest, McpToolResponse, ResourceContent, ResourceInfo, ToolInfo,
};
use crate::services::{MemoryService, TaskQueueService};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error};

/// Direct MCP client that uses services in-process
///
/// This implementation provides MCP tool functionality by directly calling
/// the underlying memory and task queue services. It's designed for use by
/// internal agents within the swarm to avoid the overhead of spawning
/// separate MCP server processes for each agent.
///
/// # Design Rationale
///
/// **Why not spawn MCP servers?**
/// - With hundreds of agents, spawning separate MCP server processes would:
///   - Create hundreds of processes
///   - Create hundreds of database connections
///   - Introduce stdio communication overhead
///   - Risk resource exhaustion
///
/// **Direct service access instead:**
/// - Single shared service instances (Arc)
/// - Efficient in-process calls
/// - Shared database connection pool
/// - Thread-safe via Arc + async
///
/// # Usage
///
/// ```ignore
/// let memory_service = Arc::new(MemoryService::new(repo));
/// let task_service = Arc::new(TaskQueueService::new(repo, resolver, calc));
///
/// let mcp_client: Arc<dyn McpClient> = Arc::new(
///     DirectMcpClient::new(memory_service, task_service)
/// );
///
/// // Agents use the MCP client as normal
/// let response = mcp_client.call_tool(
///     "abathur-memory",
///     "memory_add",
///     json!({"namespace": "test", "key": "foo", "value": "bar"})
/// ).await?;
/// ```
pub struct DirectMcpClient {
    memory_service: Arc<MemoryService>,
    task_service: Arc<TaskQueueService>,
}

impl DirectMcpClient {
    /// Create a new direct MCP client
    ///
    /// # Arguments
    ///
    /// * `memory_service` - Shared memory service instance
    /// * `task_service` - Shared task queue service instance
    pub fn new(memory_service: Arc<MemoryService>, task_service: Arc<TaskQueueService>) -> Self {
        Self {
            memory_service,
            task_service,
        }
    }

    /// Route tool call to appropriate service
    async fn route_tool_call(
        &self,
        server: &str,
        tool: &str,
        args: Value,
    ) -> Result<Value, McpError> {
        match server {
            "abathur-memory" => self.handle_memory_tool(tool, args).await,
            "abathur-task-queue" | "abathur-tasks" => self.handle_task_tool(tool, args).await,
            _ => Err(McpError::ServerNotFound(server.to_string())),
        }
    }

    /// Handle memory service tool calls
    async fn handle_memory_tool(&self, tool: &str, args: Value) -> Result<Value, McpError> {
        debug!(tool, "Handling memory tool");

        match tool {
            "memory_add" => {
                let namespace = args["namespace"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing namespace".to_string()))?;
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing key".to_string()))?;
                let value = args["value"].clone();
                let memory_type = args["memory_type"]
                    .as_str()
                    .unwrap_or("semantic")
                    .parse()
                    .map_err(|e| McpError::InvalidArguments(format!("Invalid memory_type: {}", e)))?;
                let created_by = args["created_by"]
                    .as_str()
                    .unwrap_or("agent");

                let memory = crate::domain::models::Memory::new(
                    namespace.to_string(),
                    key.to_string(),
                    value,
                    memory_type,
                    created_by.to_string(),
                );

                self.memory_service
                    .add(memory)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({"success": true}))
            }

            "memory_get" => {
                let namespace = args["namespace"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing namespace".to_string()))?;
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing key".to_string()))?;

                let memory = self
                    .memory_service
                    .get(namespace, key)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?
                    .ok_or_else(|| {
                        McpError::ResourceNotFound(format!("{}:{}", namespace, key))
                    })?;

                Ok(serde_json::to_value(memory)
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?)
            }

            "memory_search" => {
                let namespace_prefix = args["namespace_prefix"]
                    .as_str()
                    .ok_or_else(|| {
                        McpError::InvalidArguments("Missing namespace_prefix".to_string())
                    })?;
                let memory_type = args["memory_type"]
                    .as_str()
                    .and_then(|s| s.parse().ok());
                let limit = args["limit"].as_u64().map(|n| n as usize);

                let memories = self
                    .memory_service
                    .search(namespace_prefix, memory_type, limit)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({
                    "count": memories.len(),
                    "memories": memories
                }))
            }

            "memory_update" => {
                let namespace = args["namespace"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing namespace".to_string()))?;
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing key".to_string()))?;
                let value = args["value"].clone();
                let updated_by = args["updated_by"].as_str().unwrap_or("agent");

                self.memory_service
                    .update(namespace, key, value, updated_by)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({"success": true}))
            }

            "memory_delete" => {
                let namespace = args["namespace"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing namespace".to_string()))?;
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing key".to_string()))?;

                self.memory_service
                    .delete(namespace, key)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({"success": true}))
            }

            _ => Err(McpError::ToolNotFound(tool.to_string())),
        }
    }

    /// Handle task queue service tool calls
    async fn handle_task_tool(&self, tool: &str, args: Value) -> Result<Value, McpError> {
        debug!(tool, "Handling task tool");

        match tool {
            "task_enqueue" => {
                let summary = args["summary"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing summary".to_string()))?;
                let description = args["description"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing description".to_string()))?;

                let mut task = crate::domain::models::Task::new(
                    summary.to_string(),
                    description.to_string(),
                );

                if let Some(agent_type) = args["agent_type"].as_str() {
                    task.agent_type = agent_type.to_string();
                }

                if let Some(priority) = args["priority"].as_u64() {
                    task.priority = priority as u8;
                }

                let task_id = self
                    .task_service
                    .submit(task)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({
                    "task_id": task_id,
                    "success": true
                }))
            }

            "task_get" => {
                let task_id = args["task_id"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing task_id".to_string()))?
                    .parse()
                    .map_err(|_| McpError::InvalidArguments("Invalid task_id UUID".to_string()))?;

                let task = self
                    .task_service
                    .get(task_id)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?
                    .ok_or_else(|| McpError::ResourceNotFound(format!("Task {}", task_id)))?;

                Ok(serde_json::to_value(task)
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?)
            }

            "task_list" => {
                let filters = crate::domain::ports::TaskFilters {
                    status: args["status"].as_str().and_then(|s| s.parse().ok()),
                    agent_type: args["agent_type"].as_str().map(String::from),
                    limit: args["limit"].as_u64().map(|n| n as usize),
                    ..Default::default()
                };

                let tasks = self
                    .task_service
                    .list(filters)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({
                    "count": tasks.len(),
                    "tasks": tasks
                }))
            }

            "task_queue_status" => {
                let statuses = vec![
                    crate::domain::models::TaskStatus::Pending,
                    crate::domain::models::TaskStatus::Blocked,
                    crate::domain::models::TaskStatus::Ready,
                    crate::domain::models::TaskStatus::Running,
                    crate::domain::models::TaskStatus::Completed,
                    crate::domain::models::TaskStatus::Failed,
                    crate::domain::models::TaskStatus::Cancelled,
                ];

                let mut status_counts = serde_json::Map::new();
                for status in statuses {
                    let filters = crate::domain::ports::TaskFilters {
                        status: Some(status),
                        ..Default::default()
                    };
                    if let Ok(count) = self.task_service.count(filters).await {
                        status_counts.insert(status.to_string(), json!(count));
                    }
                }

                Ok(json!({
                    "status_counts": status_counts
                }))
            }

            "task_cancel" => {
                let task_id = args["task_id"]
                    .as_str()
                    .ok_or_else(|| McpError::InvalidArguments("Missing task_id".to_string()))?
                    .parse()
                    .map_err(|_| McpError::InvalidArguments("Invalid task_id UUID".to_string()))?;

                self.task_service
                    .cancel(task_id)
                    .await
                    .map_err(|e| McpError::ExecutionFailed(e.to_string()))?;

                Ok(json!({"success": true}))
            }

            _ => Err(McpError::ToolNotFound(tool.to_string())),
        }
    }
}

#[async_trait]
impl McpClient for DirectMcpClient {
    async fn invoke_tool(&self, request: McpToolRequest) -> Result<McpToolResponse, McpError> {
        let result = self
            .route_tool_call(&request.server_name, &request.tool_name, request.arguments)
            .await;

        match result {
            Ok(value) => Ok(McpToolResponse {
                task_id: request.task_id,
                result: value,
                is_error: false,
            }),
            Err(e) => {
                error!(
                    server = %request.server_name,
                    tool = %request.tool_name,
                    error = ?e,
                    "Tool execution failed"
                );
                Ok(McpToolResponse {
                    task_id: request.task_id,
                    result: json!({ "error": e.to_string() }),
                    is_error: true,
                })
            }
        }
    }

    async fn call_tool(&self, server: &str, tool: &str, args: Value) -> Result<Value, McpError> {
        self.route_tool_call(server, tool, args).await
    }

    async fn list_tools(&self, server: &str) -> Result<Vec<ToolInfo>, McpError> {
        match server {
            "abathur-memory" => Ok(vec![
                ToolInfo {
                    name: "memory_add".to_string(),
                    description: Some("Add a new memory entry".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"},
                            "value": {},
                            "memory_type": {"type": "string", "enum": ["semantic", "episodic", "procedural"]},
                            "created_by": {"type": "string"}
                        },
                        "required": ["namespace", "key", "value"]
                    }),
                },
                ToolInfo {
                    name: "memory_get".to_string(),
                    description: Some("Get memory by namespace and key".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"}
                        },
                        "required": ["namespace", "key"]
                    }),
                },
                ToolInfo {
                    name: "memory_search".to_string(),
                    description: Some("Search memories by namespace prefix".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "namespace_prefix": {"type": "string"},
                            "memory_type": {"type": "string"},
                            "limit": {"type": "integer"}
                        },
                        "required": ["namespace_prefix"]
                    }),
                },
                ToolInfo {
                    name: "memory_update".to_string(),
                    description: Some("Update an existing memory".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"},
                            "value": {},
                            "updated_by": {"type": "string"}
                        },
                        "required": ["namespace", "key", "value"]
                    }),
                },
                ToolInfo {
                    name: "memory_delete".to_string(),
                    description: Some("Delete a memory entry".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "namespace": {"type": "string"},
                            "key": {"type": "string"}
                        },
                        "required": ["namespace", "key"]
                    }),
                },
            ]),
            "abathur-task-queue" | "abathur-tasks" => Ok(vec![
                ToolInfo {
                    name: "task_enqueue".to_string(),
                    description: Some("Enqueue a new task".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "summary": {"type": "string"},
                            "description": {"type": "string"},
                            "agent_type": {"type": "string"},
                            "priority": {"type": "integer", "minimum": 0, "maximum": 10}
                        },
                        "required": ["summary", "description"]
                    }),
                },
                ToolInfo {
                    name: "task_get".to_string(),
                    description: Some("Get task by ID".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "task_id": {"type": "string", "format": "uuid"}
                        },
                        "required": ["task_id"]
                    }),
                },
                ToolInfo {
                    name: "task_list".to_string(),
                    description: Some("List tasks with filters".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "status": {"type": "string"},
                            "agent_type": {"type": "string"},
                            "limit": {"type": "integer"}
                        }
                    }),
                },
                ToolInfo {
                    name: "task_queue_status".to_string(),
                    description: Some("Get queue statistics".to_string()),
                    input_schema: json!({"type": "object"}),
                },
                ToolInfo {
                    name: "task_cancel".to_string(),
                    description: Some("Cancel a task".to_string()),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "task_id": {"type": "string", "format": "uuid"}
                        },
                        "required": ["task_id"]
                    }),
                },
            ]),
            _ => Err(McpError::ServerNotFound(server.to_string())),
        }
    }

    async fn read_resource(
        &self,
        _server: &str,
        _uri: &str,
    ) -> Result<ResourceContent, McpError> {
        // Resources not implemented for direct client
        Err(McpError::ExecutionFailed(
            "Resources not supported by DirectMcpClient".to_string(),
        ))
    }

    async fn list_resources(&self, _server: &str) -> Result<Vec<ResourceInfo>, McpError> {
        // Resources not implemented for direct client
        Ok(vec![])
    }

    async fn health_check(&self, server_name: &str) -> Result<(), McpError> {
        match server_name {
            "abathur-memory" | "abathur-task-queue" | "abathur-tasks" => Ok(()),
            _ => Err(McpError::ServerNotFound(server_name.to_string())),
        }
    }
}

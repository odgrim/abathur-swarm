//! Task queue MCP tool handlers
//!
//! Implements handlers for task-related MCP tools

use crate::domain::models::{DependencyType, Task, TaskStatus};
use crate::domain::ports::TaskFilters;
use crate::infrastructure::mcp::types::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, TaskCancelRequest, TaskEnqueueRequest,
    TaskExecutionPlanRequest, TaskGetRequest, TaskListRequest,
};
use crate::services::TaskQueueService;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Application state for tasks server
#[derive(Clone)]
pub struct TasksAppState {
    pub task_service: Arc<TaskQueueService>,
}

pub async fn handle_tasks_request(
    State(state): State<TasksAppState>,
    Json(request): Json<JsonRpcRequest>,
) -> JsonRpcResponse {
    debug!("Received request: method={}", request.method);
    let id = request.id.clone();

    match request.method.as_str() {
        "initialize" => handle_tasks_initialize(id),
        "tools/list" => handle_tasks_list_tools(id),
        "tools/call" => handle_tasks_tool_call(state, request).await,
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

fn handle_tasks_initialize(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "abathur-task-queue",
                "version": "1.0.0"
            }
        })),
        error: None,
    }
}

fn handle_tasks_list_tools(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(json!({
            "tools": [
                {
                    "name": "task_enqueue",
                    "description": "Enqueue a new task to the task queue",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "summary": { "type": "string", "description": "Brief task summary (max 140 chars)" },
                            "description": { "type": "string", "description": "Detailed task description" },
                            "agent_type": { "type": "string", "default": "requirements-gatherer" },
                            "priority": { "type": "integer", "default": 5, "minimum": 0, "maximum": 10 },
                            "dependencies": { "type": "array", "items": { "type": "string" } },
                            "dependency_type": { "type": "string", "default": "sequential" },
                            "parent_task_id": { "type": "string" },
                            "chain_id": { "type": "string", "description": "Prompt chain ID to execute task through multi-step workflow" }
                        },
                        "required": ["summary", "description"]
                    }
                },
                {
                    "name": "task_get",
                    "description": "Get task details by ID",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "task_id": { "type": "string" }
                        },
                        "required": ["task_id"]
                    }
                },
                {
                    "name": "task_list",
                    "description": "List tasks with optional filters",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "status": { "type": "string" },
                            "agent_type": { "type": "string" },
                            "limit": { "type": "integer", "default": 50 }
                        }
                    }
                },
                {
                    "name": "task_queue_status",
                    "description": "Get task queue statistics and status",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "task_cancel",
                    "description": "Cancel a task and cascade cancellation to dependent tasks",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "task_id": { "type": "string" }
                        },
                        "required": ["task_id"]
                    }
                },
                {
                    "name": "task_execution_plan",
                    "description": "Get the execution order of tasks based on dependencies",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "include_completed": { "type": "boolean", "default": false }
                        }
                    }
                }
            ]
        })),
        error: None,
    }
}

async fn handle_tasks_tool_call(state: TasksAppState, request: JsonRpcRequest) -> JsonRpcResponse {
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
        "task_enqueue" => task_enqueue(&state.task_service, arguments).await,
        "task_get" => task_get(&state.task_service, arguments).await,
        "task_list" => task_list(&state.task_service, arguments).await,
        "task_queue_status" => task_queue_status(&state.task_service).await,
        "task_cancel" => task_cancel(&state.task_service, arguments).await,
        "task_execution_plan" => task_execution_plan(&state.task_service, arguments).await,
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

async fn task_enqueue(service: &TaskQueueService, arguments: Value) -> Result<String, String> {
    let params: TaskEnqueueRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let mut task = Task::new(params.summary, params.description);
    task.agent_type = params.agent_type;
    task.priority = params.priority;

    if let Some(deps) = params.dependencies {
        let dependencies: Result<Vec<Uuid>, _> = deps.iter().map(|s| Uuid::parse_str(s)).collect();
        task.dependencies = Some(
            dependencies.map_err(|e| format!("Invalid dependency UUID: {}", e))?,
        );
    }

    task.dependency_type = params
        .dependency_type
        .parse::<DependencyType>()
        .unwrap_or(DependencyType::Sequential);

    if let Some(parent_id_str) = params.parent_task_id {
        let parent_id = Uuid::parse_str(&parent_id_str)
            .map_err(|e| format!("Invalid parent_task_id UUID: {}", e))?;
        task.parent_task_id = Some(parent_id);
    }

    // Set chain_id if provided
    task.chain_id = params.chain_id;

    service
        .submit(task)
        .await
        .map(|task_id| {
            info!("Task enqueued successfully with ID: {}", task_id);
            format!("Task enqueued successfully with ID: {}", task_id)
        })
        .map_err(|e| {
            error!("Failed to enqueue task: {}", e);
            format!("Failed to enqueue task: {}", e)
        })
}

async fn task_get(service: &TaskQueueService, arguments: Value) -> Result<String, String> {
    let params: TaskGetRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let task_id = Uuid::parse_str(&params.task_id)
        .map_err(|e| format!("Invalid task_id UUID: {}", e))?;

    service
        .get(task_id)
        .await
        .map_err(|e| {
            error!("Failed to get task: {}", e);
            format!("Failed to get task: {}", e)
        })?
        .map(|task| {
            info!("Task found: {}", task.summary);
            serde_json::to_string_pretty(&task)
                .unwrap_or_else(|_| "Error serializing task".to_string())
        })
        .ok_or_else(|| format!("Task not found: {}", task_id))
}

async fn task_list(service: &TaskQueueService, arguments: Value) -> Result<String, String> {
    let params: TaskListRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let mut filters = TaskFilters::default();

    if let Some(status_str) = params.status {
        filters.status = status_str.parse::<TaskStatus>().ok();
    }
    if let Some(agent_type) = params.agent_type {
        filters.agent_type = Some(agent_type);
    }
    filters.limit = Some(params.limit);

    service
        .list(filters)
        .await
        .map(|tasks| {
            info!("Found {} tasks", tasks.len());
            json!({
                "count": tasks.len(),
                "tasks": tasks
            })
            .to_string()
        })
        .map_err(|e| {
            error!("Failed to list tasks: {}", e);
            format!("Failed to list tasks: {}", e)
        })
}

async fn task_queue_status(service: &TaskQueueService) -> Result<String, String> {
    let statuses = vec![
        TaskStatus::Pending,
        TaskStatus::Blocked,
        TaskStatus::Ready,
        TaskStatus::Running,
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ];

    let mut status_counts = serde_json::json!({});

    for status in statuses {
        let filters = TaskFilters {
            status: Some(status),
            ..Default::default()
        };
        match service.count(filters).await {
            Ok(count) => {
                status_counts[status.to_string()] = serde_json::json!(count);
            }
            Err(e) => {
                error!("Failed to count tasks for status {:?}: {}", status, e);
            }
        }
    }

    let ready_tasks_count = service
        .get_ready_tasks(Some(10))
        .await
        .ok()
        .map(|t| t.len())
        .unwrap_or(0);

    Ok(json!({
        "status_counts": status_counts,
        "ready_tasks_preview": ready_tasks_count
    })
    .to_string())
}

async fn task_cancel(service: &TaskQueueService, arguments: Value) -> Result<String, String> {
    let params: TaskCancelRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let task_id = Uuid::parse_str(&params.task_id)
        .map_err(|e| format!("Invalid task_id UUID: {}", e))?;

    service
        .cancel(task_id)
        .await
        .map(|_| {
            info!("Task cancelled successfully: {}", task_id);
            format!(
                "Task cancelled successfully (including dependent tasks): {}",
                task_id
            )
        })
        .map_err(|e| {
            error!("Failed to cancel task: {}", e);
            format!("Failed to cancel task: {}", e)
        })
}

async fn task_execution_plan(
    service: &TaskQueueService,
    arguments: Value,
) -> Result<String, String> {
    let params: TaskExecutionPlanRequest =
        serde_json::from_value(arguments).map_err(|e| format!("Invalid parameters: {}", e))?;

    let filters = TaskFilters::default();

    service
        .list(filters)
        .await
        .map(|mut tasks| {
            if !params.include_completed {
                tasks.retain(|t| !t.is_terminal());
            }

            tasks.sort_by(|a, b| {
                a.dependency_depth.cmp(&b.dependency_depth).then_with(|| {
                    b.calculated_priority
                        .partial_cmp(&a.calculated_priority)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });

            let execution_plan: Vec<_> = tasks
                .iter()
                .map(|t| {
                    json!({
                        "task_id": t.id,
                        "summary": t.summary,
                        "status": t.status.to_string(),
                        "dependency_depth": t.dependency_depth,
                        "calculated_priority": t.calculated_priority,
                        "dependencies": t.dependencies
                    })
                })
                .collect();

            info!("Generated execution plan with {} tasks", execution_plan.len());

            json!({
                "count": execution_plan.len(),
                "execution_order": execution_plan
            })
            .to_string()
        })
        .map_err(|e| {
            error!("Failed to generate execution plan: {}", e);
            format!("Failed to generate execution plan: {}", e)
        })
}

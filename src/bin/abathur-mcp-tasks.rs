//! MCP server for Abathur task queue management
//!
//! This MCP server exposes task queue management operations via the Model Context Protocol.
//! It uses stdio transport (stdin/stdout for JSON-RPC communication).
//!
//! # Usage
//!
//! ```bash
//! abathur-mcp-tasks --db-path /path/to/abathur.db
//! ```
//!
//! # Tools Exposed
//!
//! - `task_enqueue` - Enqueue a new task
//! - `task_get` - Get task by ID
//! - `task_list` - List/filter tasks
//! - `task_queue_status` - Get queue statistics
//! - `task_cancel` - Cancel a task
//! - `task_execution_plan` - Get execution order based on dependencies

use abathur_cli::{
    domain::{
        models::{DependencyType, Task, TaskSource, TaskStatus},
        ports::TaskFilters,
    },
    infrastructure::database::{connection::DatabaseConnection, task_repo::TaskRepositoryImpl},
    services::{DependencyResolver, PriorityCalculator, TaskQueueService},
};
use anyhow::{Context, Result};
use clap::Parser;
use rmcp::{
    router::RouterService, CallToolRequestParams, CallToolResult, ListToolsResult, Resource,
    Role, Server, ServerCapabilities, Tool, ToolInputSchema,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "abathur-mcp-tasks")]
#[command(about = "MCP server for Abathur task queue management")]
struct Args {
    /// Path to SQLite database file
    #[arg(long, default_value = ".abathur/abathur.db")]
    db_path: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing to stderr (stdout is reserved for MCP JSON-RPC)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    info!("Starting Abathur Task Queue MCP server");
    info!("Database path: {}", args.db_path);

    // Initialize database connection
    let database_url = format!("sqlite:{}", args.db_path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    db.migrate()
        .await
        .context("Failed to run database migrations")?;

    // Initialize task repository and service
    let task_repo = Arc::new(TaskRepositoryImpl::new(db.pool().clone()));
    let dependency_resolver = DependencyResolver::new();
    let priority_calc = PriorityCalculator::new();
    let task_service = Arc::new(TaskQueueService::new(
        task_repo.clone(),
        dependency_resolver,
        priority_calc,
    ));

    info!("Database initialized successfully");

    // Create MCP server
    let server = Server::new(
        ServerCapabilities {
            tools: Some(json!({
                "supported": true
            })),
            resources: None,
            prompts: None,
        },
        "abathur-tasks".to_string(),
        "1.0.0".to_string(),
    );

    // Create router service
    let router = TaskRouter::new(task_service);

    info!("MCP server ready, listening on stdio");

    // Run server with stdio transport
    server
        .start_with_stdio(router)
        .await
        .context("MCP server failed")?;

    Ok(())
}

/// Router implementation for task queue MCP tools
struct TaskRouter {
    task_service: Arc<TaskQueueService>,
}

impl TaskRouter {
    fn new(task_service: Arc<TaskQueueService>) -> Self {
        Self { task_service }
    }

    /// Define the tools exposed by this MCP server
    fn define_tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "task_enqueue".to_string(),
                description: Some("Enqueue a new task to the task queue".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "summary": {
                            "type": "string",
                            "description": "Brief task summary (max 140 chars)",
                            "maxLength": 140
                        },
                        "description": {
                            "type": "string",
                            "description": "Detailed task description"
                        },
                        "agent_type": {
                            "type": "string",
                            "description": "Type of agent to execute this task",
                            "default": "requirements-gatherer"
                        },
                        "priority": {
                            "type": "integer",
                            "description": "Task priority (0-10)",
                            "minimum": 0,
                            "maximum": 10,
                            "default": 5
                        },
                        "dependencies": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "format": "uuid"
                            },
                            "description": "List of task IDs this task depends on"
                        },
                        "dependency_type": {
                            "type": "string",
                            "enum": ["sequential", "parallel"],
                            "description": "Type of dependency relationship",
                            "default": "sequential"
                        }
                    })),
                    required: Some(vec!["summary".to_string(), "description".to_string()]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "task_get".to_string(),
                description: Some("Get task details by ID".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "task_id": {
                            "type": "string",
                            "format": "uuid",
                            "description": "UUID of the task to retrieve"
                        }
                    })),
                    required: Some(vec!["task_id".to_string()]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "task_list".to_string(),
                description: Some("List tasks with optional filters".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "status": {
                            "type": "string",
                            "enum": ["pending", "blocked", "ready", "running", "completed", "failed", "cancelled"],
                            "description": "Filter by task status"
                        },
                        "agent_type": {
                            "type": "string",
                            "description": "Filter by agent type"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of tasks to return",
                            "default": 50
                        }
                    })),
                    required: None,
                    additional_properties: None,
                },
            },
            Tool {
                name: "task_queue_status".to_string(),
                description: Some("Get task queue statistics and status".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({})),
                    required: None,
                    additional_properties: None,
                },
            },
            Tool {
                name: "task_cancel".to_string(),
                description: Some("Cancel a task and cascade cancellation to dependent tasks".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "task_id": {
                            "type": "string",
                            "format": "uuid",
                            "description": "UUID of the task to cancel"
                        }
                    })),
                    required: Some(vec!["task_id".to_string()]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "task_execution_plan".to_string(),
                description: Some("Get the execution order of tasks based on dependencies".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "include_completed": {
                            "type": "boolean",
                            "description": "Include completed tasks in the plan",
                            "default": false
                        }
                    })),
                    required: None,
                    additional_properties: None,
                },
            },
        ]
    }

    /// Handle tool calls
    async fn handle_tool_call(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        match name {
            "task_enqueue" => self.handle_task_enqueue(arguments).await,
            "task_get" => self.handle_task_get(arguments).await,
            "task_list" => self.handle_task_list(arguments).await,
            "task_queue_status" => self.handle_task_queue_status(arguments).await,
            "task_cancel" => self.handle_task_cancel(arguments).await,
            "task_execution_plan" => self.handle_task_execution_plan(arguments).await,
            _ => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Unknown tool: {}", name)),
                    data: None,
                    annotations: None,
                }],
                is_error: Some(true),
            }),
        }
    }

    async fn handle_task_enqueue(&self, arguments: Value) -> Result<CallToolResult> {
        let summary = arguments["summary"]
            .as_str()
            .context("missing summary")?
            .to_string();
        let description = arguments["description"]
            .as_str()
            .context("missing description")?
            .to_string();

        let mut task = Task::new(summary, description);

        // Optional fields
        if let Some(agent_type) = arguments["agent_type"].as_str() {
            task.agent_type = agent_type.to_string();
        }
        if let Some(priority) = arguments["priority"].as_u64() {
            task.priority = priority as u8;
        }
        if let Some(deps) = arguments["dependencies"].as_array() {
            let dependencies: Result<Vec<Uuid>> = deps
                .iter()
                .map(|v| {
                    v.as_str()
                        .context("invalid dependency ID")
                        .and_then(|s| Uuid::parse_str(s).context("invalid UUID"))
                })
                .collect();
            task.dependencies = Some(dependencies?);
        }
        if let Some(dep_type_str) = arguments["dependency_type"].as_str() {
            task.dependency_type = dep_type_str
                .parse::<DependencyType>()
                .unwrap_or(DependencyType::Sequential);
        }

        match self.task_service.submit(task).await {
            Ok(task_id) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Task enqueued successfully with ID: {}", task_id)),
                    data: Some(json!({"task_id": task_id})),
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to enqueue task: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to enqueue task: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_task_get(&self, arguments: Value) -> Result<CallToolResult> {
        let task_id_str = arguments["task_id"]
            .as_str()
            .context("missing task_id")?;
        let task_id = Uuid::parse_str(task_id_str).context("invalid task_id UUID")?;

        match self.task_service.get(task_id).await {
            Ok(Some(task)) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Task found: {}", task.summary)),
                    data: Some(serde_json::to_value(&task).unwrap()),
                    annotations: None,
                }],
                is_error: None,
            }),
            Ok(None) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Task not found: {}", task_id)),
                    data: None,
                    annotations: None,
                }],
                is_error: Some(true),
            }),
            Err(e) => {
                error!("Failed to get task: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to get task: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_task_list(&self, arguments: Value) -> Result<CallToolResult> {
        let mut filters = TaskFilters::default();

        if let Some(status_str) = arguments["status"].as_str() {
            filters.status = status_str.parse::<TaskStatus>().ok();
        }
        if let Some(agent_type) = arguments["agent_type"].as_str() {
            filters.agent_type = Some(agent_type.to_string());
        }
        if let Some(limit) = arguments["limit"].as_u64() {
            filters.limit = Some(limit as usize);
        }

        match self.task_service.list(filters).await {
            Ok(tasks) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Found {} tasks", tasks.len())),
                    data: Some(json!({
                        "count": tasks.len(),
                        "tasks": tasks
                    })),
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to list tasks: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to list tasks: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_task_queue_status(&self, _arguments: Value) -> Result<CallToolResult> {
        // Get counts for each status
        let statuses = vec![
            TaskStatus::Pending,
            TaskStatus::Blocked,
            TaskStatus::Ready,
            TaskStatus::Running,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ];

        let mut status_counts = json!({});

        for status in statuses {
            let filters = TaskFilters {
                status: Some(status),
                ..Default::default()
            };
            match self.task_service.count(filters).await {
                Ok(count) => {
                    status_counts[status.to_string()] = json!(count);
                }
                Err(e) => {
                    error!("Failed to count tasks for status {:?}: {}", status, e);
                }
            }
        }

        // Get ready tasks
        let ready_tasks = self.task_service.get_ready_tasks(Some(10)).await.ok();

        Ok(CallToolResult {
            content: vec![rmcp::Content {
                content_type: "text".to_string(),
                text: Some("Task queue status".to_string()),
                data: Some(json!({
                    "status_counts": status_counts,
                    "ready_tasks_preview": ready_tasks.map(|t| t.len()).unwrap_or(0),
                })),
                annotations: None,
            }],
            is_error: None,
        })
    }

    async fn handle_task_cancel(&self, arguments: Value) -> Result<CallToolResult> {
        let task_id_str = arguments["task_id"]
            .as_str()
            .context("missing task_id")?;
        let task_id = Uuid::parse_str(task_id_str).context("invalid task_id UUID")?;

        match self.task_service.cancel(task_id).await {
            Ok(()) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!(
                        "Task cancelled successfully (including dependent tasks): {}",
                        task_id
                    )),
                    data: None,
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to cancel task: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to cancel task: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_task_execution_plan(&self, arguments: Value) -> Result<CallToolResult> {
        let include_completed = arguments["include_completed"].as_bool().unwrap_or(false);

        let mut filters = TaskFilters::default();
        if !include_completed {
            // Exclude completed, failed, and cancelled tasks
            filters.status = None; // We'll filter manually
        }

        match self.task_service.list(filters).await {
            Ok(mut tasks) => {
                // Filter out terminal states if not including completed
                if !include_completed {
                    tasks.retain(|t| !t.is_terminal());
                }

                // Sort by dependency depth (lower depth = execute first) and priority
                tasks.sort_by(|a, b| {
                    a.dependency_depth
                        .cmp(&b.dependency_depth)
                        .then_with(|| {
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
                            "dependencies": t.dependencies,
                        })
                    })
                    .collect();

                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Execution plan with {} tasks", execution_plan.len())),
                        data: Some(json!({
                            "count": execution_plan.len(),
                            "execution_order": execution_plan
                        })),
                        annotations: None,
                    }],
                    is_error: None,
                })
            }
            Err(e) => {
                error!("Failed to generate execution plan: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to generate execution plan: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }
}

#[async_trait::async_trait]
impl RouterService for TaskRouter {
    async fn list_tools(&self) -> Result<ListToolsResult> {
        Ok(ListToolsResult {
            tools: Self::define_tools(),
        })
    }

    async fn call_tool(&self, params: CallToolRequestParams) -> Result<CallToolResult> {
        info!("Tool call: {}", params.name);
        self.handle_tool_call(&params.name, params.arguments.unwrap_or(json!({})))
            .await
    }

    async fn list_resources(&self) -> Result<Vec<Resource>> {
        // Task server doesn't expose resources
        Ok(vec![])
    }

    async fn read_resource(&self, _uri: String) -> Result<String> {
        Err(anyhow::anyhow!("Resources not supported"))
    }

    async fn list_prompts(&self) -> Result<Vec<rmcp::Prompt>> {
        // Task server doesn't expose prompts
        Ok(vec![])
    }

    async fn get_prompt(
        &self,
        _name: String,
        _arguments: Option<Value>,
    ) -> Result<rmcp::GetPromptResult> {
        Err(anyhow::anyhow!("Prompts not supported"))
    }

    async fn set_logging_level(&self, _level: rmcp::LoggingLevel) -> Result<()> {
        // Could implement dynamic log level changes here
        Ok(())
    }
}

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
        models::{DependencyType, Task, TaskStatus},
        ports::TaskFilters,
    },
    infrastructure::database::{connection::DatabaseConnection, task_repo::TaskRepositoryImpl},
    services::{DependencyResolver, PriorityCalculator, TaskQueueService},
};
use anyhow::{Context, Result};
use clap::Parser;
use rmcp::{
    handler::server::wrapper::Parameters, model::ServerInfo, tool, tool_handler, tool_router,
    ErrorData as McpError, Json, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

/// Request parameters for enqueuing a task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TaskEnqueueRequest {
    /// Brief task summary (max 140 chars)
    summary: String,
    /// Detailed task description
    description: String,
    /// Type of agent to execute this task
    #[serde(default = "default_agent_type")]
    agent_type: String,
    /// Task priority (0-10)
    #[serde(default = "default_priority")]
    priority: u8,
    /// List of task IDs this task depends on
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Vec<String>>,
    /// Type of dependency relationship
    #[serde(default = "default_dependency_type")]
    dependency_type: String,
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

/// Request parameters for getting a task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TaskGetRequest {
    /// UUID of the task to retrieve
    task_id: String,
}

/// Request parameters for listing tasks
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TaskListRequest {
    /// Filter by task status
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    /// Filter by agent type
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_type: Option<String>,
    /// Maximum number of tasks to return
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Request parameters for canceling a task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TaskCancelRequest {
    /// UUID of the task to cancel
    task_id: String,
}

/// Request parameters for execution plan
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TaskExecutionPlanRequest {
    /// Include completed tasks in the plan
    #[serde(default)]
    include_completed: bool,
}

/// Task list result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TaskListResult {
    /// Number of tasks found
    count: usize,
    /// List of tasks
    tasks: Vec<Task>,
}

/// Queue status result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct QueueStatusResult {
    /// Counts by status
    status_counts: serde_json::Value,
    /// Number of ready tasks (preview)
    ready_tasks_preview: usize,
}

/// Execution plan item
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ExecutionPlanItem {
    /// Task ID
    #[schemars(with = "String")]
    task_id: Uuid,
    /// Task summary
    summary: String,
    /// Task status
    status: String,
    /// Dependency depth
    dependency_depth: u32,
    /// Calculated priority
    calculated_priority: f64,
    /// Dependencies
    #[schemars(with = "Option<Vec<String>>")]
    dependencies: Option<Vec<Uuid>>,
}

/// Execution plan result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ExecutionPlanResult {
    /// Number of tasks in plan
    count: usize,
    /// Execution order
    execution_order: Vec<ExecutionPlanItem>,
}

/// Task Queue MCP Server implementation
#[derive(Clone)]
struct TaskServer {
    task_service: Arc<TaskQueueService>,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl TaskServer {
    fn new(task_service: Arc<TaskQueueService>) -> Self {
        Self {
            task_service,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl TaskServer {
    /// Enqueue a new task
    #[tool(description = "Enqueue a new task to the task queue")]
    async fn task_enqueue(
        &self,
        params: Parameters<TaskEnqueueRequest>,
    ) -> Result<String, McpError> {
        let params = params.0;

        let mut task = Task::new(params.summary, params.description);
        task.agent_type = params.agent_type;
        task.priority = params.priority;

        if let Some(deps) = params.dependencies {
            let dependencies: Result<Vec<Uuid>, _> = deps
                .iter()
                .map(|s| Uuid::parse_str(s))
                .collect();
            task.dependencies = Some(dependencies.map_err(|e| {
                McpError::invalid_params(format!("Invalid dependency UUID: {}", e), None)
            })?);
        }

        task.dependency_type = params
            .dependency_type
            .parse::<DependencyType>()
            .unwrap_or(DependencyType::Sequential);

        match self.task_service.submit(task).await {
            Ok(task_id) => {
                info!("Task enqueued successfully with ID: {}", task_id);
                Ok(format!("Task enqueued successfully with ID: {}", task_id))
            }
            Err(e) => {
                error!("Failed to enqueue task: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to enqueue task: {}",
                    e
                ), None))
            }
        }
    }

    /// Get task by ID
    #[tool(description = "Get task details by ID")]
    async fn task_get(
        &self,
        params: Parameters<TaskGetRequest>,
    ) -> Result<Json<Task>, McpError> {
        let params = params.0;

        let task_id = Uuid::parse_str(&params.task_id)
            .map_err(|e| McpError::invalid_params(format!("Invalid task_id UUID: {}", e), None))?;

        match self.task_service.get(task_id).await {
            Ok(Some(task)) => {
                info!("Task found: {}", task.summary);
                Ok(Json(task))
            }
            Ok(None) => Err(McpError::invalid_params(format!(
                "Task not found: {}",
                task_id
            ), None)),
            Err(e) => {
                error!("Failed to get task: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to get task: {}",
                    e
                ), None))
            }
        }
    }

    /// List tasks with filters
    #[tool(description = "List tasks with optional filters")]
    async fn task_list(
        &self,
        params: Parameters<TaskListRequest>,
    ) -> Result<Json<TaskListResult>, McpError> {
        let params = params.0;

        let mut filters = TaskFilters::default();

        if let Some(status_str) = params.status {
            filters.status = status_str.parse::<TaskStatus>().ok();
        }
        if let Some(agent_type) = params.agent_type {
            filters.agent_type = Some(agent_type);
        }
        filters.limit = Some(params.limit);

        match self.task_service.list(filters).await {
            Ok(tasks) => {
                info!("Found {} tasks", tasks.len());
                Ok(Json(TaskListResult {
                    count: tasks.len(),
                    tasks,
                }))
            }
            Err(e) => {
                error!("Failed to list tasks: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to list tasks: {}",
                    e
                ), None))
            }
        }
    }

    /// Get task queue status
    #[tool(description = "Get task queue statistics and status")]
    async fn task_queue_status(&self) -> Result<Json<QueueStatusResult>, McpError> {
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

        let mut status_counts = serde_json::json!({});

        for status in statuses {
            let filters = TaskFilters {
                status: Some(status),
                ..Default::default()
            };
            match self.task_service.count(filters).await {
                Ok(count) => {
                    status_counts[status.to_string()] = serde_json::json!(count);
                }
                Err(e) => {
                    error!("Failed to count tasks for status {:?}: {}", status, e);
                }
            }
        }

        // Get ready tasks preview
        let ready_tasks_count = self
            .task_service
            .get_ready_tasks(Some(10))
            .await
            .ok()
            .map(|t| t.len())
            .unwrap_or(0);

        Ok(Json(QueueStatusResult {
            status_counts,
            ready_tasks_preview: ready_tasks_count,
        }))
    }

    /// Cancel a task
    #[tool(description = "Cancel a task and cascade cancellation to dependent tasks")]
    async fn task_cancel(
        &self,
        params: Parameters<TaskCancelRequest>,
    ) -> Result<String, McpError> {
        let params = params.0;

        let task_id = Uuid::parse_str(&params.task_id)
            .map_err(|e| McpError::invalid_params(format!("Invalid task_id UUID: {}", e), None))?;

        match self.task_service.cancel(task_id).await {
            Ok(()) => {
                info!("Task cancelled successfully: {}", task_id);
                Ok(format!(
                    "Task cancelled successfully (including dependent tasks): {}",
                    task_id
                ))
            }
            Err(e) => {
                error!("Failed to cancel task: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to cancel task: {}",
                    e
                ), None))
            }
        }
    }

    /// Get execution plan
    #[tool(description = "Get the execution order of tasks based on dependencies")]
    async fn task_execution_plan(
        &self,
        params: Parameters<TaskExecutionPlanRequest>,
    ) -> Result<Json<ExecutionPlanResult>, McpError> {
        let params = params.0;

        let filters = TaskFilters::default();

        match self.task_service.list(filters).await {
            Ok(mut tasks) => {
                // Filter out terminal states if not including completed
                if !params.include_completed {
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
                    .map(|t| ExecutionPlanItem {
                        task_id: t.id,
                        summary: t.summary.clone(),
                        status: t.status.to_string(),
                        dependency_depth: t.dependency_depth,
                        calculated_priority: t.calculated_priority,
                        dependencies: t.dependencies.clone(),
                    })
                    .collect();

                info!("Generated execution plan with {} tasks", execution_plan.len());

                Ok(Json(ExecutionPlanResult {
                    count: execution_plan.len(),
                    execution_order: execution_plan,
                }))
            }
            Err(e) => {
                error!("Failed to generate execution plan: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to generate execution plan: {}",
                    e
                ), None))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TaskServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: rmcp::model::ProtocolVersion::default(),
            capabilities: rmcp::model::ServerCapabilities::default(),
            server_info: rmcp::model::Implementation {
                name: "abathur-tasks".to_string(),
                title: Some("Abathur Task Queue Management Server".to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Task queue management server for Abathur. Use tools to enqueue, list, cancel tasks and get queue status.".to_string()
            ),
        }
    }
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
    let server = TaskServer::new(task_service);

    info!("MCP server ready, listening on stdio");

    // Run server with stdio transport
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let _running = server
        .serve((stdin, stdout))
        .await
        .map_err(|_| anyhow::anyhow!("Server initialization failed"))?;

    // Keep running until interrupted
    tokio::signal::ctrl_c().await?;

    Ok(())
}

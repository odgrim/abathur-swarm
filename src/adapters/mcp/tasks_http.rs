//! MCP Tasks HTTP Server.
//!
//! Provides HTTP endpoints for Claude Code agents to interact with
//! the task queue. Supports querying, submitting, and updating tasks.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::domain::models::{Task, TaskPriority, TaskSource, TaskStatus};
use crate::domain::ports::TaskRepository;
use crate::services::TaskService;

/// Configuration for the tasks HTTP server.
#[derive(Debug, Clone)]
pub struct TasksHttpConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Whether to enable CORS.
    pub enable_cors: bool,
}

impl Default for TasksHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 9101,
            enable_cors: true,
        }
    }
}

/// Request to submit a new task.
#[derive(Debug, Deserialize)]
pub struct SubmitTaskRequest {
    pub prompt: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<Uuid>,
    #[serde(default)]
    pub goal_id: Option<Uuid>,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Request to complete a task.
#[derive(Debug, Deserialize)]
pub struct CompleteTaskRequest {
    // No additional fields needed - just signals completion
}

/// Request to fail a task.
#[derive(Debug, Deserialize)]
pub struct FailTaskRequest {
    #[serde(default)]
    pub error: Option<String>,
}

/// Request to claim a task.
#[derive(Debug, Deserialize)]
pub struct ClaimTaskRequest {
    pub agent_type: String,
}

/// Query parameters for task listing.
#[derive(Debug, Deserialize)]
pub struct TaskQueryParams {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub goal_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Response with a task.
#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub agent_type: Option<String>,
    pub depends_on: Vec<Uuid>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub worktree_path: Option<String>,
    pub artifacts: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl From<Task> for TaskResponse {
    fn from(t: Task) -> Self {
        Self {
            id: t.id,
            parent_id: t.parent_id,
            title: t.title,
            description: t.description,
            status: t.status.as_str().to_string(),
            priority: t.priority.as_str().to_string(),
            agent_type: t.agent_type,
            depends_on: t.depends_on,
            retry_count: t.retry_count,
            max_retries: t.max_retries,
            worktree_path: t.worktree_path,
            artifacts: t.artifacts.iter().map(|a| a.uri.clone()).collect(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            started_at: t.started_at.map(|dt| dt.to_rfc3339()),
            completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

/// Response with queue statistics.
#[derive(Debug, Serialize)]
pub struct QueueStatsResponse {
    pub pending: u64,
    pub ready: u64,
    pub running: u64,
    pub complete: u64,
    pub failed: u64,
    pub total: u64,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

/// Shared state for the tasks HTTP server.
struct AppState<T: TaskRepository> {
    service: TaskService<T>,
}

/// Tasks HTTP Server.
pub struct TasksHttpServer<T: TaskRepository + 'static> {
    config: TasksHttpConfig,
    service: TaskService<T>,
}

impl<T: TaskRepository + Clone + Send + Sync + 'static>
    TasksHttpServer<T>
{
    pub fn new(service: TaskService<T>, config: TasksHttpConfig) -> Self {
        Self { config, service }
    }

    /// Build the router.
    fn build_router(self) -> Router {
        let state = Arc::new(AppState {
            service: self.service,
        });

        let app = Router::new()
            // Task CRUD operations
            .route("/api/v1/tasks", get(list_tasks::<T>))
            .route("/api/v1/tasks", post(submit_task::<T>))
            .route("/api/v1/tasks/{id}", get(get_task::<T>))
            // Task lifecycle operations
            .route("/api/v1/tasks/{id}/claim", post(claim_task::<T>))
            .route("/api/v1/tasks/{id}/complete", post(complete_task::<T>))
            .route("/api/v1/tasks/{id}/fail", post(fail_task::<T>))
            .route("/api/v1/tasks/{id}/retry", post(retry_task::<T>))
            // Ready tasks
            .route("/api/v1/tasks/ready", get(list_ready_tasks::<T>))
            // Statistics
            .route("/api/v1/tasks/stats", get(get_stats::<T>))
            // Health check
            .route("/health", get(health_check))
            .with_state(state);

        if self.config.enable_cors {
            app.layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
                .layer(TraceLayer::new_for_http())
        } else {
            app.layer(TraceLayer::new_for_http())
        }
    }

    /// Start the server.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("MCP Tasks HTTP server listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Start the server with a shutdown signal.
    pub async fn serve_with_shutdown<F>(
        self,
        shutdown: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("MCP Tasks HTTP server listening on {}", addr);

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

async fn list_tasks<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Query(params): Query<TaskQueryParams>,
) -> Result<Json<Vec<TaskResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let result = state.service.get_ready_tasks(params.limit).await;

    match result {
        Ok(tasks) => {
            let mut tasks: Vec<_> = tasks.into_iter().map(TaskResponse::from).collect();

            // Filter by status if specified
            if let Some(status_str) = &params.status {
                if let Some(status) = parse_status(status_str) {
                    tasks.retain(|t| t.status == status.as_str());
                }
            }

            tasks.truncate(params.limit);
            Ok(Json(tasks))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "QUERY_ERROR".to_string(),
            }),
        )),
    }
}

async fn submit_task<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Json(req): Json<SubmitTaskRequest>,
) -> Result<(StatusCode, Json<TaskResponse>), (StatusCode, Json<ErrorResponse>)> {
    let priority = req
        .priority
        .as_ref()
        .and_then(|p| parse_priority(p))
        .unwrap_or(TaskPriority::Normal);

    // Determine source based on whether a goal_id was provided
    let source = match req.goal_id {
        Some(gid) => TaskSource::GoalEvaluation(gid),
        None => TaskSource::Human,
    };

    match state
        .service
        .submit_task(
            req.title,
            req.prompt,
            req.parent_id,
            priority,
            req.agent_type,
            req.depends_on,
            None,
            req.idempotency_key,
            source,
        )
        .await
    {
        Ok(task) => Ok((StatusCode::CREATED, Json(TaskResponse::from(task)))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SUBMIT_ERROR".to_string(),
            }),
        )),
    }
}

async fn get_task<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.get_task(id).await {
        Ok(Some(task)) => Ok(Json(TaskResponse::from(task))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Task {} not found", id),
                code: "NOT_FOUND".to_string(),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "GET_ERROR".to_string(),
            }),
        )),
    }
}

async fn claim_task<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ClaimTaskRequest>,
) -> Result<Json<TaskResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.claim_task(id, &req.agent_type).await {
        Ok(task) => Ok(Json(TaskResponse::from(task))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CLAIM_ERROR".to_string(),
            }),
        )),
    }
}

async fn complete_task<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.complete_task(id).await {
        Ok(task) => Ok(Json(TaskResponse::from(task))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "COMPLETE_ERROR".to_string(),
            }),
        )),
    }
}

async fn fail_task<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<FailTaskRequest>,
) -> Result<Json<TaskResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.fail_task(id, req.error).await {
        Ok(task) => Ok(Json(TaskResponse::from(task))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "FAIL_ERROR".to_string(),
            }),
        )),
    }
}

async fn retry_task<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.retry_task(id).await {
        Ok(task) => Ok(Json(TaskResponse::from(task))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "RETRY_ERROR".to_string(),
            }),
        )),
    }
}

async fn list_ready_tasks<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
    Query(params): Query<TaskQueryParams>,
) -> Result<Json<Vec<TaskResponse>>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.get_ready_tasks(params.limit).await {
        Ok(tasks) => Ok(Json(tasks.into_iter().map(TaskResponse::from).collect())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "QUERY_ERROR".to_string(),
            }),
        )),
    }
}

async fn get_stats<T: TaskRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<T>>>,
) -> Result<Json<QueueStatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get counts by fetching tasks for each status
    // Note: In production, we'd add a dedicated count_by_status method
    let ready = state.service.get_ready_tasks(1000).await.map(|t| t.len() as u64).unwrap_or(0);

    Ok(Json(QueueStatsResponse {
        pending: 0, // Would need additional query
        ready,
        running: 0, // Would need additional query
        complete: 0, // Would need additional query
        failed: 0, // Would need additional query
        total: ready, // Incomplete - just ready for now
    }))
}

fn parse_status(s: &str) -> Option<TaskStatus> {
    match s.to_lowercase().as_str() {
        "pending" => Some(TaskStatus::Pending),
        "ready" => Some(TaskStatus::Ready),
        "running" => Some(TaskStatus::Running),
        "complete" | "completed" => Some(TaskStatus::Complete),
        "failed" => Some(TaskStatus::Failed),
        "blocked" => Some(TaskStatus::Blocked),
        "canceled" | "cancelled" => Some(TaskStatus::Canceled),
        _ => None,
    }
}

fn parse_priority(s: &str) -> Option<TaskPriority> {
    match s.to_lowercase().as_str() {
        "low" => Some(TaskPriority::Low),
        "normal" => Some(TaskPriority::Normal),
        "high" => Some(TaskPriority::High),
        "critical" => Some(TaskPriority::Critical),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = TasksHttpConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 9101);
        assert!(config.enable_cors);
    }

    #[test]
    fn test_parse_status() {
        assert_eq!(parse_status("pending"), Some(TaskStatus::Pending));
        assert_eq!(parse_status("READY"), Some(TaskStatus::Ready));
        assert_eq!(parse_status("Running"), Some(TaskStatus::Running));
        assert_eq!(parse_status("complete"), Some(TaskStatus::Complete));
        assert_eq!(parse_status("completed"), Some(TaskStatus::Complete));
        assert_eq!(parse_status("failed"), Some(TaskStatus::Failed));
        assert_eq!(parse_status("invalid"), None);
    }

    #[test]
    fn test_parse_priority() {
        assert_eq!(parse_priority("low"), Some(TaskPriority::Low));
        assert_eq!(parse_priority("NORMAL"), Some(TaskPriority::Normal));
        assert_eq!(parse_priority("High"), Some(TaskPriority::High));
        assert_eq!(parse_priority("critical"), Some(TaskPriority::Critical));
        assert_eq!(parse_priority("invalid"), None);
    }

    #[test]
    fn test_submit_request_deserialization() {
        let json = r#"{"prompt": "Do something"}"#;
        let req: SubmitTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.prompt, "Do something");
        assert!(req.title.is_none());
        assert!(req.depends_on.is_empty());

        // With optional title
        let json = r#"{"prompt": "Do something", "title": "Custom title"}"#;
        let req: SubmitTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.prompt, "Do something");
        assert_eq!(req.title, Some("Custom title".to_string()));
    }

    #[test]
    fn test_task_response_serialization() {
        let response = TaskResponse {
            id: Uuid::new_v4(),
            parent_id: None,
            title: "Test".to_string(),
            description: "Description".to_string(),
            status: "pending".to_string(),
            priority: "normal".to_string(),
            agent_type: Some("developer".to_string()),
            depends_on: vec![],
            retry_count: 0,
            max_retries: 3,
            worktree_path: None,
            artifacts: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            started_at: None,
            completed_at: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"title\":\"Test\""));
        assert!(json.contains("\"status\":\"pending\""));
    }
}

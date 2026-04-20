//! Axum server handling incoming federation JSON-RPC requests.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse, routing::{get, post}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, info, warn};

use abathur::domain::models::a2a::{
    A2AAgentCard, FederationCard, FederationRole, FederationTaskEnvelope, MessagePriority,
};
use abathur::services::federation::service::FederationHttpClient;

use crate::clickup::client::ClickUpApi;
use crate::clickup::models::CreateTaskRequest;
use crate::config::Config;
use crate::state;

/// Shared application state.
pub struct AppState {
    pub config: Config,
    pub db: sqlx::SqlitePool,
    pub clickup: Arc<dyn ClickUpApi>,
    pub federation_client: FederationHttpClient,
}

/// Build the Axum router.
pub fn build_router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/", post(handle_jsonrpc))
        .route("/health", get(handle_health))
        .with_state(state)
}

async fn handle_health() -> impl IntoResponse {
    StatusCode::OK
}

// ---- JSON-RPC types ----

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[expect(dead_code, reason = "required for JSON-RPC protocol validation")]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// ---- Main dispatcher ----

async fn handle_jsonrpc(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    info!(method = %req.method, "Received JSON-RPC request");

    let response = match req.method.as_str() {
        "federation/discover" => handle_discover(&state).await,
        "federation/delegate" => handle_delegate(&state, req.params).await,
        "federation/register" => handle_register(req.params),
        "federation/disconnect" => handle_disconnect(),
        "federation/reconcile" => handle_reconcile(&state).await,
        _ => {
            warn!(method = %req.method, "Unknown JSON-RPC method");
            Err((-32601, format!("Method not found: {}", req.method)))
        }
    };

    Json(match response {
        Ok(result) => JsonRpcResponse::success(req.id, result),
        Err((code, msg)) => JsonRpcResponse::error(req.id, code, msg),
    })
}

// ---- Handlers ----

async fn handle_discover(state: &AppState) -> Result<Value, (i32, String)> {
    let active = state::count_active(&state.db).await.unwrap_or(0);
    let load = active as f64 / state.config.identity.max_concurrent_tasks.max(1) as f64;

    let card = FederationCard {
        card: A2AAgentCard {
            agent_id: state.config.identity.cerebrate_id.clone(),
            display_name: state.config.identity.display_name.clone(),
            description: "Human operator proxy — delegates tasks to humans via ClickUp".to_string(),
            tier: "human".to_string(),
            capabilities: state.config.identity.capabilities.clone(),
            accepts: vec![],
            handoff_targets: vec![],
            available: true,
            load,
        },
        parent_id: None,
        hive_id: None,
        federation_role: FederationRole::Cerebrate,
        max_accepted_tasks: state.config.identity.max_concurrent_tasks,
        heartbeat_ok: true,
    };

    serde_json::to_value(&card).map_err(|e| (-32603, format!("Serialization error: {e}")))
}

async fn handle_delegate(
    state: &AppState,
    params: Value,
) -> Result<Value, (i32, String)> {
    let envelope: FederationTaskEnvelope =
        serde_json::from_value(params).map_err(|e| (-32602, format!("Invalid params: {e}")))?;

    let task_id_str = envelope.task_id.to_string();

    // Check for duplicate
    if let Ok(Some(existing)) = state::get_mapping(&state.db, &task_id_str).await {
        return Ok(serde_json::json!({
            "status": "accepted",
            "clickup_task_id": existing.clickup_task_id
        }));
    }

    // Check capacity
    let active = state::count_active(&state.db)
        .await
        .map_err(|e| (-32603, format!("Database error: {e}")))?;
    if active >= state.config.identity.max_concurrent_tasks as i64 {
        return Err((-32001, "At capacity".to_string()));
    }

    // Map priority
    let (priority_num, priority_str) = match envelope.priority {
        MessagePriority::Urgent => (1u8, "urgent"),
        MessagePriority::High => (2, "high"),
        MessagePriority::Normal => (3, "normal"),
        MessagePriority::Low => (4, "low"),
    };

    // Compute deadline
    let now = chrono::Utc::now();
    let deadline = now
        + chrono::Duration::seconds(state.config.polling.task_deadline_secs as i64);
    let due_date_ms = deadline.timestamp_millis();

    // Format description
    let description = format_clickup_description(&envelope, &deadline);

    // Create ClickUp task
    let req = CreateTaskRequest {
        name: envelope.title.clone(),
        description,
        priority: priority_num,
        due_date: Some(due_date_ms),
        status: None,
    };

    let clickup_resp = state
        .clickup
        .create_task(&state.config.clickup.list_id, &req)
        .await
        .map_err(|e| {
            error!("Failed to create ClickUp task: {e}");
            (-32603, format!("Failed to create ClickUp task: {e}"))
        })?;

    let envelope_json = serde_json::to_string(&envelope).unwrap_or_default();
    let now_str = now.to_rfc3339();
    let deadline_str = deadline.to_rfc3339();

    let mapping = state::TaskMapping {
        federation_task_id: task_id_str.clone(),
        correlation_id: envelope.correlation_id.to_string(),
        clickup_task_id: clickup_resp.id.clone(),
        title: envelope.title.clone(),
        status: "pending".to_string(),
        priority: priority_str.to_string(),
        parent_goal_id: envelope.parent_goal_id.map(|id| id.to_string()),
        envelope_json,
        clickup_status: clickup_resp.status.status.clone(),
        human_response: None,
        created_at: now_str.clone(),
        updated_at: now_str,
        deadline_at: deadline_str,
        result_sent: false,
    };

    state::insert_mapping(&state.db, &mapping)
        .await
        .map_err(|e| (-32603, format!("Database error: {e}")))?;

    // Fire-and-forget: send accept to overmind
    let fed_client = state.federation_client.clone();
    let overmind_url = state.config.parent.overmind_url.clone();
    let task_id = envelope.task_id;
    let cerebrate_id = state.config.identity.cerebrate_id.clone();
    tokio::spawn(async move {
        if let Err(e) = fed_client
            .send_accept(&overmind_url, task_id, &cerebrate_id)
            .await
        {
            warn!("Failed to send accept to overmind: {e}");
        }
    });

    Ok(serde_json::json!({
        "status": "accepted",
        "clickup_task_id": clickup_resp.id
    }))
}

fn handle_register(params: Value) -> Result<Value, (i32, String)> {
    let cerebrate_id = params
        .get("cerebrate_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Ok(serde_json::json!({
        "status": "registered",
        "cerebrate_id": cerebrate_id
    }))
}

fn handle_disconnect() -> Result<Value, (i32, String)> {
    Ok(serde_json::json!({
        "status": "disconnected"
    }))
}

async fn handle_reconcile(state: &AppState) -> Result<Value, (i32, String)> {
    let task_ids = state::get_active_task_ids(&state.db)
        .await
        .map_err(|e| (-32603, format!("Database error: {e}")))?;

    Ok(serde_json::json!({
        "task_ids": task_ids
    }))
}

// ---- Helpers ----

fn format_clickup_description(
    envelope: &FederationTaskEnvelope,
    deadline: &chrono::DateTime<chrono::Utc>,
) -> String {
    let parent_goal = envelope
        .context
        .parent_goal_summary
        .as_deref()
        .unwrap_or("N/A");

    let constraints = if envelope.constraints.is_empty() {
        "None".to_string()
    } else {
        envelope
            .constraints
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let hints = if envelope.context.hints.is_empty() {
        "None".to_string()
    } else {
        envelope
            .context
            .hints
            .iter()
            .map(|h| format!("- {h}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let related = if envelope.context.related_artifacts.is_empty() {
        "None".to_string()
    } else {
        envelope
            .context
            .related_artifacts
            .iter()
            .map(|a| format!("- {} : {}", a.artifact_type, a.value))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let deadline_str = deadline.format("%Y-%m-%d %H:%M UTC").to_string();

    format!(
        r#"## Federation Task: {title}

**Task ID**: {task_id}
**Priority**: {priority:?}
**Parent Goal**: {parent_goal}
**Deadline**: {deadline_str}

### Description
{description}

### Constraints
{constraints}

### Context
{hints}

### Related Artifacts
{related}

---
**Instructions**: Complete this task and change the status to "complete".
Add a comment with your results. Include any URLs, account numbers,
or relevant details. You can use a ```json block for structured data."#,
        title = envelope.title,
        task_id = envelope.task_id,
        priority = envelope.priority,
        description = envelope.description,
    )
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clickup::models::*;
    use crate::config::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    use tower::ServiceExt;

    struct MockClickUp;

    #[async_trait]
    impl ClickUpApi for MockClickUp {
        async fn create_task(&self, _list_id: &str, req: &CreateTaskRequest) -> Result<CreateTaskResponse> {
            Ok(CreateTaskResponse {
                id: "mock_cu_123".to_string(),
                name: req.name.clone(),
                status: ClickUpStatus {
                    status: "to do".to_string(),
                },
            })
        }
        async fn get_task(&self, _task_id: &str) -> Result<Option<ClickUpTask>> {
            Ok(None)
        }
        async fn get_comments(&self, _task_id: &str) -> Result<Vec<ClickUpComment>> {
            Ok(vec![])
        }
    }

    fn test_config() -> Config {
        Config {
            server: ServerConfig {
                bind_address: "127.0.0.1".to_string(),
                port: 0,
            },
            identity: IdentityConfig {
                cerebrate_id: "test-cerebrate".to_string(),
                display_name: "Test Human".to_string(),
                capabilities: vec!["real-world".to_string()],
                max_concurrent_tasks: 5,
            },
            parent: ParentConfig {
                overmind_url: "http://localhost:9999".to_string(),
                heartbeat_interval_secs: 30,
            },
            clickup: ClickUpConfig {
                workspace_id: "ws1".to_string(),
                list_id: "list1".to_string(),
                completed_statuses: vec!["complete".to_string()],
                failed_statuses: vec!["cancelled".to_string()],
            },
            polling: PollingConfig {
                interval_secs: 60,
                task_deadline_secs: 1209600,
                progress_interval_secs: 900,
            },
            database: DatabaseConfig {
                path: ":memory:".to_string(),
            },
            tls: TlsConfig::default(),
        }
    }

    async fn setup_state() -> Arc<AppState> {
        let db = sqlx::SqlitePool::connect_with(
            SqliteConnectOptions::from_str(":memory:")
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        state::run_migrations(&db).await.unwrap();
        Arc::new(AppState {
            config: test_config(),
            db,
            clickup: Arc::new(MockClickUp),
            federation_client: FederationHttpClient::new(),
        })
    }

    async fn jsonrpc_request(app: axum::Router, method: &str, params: Value) -> Value {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_discover_returns_federation_card() {
        let state = setup_state().await;
        let app = build_router(state);
        let resp = jsonrpc_request(app, "federation/discover", Value::Null).await;
        assert!(resp.get("result").is_some(), "Expected result field: {resp}");
        let result = &resp["result"];
        // card fields are flattened via #[serde(flatten)]
        assert_eq!(result["agent_id"], "test-cerebrate");
        assert_eq!(result["tier"], "human");
        assert_eq!(result["federation_role"], "cerebrate");
    }

    #[tokio::test]
    async fn test_register_returns_ack() {
        let state = setup_state().await;
        let app = build_router(state);
        let resp = jsonrpc_request(
            app,
            "federation/register",
            serde_json::json!({"cerebrate_id": "test-cerebrate"}),
        )
        .await;
        assert_eq!(resp["result"]["status"], "registered");
    }

    #[tokio::test]
    async fn test_disconnect_returns_ack() {
        let state = setup_state().await;
        let app = build_router(state);
        let resp = jsonrpc_request(app, "federation/disconnect", Value::Null).await;
        assert_eq!(resp["result"]["status"], "disconnected");
    }

    #[tokio::test]
    async fn test_unknown_method_returns_error() {
        let state = setup_state().await;
        let app = build_router(state);
        let resp = jsonrpc_request(app, "unknown/method", Value::Null).await;
        assert!(resp.get("error").is_some(), "Expected error: {resp}");
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_reconcile_returns_task_ids() {
        let state = setup_state().await;
        let app = build_router(state);
        let resp = jsonrpc_request(
            app,
            "federation/reconcile",
            serde_json::json!({"cerebrate_id": "test"}),
        )
        .await;
        assert_eq!(resp["result"]["task_ids"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_delegate_creates_task_and_mapping() {
        let state = setup_state().await;
        let app = build_router(state.clone());

        let task_id = uuid::Uuid::new_v4();
        let envelope = abathur::domain::models::a2a::FederationTaskEnvelope::new(
            task_id,
            "Open bank account",
            "Open a business bank account at Example Bank",
        );
        let params = serde_json::to_value(&envelope).unwrap();

        let resp = jsonrpc_request(app, "federation/delegate", params).await;
        assert!(resp.get("result").is_some(), "Expected result: {resp}");
        assert_eq!(resp["result"]["status"], "accepted");
        assert_eq!(resp["result"]["clickup_task_id"], "mock_cu_123");

        // Verify mapping was persisted in SQLite
        let mapping = state::get_mapping(&state.db, &task_id.to_string())
            .await
            .unwrap()
            .expect("mapping should exist");
        assert_eq!(mapping.clickup_task_id, "mock_cu_123");
        assert_eq!(mapping.status, "pending");
        assert_eq!(mapping.priority, "normal");
    }

    #[tokio::test]
    async fn test_delegate_duplicate_returns_existing() {
        let state = setup_state().await;

        let task_id = uuid::Uuid::new_v4();
        let envelope = abathur::domain::models::a2a::FederationTaskEnvelope::new(
            task_id,
            "Open bank account",
            "Open a business bank account",
        );
        let params = serde_json::to_value(&envelope).unwrap();

        // First delegate
        let app = build_router(state.clone());
        let resp1 = jsonrpc_request(app, "federation/delegate", params.clone()).await;
        assert_eq!(resp1["result"]["status"], "accepted");

        // Second delegate with same task_id — should return existing
        let app = build_router(state.clone());
        let resp2 = jsonrpc_request(app, "federation/delegate", params).await;
        assert_eq!(resp2["result"]["status"], "accepted");
        assert_eq!(resp2["result"]["clickup_task_id"], "mock_cu_123");

        // Only one mapping should exist
        let active = state::count_active(&state.db).await.unwrap();
        assert_eq!(active, 1);
    }

    #[tokio::test]
    async fn test_delegate_at_capacity_returns_error() {
        // Create config with max_concurrent_tasks = 1
        let mut config = test_config();
        config.identity.max_concurrent_tasks = 1;

        let db = sqlx::SqlitePool::connect_with(
            SqliteConnectOptions::from_str(":memory:")
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        state::run_migrations(&db).await.unwrap();
        let state = Arc::new(AppState {
            config,
            db,
            clickup: Arc::new(MockClickUp),
            federation_client: FederationHttpClient::new(),
        });

        // First task should succeed
        let envelope1 = abathur::domain::models::a2a::FederationTaskEnvelope::new(
            uuid::Uuid::new_v4(),
            "Task 1",
            "First task",
        );
        let app = build_router(state.clone());
        let resp = jsonrpc_request(app, "federation/delegate", serde_json::to_value(&envelope1).unwrap()).await;
        assert_eq!(resp["result"]["status"], "accepted");

        // Second task should fail with capacity error
        let envelope2 = abathur::domain::models::a2a::FederationTaskEnvelope::new(
            uuid::Uuid::new_v4(),
            "Task 2",
            "Second task",
        );
        let app = build_router(state.clone());
        let resp = jsonrpc_request(app, "federation/delegate", serde_json::to_value(&envelope2).unwrap()).await;
        assert!(resp.get("error").is_some(), "Expected capacity error: {resp}");
        assert_eq!(resp["error"]["code"], -32001);
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = setup_state().await;
        let app = build_router(state);
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

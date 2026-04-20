//! `tasks/*` JSON-RPC handlers — send, get, cancel, sendSubscribe (SSE),
//! pushNotificationConfig.
//!
//! Also hosts `handle_federation_routing` (the `tasks/send` fast path that
//! recognises `abathur:federation` metadata and routes into
//! `FederationService` instead of creating a local task) and the
//! `extract_title_description` helper.

use axum::response::{
    Json,
    sse::{Event, KeepAlive, Sse},
};
use chrono::Utc;
use futures::stream::{self, Stream};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::models::a2a::FederationTaskEnvelope;

use super::{
    A2AErrorCode, A2AProtocolMessage, A2AState, A2ATask, A2ATaskState, A2ATaskStatus, InMemoryTask,
    JsonRpcRequest, JsonRpcResponse, MessagePart, PushNotificationConfigParams, TaskCancelParams,
    TaskGetParams, TaskSendParams,
};

pub(super) async fn handle_tasks_send(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: TaskSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    // Phase 1.4: If metadata contains "abathur:federation", route to the
    // FederationService instead of creating a local task.
    if let Some(ref metadata) = params.metadata
        && let Some(federation_val) = metadata.get("abathur:federation")
        && let Some(result) =
            handle_federation_routing(&state, &request.id, federation_val, &params).await
    {
        return result;
    }
    // If handle_federation_routing returns None, fall through to
    // normal task handling (e.g. unknown intent or no federation service).

    // Check if task exists (continue existing task) or create new.
    // Note: session tracking is intentionally omitted here — the sessions map
    // is not read by any handler, so populating it would be a memory leak
    // (unbounded growth with no eviction). Session support should be added
    // with proper lifecycle management when multi-turn conversations are needed.
    let new_task = InMemoryTask::from_params(&params);
    let task_id = new_task.id.clone();

    let task = {
        let mut tasks = state.tasks.write().await;
        if let Some(existing) = tasks.get_mut(&task_id) {
            existing.history.push(params.message);
            existing.state = A2ATaskState::Working;
            existing.updated_at = Utc::now();
            existing.clone()
        } else {
            tasks.insert(task_id.clone(), new_task.clone());
            new_task
        }
    };

    let task_value = match serde_json::to_value(task.to_a2a_task()) {
        Ok(v) => v,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InternalError,
                Some(json!({"message": format!("Failed to serialize task: {}", e)})),
            ))
        }
    };
    Json(JsonRpcResponse::success(request.id, task_value))
}

/// Route a `tasks/send` request containing federation metadata to the
/// appropriate FederationService method.
///
/// Returns `Some(Json<JsonRpcResponse>)` when the request was handled,
/// or `None` when the caller should fall through to normal task handling.
///
/// Goal-existence validation (e.g. verifying that a `parent_goal_id`
/// referenced in a `delegate` request actually exists) lives on
/// `FederationService::validate_goal_exists`. The transport layer
/// translates the resulting `DomainError` into a JSON-RPC error.
pub(super) async fn handle_federation_routing(
    state: &Arc<A2AState>,
    request_id: &Option<Value>,
    federation_val: &Value,
    params: &TaskSendParams,
) -> Option<Json<JsonRpcResponse>> {
    let federation_service = state.federation_service.as_ref()?;

    let intent = federation_val
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match intent {
        "delegate" => {
            // Extract required fields from federation metadata to build an envelope.
            // task_id and correlation_id are required — return an error if missing or invalid.
            let task_id = match federation_val
                .get("task_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
            {
                Some(id) => id,
                None => {
                    return Some(Json(JsonRpcResponse::error(
                        request_id.clone(),
                        A2AErrorCode::InvalidParams,
                        Some(
                            json!({"message": "Missing or invalid 'task_id' in federation metadata"}),
                        ),
                    )));
                }
            };

            let parent_goal_id = federation_val
                .get("parent_goal_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok());

            // If a parent_goal_id is supplied, validate it exists. This is
            // delegated to FederationService rather than performed inline so
            // the transport layer never touches the goal repository.
            if let Some(pgid) = parent_goal_id
                && let Err(e) = federation_service.validate_goal_exists(pgid).await
            {
                return Some(Json(JsonRpcResponse::error(
                    request_id.clone(),
                    A2AErrorCode::InvalidParams,
                    Some(json!({"message": format!("parent_goal_id validation failed: {}", e)})),
                )));
            }

            let correlation_id = match federation_val
                .get("correlation_id")
                .and_then(|v| v.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
            {
                Some(id) => id,
                None => {
                    return Some(Json(JsonRpcResponse::error(
                        request_id.clone(),
                        A2AErrorCode::InvalidParams,
                        Some(
                            json!({"message": "Missing or invalid 'correlation_id' in federation metadata"}),
                        ),
                    )));
                }
            };

            // Extract title and description from federation metadata first, falling back to message parts
            let (title, description) = extract_title_description(federation_val, &params.message);

            let constraints: Vec<String> = federation_val
                .get("constraints")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let result_schema = federation_val
                .get("result_schema")
                .and_then(|v| v.as_str())
                .map(String::from);

            let mut envelope = FederationTaskEnvelope::new(task_id, title, description);
            envelope.parent_goal_id = parent_goal_id;
            envelope.correlation_id = correlation_id;
            envelope.constraints = constraints;
            envelope.result_schema = result_schema;

            // Delegate to the best available cerebrate
            match federation_service.delegate(envelope).await {
                Ok(cerebrate_id) => {
                    let response_task = A2ATask {
                        id: task_id.to_string(),
                        session_id: Uuid::new_v4().to_string(),
                        status: A2ATaskStatus {
                            state: A2ATaskState::Working,
                            message: Some(A2AProtocolMessage {
                                role: "agent".to_string(),
                                parts: vec![MessagePart::Text {
                                    text: format!("Task delegated to cerebrate {}", cerebrate_id),
                                }],
                            }),
                            timestamp: Some(Utc::now().to_rfc3339()),
                        },
                        history: None,
                        artifacts: None,
                        metadata: Some(json!({
                            "abathur:federation": {
                                "delegated_to": cerebrate_id,
                                "task_id": task_id.to_string(),
                                "correlation_id": correlation_id.to_string(),
                            }
                        })),
                    };
                    match serde_json::to_value(response_task) {
                        Ok(v) => Some(Json(JsonRpcResponse::success(request_id.clone(), v))),
                        Err(e) => Some(Json(JsonRpcResponse::error(
                            request_id.clone(),
                            A2AErrorCode::InternalError,
                            Some(json!({"message": format!("Failed to serialize task: {}", e)})),
                        ))),
                    }
                }
                Err(e) => Some(Json(JsonRpcResponse::error(
                    request_id.clone(),
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Federation delegate failed: {}", e)})),
                ))),
            }
        }

        "heartbeat" => {
            let cerebrate_id = federation_val
                .get("cerebrate_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let load = federation_val
                .get("load")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            federation_service
                .handle_heartbeat(cerebrate_id, load)
                .await;

            let response_task = A2ATask {
                id: Uuid::new_v4().to_string(),
                session_id: Uuid::new_v4().to_string(),
                status: A2ATaskStatus {
                    state: A2ATaskState::Completed,
                    message: Some(A2AProtocolMessage {
                        role: "agent".to_string(),
                        parts: vec![MessagePart::Text {
                            text: "Heartbeat acknowledged".to_string(),
                        }],
                    }),
                    timestamp: Some(Utc::now().to_rfc3339()),
                },
                history: None,
                artifacts: None,
                metadata: Some(json!({
                    "abathur:federation": {
                        "intent": "heartbeat_ack",
                        "cerebrate_id": cerebrate_id,
                    }
                })),
            };
            match serde_json::to_value(response_task) {
                Ok(v) => Some(Json(JsonRpcResponse::success(request_id.clone(), v))),
                Err(e) => Some(Json(JsonRpcResponse::error(
                    request_id.clone(),
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Failed to serialize task: {}", e)})),
                ))),
            }
        }

        "register" => {
            let cerebrate_id = federation_val
                .get("cerebrate_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let display_name = federation_val
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(cerebrate_id);

            let url = federation_val
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            federation_service
                .register_cerebrate(cerebrate_id, display_name, url)
                .await;

            // Attempt to connect
            let connect_result = federation_service.connect(cerebrate_id).await;

            let (state_val, msg) = match connect_result {
                Ok(()) => (
                    A2ATaskState::Completed,
                    format!("Cerebrate {} registered and connected", cerebrate_id),
                ),
                Err(e) => (
                    A2ATaskState::Failed,
                    format!(
                        "Cerebrate {} registered but connection failed: {}",
                        cerebrate_id, e
                    ),
                ),
            };

            let response_task = A2ATask {
                id: Uuid::new_v4().to_string(),
                session_id: Uuid::new_v4().to_string(),
                status: A2ATaskStatus {
                    state: state_val,
                    message: Some(A2AProtocolMessage {
                        role: "agent".to_string(),
                        parts: vec![MessagePart::Text { text: msg }],
                    }),
                    timestamp: Some(Utc::now().to_rfc3339()),
                },
                history: None,
                artifacts: None,
                metadata: Some(json!({
                    "abathur:federation": {
                        "intent": "register_ack",
                        "cerebrate_id": cerebrate_id,
                    }
                })),
            };
            match serde_json::to_value(response_task) {
                Ok(v) => Some(Json(JsonRpcResponse::success(request_id.clone(), v))),
                Err(e) => Some(Json(JsonRpcResponse::error(
                    request_id.clone(),
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Failed to serialize task: {}", e)})),
                ))),
            }
        }

        "goal_delegate" => {
            // Phase 2.4: A parent is delegating a goal to this child.
            // We create a local A2A task in Working state to represent
            // the delegated goal, storing the convergence contract in metadata.
            let goal_id = federation_val
                .get("goal_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let goal_name = federation_val
                .get("goal_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Delegated goal");

            let goal_description = federation_val
                .get("goal_description")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let priority = federation_val
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("normal");

            let constraints: Vec<String> = federation_val
                .get("constraints")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let convergence_contract = federation_val
                .get("convergence_contract")
                .cloned()
                .unwrap_or(json!({}));

            // Create a local in-memory task to track this delegated goal.
            let task_id = Uuid::new_v4().to_string();
            let session_id = Uuid::new_v4().to_string();
            let now = Utc::now();

            let task_metadata = json!({
                "abathur:federation": {
                    "intent": "goal_delegate",
                    "goal_id": goal_id,
                    "goal_name": goal_name,
                    "goal_description": goal_description,
                    "priority": priority,
                    "constraints": constraints,
                    "convergence_contract": convergence_contract,
                }
            });

            let local_task = InMemoryTask {
                id: task_id.clone(),
                session_id: session_id.clone(),
                state: A2ATaskState::Working,
                history: vec![params.message.clone()],
                artifacts: vec![],
                metadata: Some(task_metadata.clone()),
                created_at: now,
                updated_at: now,
                push_config: None,
            };

            // Store in the shared task map.
            {
                let mut tasks = state.tasks.write().await;
                tasks.insert(task_id.clone(), local_task);
            }

            // Lazily spawn the convergence publisher on first goal_delegate.
            {
                let mut publisher_guard = state.convergence_publisher_handle.write().await;
                if publisher_guard.is_none() {
                    use crate::services::federation::convergence_publisher::ConvergencePublisher;
                    let publisher = ConvergencePublisher::new(
                        state.tasks.clone(),
                        std::time::Duration::from_secs(10),
                    );
                    let handle = publisher.spawn();
                    *publisher_guard = Some(handle);
                    tracing::info!("Spawned convergence publisher for goal_delegate tasks");
                }
            }

            let response_task = A2ATask {
                id: task_id.clone(),
                session_id,
                status: A2ATaskStatus {
                    state: A2ATaskState::Working,
                    message: Some(A2AProtocolMessage {
                        role: "agent".to_string(),
                        parts: vec![MessagePart::Text {
                            text: format!("Accepted delegated goal '{}' — now working", goal_name),
                        }],
                    }),
                    timestamp: Some(now.to_rfc3339()),
                },
                history: None,
                artifacts: None,
                metadata: Some(task_metadata),
            };
            match serde_json::to_value(response_task) {
                Ok(v) => Some(Json(JsonRpcResponse::success(request_id.clone(), v))),
                Err(e) => Some(Json(JsonRpcResponse::error(
                    request_id.clone(),
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Failed to serialize task: {}", e)})),
                ))),
            }
        }

        // Unknown intent — fall through to normal task handling
        _ => None,
    }
}

/// Extract title and description from federation metadata or message parts.
///
/// Prefers structured `title` and `description` fields from the federation
/// metadata (set by `From<&FederationTaskEnvelope> for TaskSendParams`).
/// Falls back to parsing message text if metadata doesn't have them.
pub(super) fn extract_title_description(
    federation_val: &Value,
    message: &A2AProtocolMessage,
) -> (String, String) {
    // 1. Try federation metadata first (most reliable source).
    let meta_title = federation_val.get("title").and_then(|v| v.as_str());
    let meta_desc = federation_val.get("description").and_then(|v| v.as_str());

    if let (Some(t), Some(d)) = (meta_title, meta_desc) {
        return (t.to_string(), d.to_string());
    }

    // 2. Fall back to parsing message parts.
    let mut title = meta_title.map(String::from).unwrap_or_default();
    let mut description = meta_desc.map(String::from).unwrap_or_default();

    for part in &message.parts {
        match part {
            MessagePart::Data { data, .. } => {
                if title.is_empty()
                    && let Some(t) = data.get("title").and_then(|v| v.as_str())
                {
                    title = t.to_string();
                }
                if description.is_empty()
                    && let Some(d) = data.get("description").and_then(|v| v.as_str())
                {
                    description = d.to_string();
                }
            }
            MessagePart::Text { text } => {
                if title.is_empty() {
                    // If the text has a double-newline, split into title + description
                    if let Some((t, d)) = text.split_once("\n\n") {
                        title = t.to_string();
                        if description.is_empty() {
                            description = d.to_string();
                        }
                    } else {
                        title = text.clone();
                    }
                }
            }
            _ => {}
        }
    }

    if title.is_empty() {
        title = "Federated task".to_string();
    }
    if description.is_empty() {
        description = title.clone();
    }

    (title, description)
}

// ============================================================================
// Phase 1.5: tasks/sendSubscribe — SSE streaming with federation progress
// ============================================================================

pub(super) async fn handle_tasks_send_subscribe(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Json<JsonRpcResponse>> {
    let params: TaskSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Err(Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            )));
        }
    };

    if !state.config.enable_streaming {
        return Err(Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::UnsupportedOperation,
            Some(json!({"message": "Streaming not enabled"})),
        )));
    }

    // Check for federation metadata — if present, we stream federation progress
    let is_federation = params
        .metadata
        .as_ref()
        .is_some_and(|m| m.get("abathur:federation").is_some());

    // Create the task just like tasks/send
    let new_task = InMemoryTask::from_params(&params);
    let task_id = new_task.id.clone();

    {
        let mut tasks = state.tasks.write().await;
        if let Some(existing) = tasks.get_mut(&task_id) {
            existing.history.push(params.message.clone());
            existing.state = A2ATaskState::Working;
            existing.updated_at = Utc::now();
        } else {
            tasks.insert(task_id.clone(), new_task.clone());
        }
    }

    // If federation, attempt to delegate and track progress
    if is_federation
        && let Some(ref federation_service) = state.federation_service
        && let Some(federation_val) = params
            .metadata
            .as_ref()
            .and_then(|m| m.get("abathur:federation"))
    {
        let (title, description) = extract_title_description(federation_val, &params.message);

        let fed_task_id = federation_val
            .get("task_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);

        let envelope = FederationTaskEnvelope::new(fed_task_id, title, description);

        // Fire and forget the delegation
        let fed_svc = Arc::clone(federation_service);
        let task_id_for_spawn = task_id.clone();
        let state_for_spawn = Arc::clone(&state);
        tokio::spawn(async move {
            match fed_svc.delegate(envelope).await {
                Ok(cerebrate_id) => {
                    let mut tasks = state_for_spawn.tasks.write().await;
                    if let Some(t) = tasks.get_mut(&task_id_for_spawn) {
                        t.state = A2ATaskState::Working;
                        t.metadata = Some(json!({
                            "abathur:federation": {
                                "delegated_to": cerebrate_id,
                                "task_id": fed_task_id.to_string(),
                            }
                        }));
                        t.updated_at = Utc::now();
                    }
                }
                Err(e) => {
                    let mut tasks = state_for_spawn.tasks.write().await;
                    if let Some(t) = tasks.get_mut(&task_id_for_spawn) {
                        t.state = A2ATaskState::Failed;
                        t.metadata = Some(json!({
                            "error": format!("Federation delegation failed: {}", e),
                        }));
                        t.updated_at = Utc::now();
                    }
                }
            }
        });
    }

    let heartbeat_interval = Duration::from_millis(state.config.heartbeat_interval_ms);
    let state_clone = Arc::clone(&state);
    let task_id_clone = task_id.clone();

    // SSE stream that emits status updates as the task progresses
    let stream = stream::unfold(
        (state_clone, task_id_clone, None::<A2ATaskState>, 0u32),
        |(state, task_id, last_state, tick)| async move {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let tasks = state.tasks.read().await;
            if let Some(task) = tasks.get(&task_id) {
                if last_state != Some(task.state) {
                    let event_data = json!({
                        "type": "TaskStatusUpdate",
                        "taskId": task.id,
                        "status": {
                            "state": task.state,
                            "timestamp": task.updated_at.to_rfc3339(),
                        },
                        "metadata": task.metadata,
                        "final": matches!(
                            task.state,
                            A2ATaskState::Completed
                                | A2ATaskState::Failed
                                | A2ATaskState::Canceled
                        ),
                    });

                    let sse_event = Event::default()
                        .event("TaskStatusUpdate")
                        .data(event_data.to_string());

                    let is_terminal = matches!(
                        task.state,
                        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled
                    );

                    if is_terminal {
                        // Send final event, then end stream
                        return Some((
                            Ok(sse_event),
                            (state.clone(), task_id, Some(task.state), tick + 1),
                        ));
                    }

                    return Some((
                        Ok(sse_event),
                        (state.clone(), task_id, Some(task.state), tick + 1),
                    ));
                }

                // Terminal state already reported — end the stream
                if last_state.is_some_and(|s| {
                    matches!(
                        s,
                        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled
                    )
                }) {
                    return None;
                }

                // Heartbeat
                Some((
                    Ok(Event::default().comment("heartbeat")),
                    (state.clone(), task_id, last_state, tick + 1),
                ))
            } else {
                None
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(heartbeat_interval)))
}

pub(super) async fn handle_tasks_get(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: TaskGetParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    let tasks = state.tasks.read().await;
    let task = match tasks.get(&params.id) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotFound,
                Some(json!({"taskId": params.id})),
            ));
        }
    };

    let history = if let Some(len) = params.history_length {
        let start = task.history.len().saturating_sub(len as usize);
        Some(task.history[start..].to_vec())
    } else {
        Some(task.history.clone())
    };

    let response = A2ATask {
        id: task.id.clone(),
        session_id: task.session_id.clone(),
        status: A2ATaskStatus {
            state: task.state,
            message: None,
            timestamp: Some(task.updated_at.to_rfc3339()),
        },
        history,
        artifacts: if task.artifacts.is_empty() {
            None
        } else {
            Some(task.artifacts.clone())
        },
        metadata: task.metadata.clone(),
    };

    let value = match serde_json::to_value(response) {
        Ok(v) => v,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InternalError,
                Some(json!({"message": format!("Failed to serialize task: {}", e)})),
            ))
        }
    };
    Json(JsonRpcResponse::success(request.id, value))
}

pub(super) async fn handle_tasks_cancel(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let params: TaskCancelParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    let mut tasks = state.tasks.write().await;
    let task = match tasks.get_mut(&params.id) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotFound,
                Some(json!({"taskId": params.id})),
            ));
        }
    };

    // Check if task can be canceled
    match task.state {
        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotCancelable,
                Some(json!({
                    "taskId": params.id,
                    "currentState": task.state.to_string()
                })),
            ));
        }
        _ => {}
    }

    task.state = A2ATaskState::Canceled;
    task.updated_at = Utc::now();

    let response = A2ATask {
        id: task.id.clone(),
        session_id: task.session_id.clone(),
        status: A2ATaskStatus {
            state: task.state,
            message: None,
            timestamp: Some(task.updated_at.to_rfc3339()),
        },
        history: None,
        artifacts: None,
        metadata: task.metadata.clone(),
    };

    let value = match serde_json::to_value(response) {
        Ok(v) => v,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InternalError,
                Some(json!({"message": format!("Failed to serialize task: {}", e)})),
            ))
        }
    };
    Json(JsonRpcResponse::success(request.id, value))
}

pub(super) async fn handle_push_notification_config(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    if !state.config.enable_push_notifications {
        return Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::PushNotificationNotSupported,
            None,
        ));
    }

    let params: PushNotificationConfigParams = match serde_json::from_value(request.params.clone())
    {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    let mut tasks = state.tasks.write().await;
    let task = match tasks.get_mut(&params.id) {
        Some(t) => t,
        None => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::TaskNotFound,
                Some(json!({"taskId": params.id})),
            ));
        }
    };

    task.push_config = Some(params.config.clone());

    Json(JsonRpcResponse::success(
        request.id,
        json!({"taskId": params.id, "configured": true}),
    ))
}

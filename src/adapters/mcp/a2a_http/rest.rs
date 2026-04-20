//! REST endpoint handlers (alternative to JSON-RPC) and ancillary
//! discovery / delegation routes.
//!
//! Includes:
//! - `/health` and `/.well-known/agent.json`
//! - `/agents/*` REST agent discovery
//! - `/tasks/*` REST task CRUD + SSE streaming
//! - `/api/v1/delegations/*` delegation queue used by the orchestrator

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        Json,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::Utc;
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::models::a2a::A2AAgentCard;

use super::{
    A2ASkill, A2AState, A2ATask, A2ATaskState, A2ATaskStatus, ErrorResponse, InMemoryTask,
    PendingDelegation, TaskSendParams,
};

pub(super) async fn health_check() -> &'static str {
    "OK"
}

/// Handle GET /.well-known/agent.json — A2A standard agent card discovery.
pub(super) async fn handle_well_known_agent_card(
    State(state): State<Arc<A2AState>>,
) -> Json<crate::domain::models::a2a_protocol::A2AStandardAgentCard> {
    use crate::domain::models::a2a_protocol::*;

    let (swarm_id, display_name) = if let Some(ref fed) = state.federation_service {
        let config = fed.config();
        (config.swarm_id.clone(), config.display_name.clone())
    } else {
        (
            uuid::Uuid::new_v4().to_string(),
            "Abathur Swarm".to_string(),
        )
    };

    let url = format!("http://{}:{}", state.config.host, state.config.port);

    Json(A2AStandardAgentCard {
        id: swarm_id,
        name: display_name,
        description: "Abathur swarm orchestrator".to_string(),
        url,
        version: Some("0.3".to_string()),
        provider: Some(A2AProvider {
            organization: "Abathur".to_string(),
            url: None,
        }),
        capabilities: A2ACapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: false,
        },
        skills: vec![A2ASkill {
            id: "task-execution".to_string(),
            name: "Task Execution".to_string(),
            description: Some("Execute delegated tasks via AI agent orchestration".to_string()),
            tags: vec!["orchestration".to_string(), "delegation".to_string()],
            examples: vec![],
        }],
        security_schemes: vec![],
        default_input_modes: vec!["application/json".to_string()],
        default_output_modes: vec!["application/json".to_string()],
    })
}

/// Request to create a delegation.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDelegationRequest {
    sender_id: String,
    target_agent: String,
    task_description: String,
    parent_task_id: Option<Uuid>,
    goal_id: Option<Uuid>,
    #[serde(default = "default_priority")]
    priority: String,
}

fn default_priority() -> String {
    "normal".to_string()
}

/// Create a new delegation request.
pub(super) async fn create_delegation(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<CreateDelegationRequest>,
) -> Result<Json<PendingDelegation>, (StatusCode, String)> {
    let delegation = PendingDelegation {
        id: Uuid::new_v4(),
        sender_id: request.sender_id,
        target_agent: request.target_agent,
        task_description: request.task_description,
        parent_task_id: request.parent_task_id,
        goal_id: request.goal_id,
        priority: request.priority,
        created_at: Utc::now(),
        acknowledged: false,
    };

    let mut delegations = state.delegations.write().await;
    let id = delegation.id;
    delegations.insert(id, delegation.clone());

    Ok(Json(delegation))
}

/// List pending (unacknowledged) delegations.
pub(super) async fn list_pending_delegations(
    State(state): State<Arc<A2AState>>,
) -> Json<Vec<PendingDelegation>> {
    let delegations = state.delegations.read().await;
    let pending: Vec<PendingDelegation> = delegations
        .values()
        .filter(|d| !d.acknowledged)
        .cloned()
        .collect();
    Json(pending)
}

/// Acknowledge a delegation (mark as processed).
pub(super) async fn acknowledge_delegation(
    State(state): State<Arc<A2AState>>,
    Path(delegation_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut delegations = state.delegations.write().await;
    if let Some(delegation) = delegations.get_mut(&delegation_id) {
        delegation.acknowledged = true;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "Delegation not found".to_string()))
    }
}

// REST endpoints for agent discovery

pub(super) async fn list_agents(
    State(state): State<Arc<A2AState>>,
) -> Result<Json<Vec<A2AAgentCard>>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    Ok(Json(cards.values().cloned().collect()))
}

pub(super) async fn get_agent(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<A2AAgentCard>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    match cards.get(&agent_id) {
        Some(card) => Ok(Json(card.clone())),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Agent {} not found", agent_id),
                code: "AGENT_NOT_FOUND".to_string(),
            }),
        )),
    }
}

pub(super) async fn get_agent_card(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<A2AAgentCard>, (StatusCode, Json<ErrorResponse>)> {
    get_agent(State(state), Path(agent_id)).await
}

pub(super) async fn get_agent_skills(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<A2ASkill>>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    match cards.get(&agent_id) {
        Some(card) => {
            let skills: Vec<A2ASkill> = card
                .capabilities
                .iter()
                .map(|cap| A2ASkill {
                    id: cap.to_lowercase().replace(' ', "-"),
                    name: cap.clone(),
                    description: None,
                    tags: vec![],
                    examples: vec![],
                })
                .collect();
            Ok(Json(skills))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Agent {} not found", agent_id),
                code: "AGENT_NOT_FOUND".to_string(),
            }),
        )),
    }
}

// REST endpoints for task operations

pub(super) async fn create_task(
    State(state): State<Arc<A2AState>>,
    Json(params): Json<TaskSendParams>,
) -> Result<(StatusCode, Json<A2ATask>), (StatusCode, Json<ErrorResponse>)> {
    let new_task = InMemoryTask::from_params(&params);
    let response = new_task.to_a2a_task();

    let mut tasks = state.tasks.write().await;
    tasks.insert(new_task.id.clone(), new_task);

    Ok((StatusCode::CREATED, Json(response)))
}

pub(super) async fn get_task(
    State(state): State<Arc<A2AState>>,
    Path(task_id): Path<String>,
) -> Result<Json<A2ATask>, (StatusCode, Json<ErrorResponse>)> {
    let tasks = state.tasks.read().await;
    match tasks.get(&task_id) {
        Some(task) => Ok(Json(task.to_a2a_task())),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Task {} not found", task_id),
                code: "TASK_NOT_FOUND".to_string(),
            }),
        )),
    }
}

pub(super) async fn cancel_task(
    State(state): State<Arc<A2AState>>,
    Path(task_id): Path<String>,
) -> Result<Json<A2ATask>, (StatusCode, Json<ErrorResponse>)> {
    let mut tasks = state.tasks.write().await;
    let task = match tasks.get_mut(&task_id) {
        Some(t) => t,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Task {} not found", task_id),
                    code: "TASK_NOT_FOUND".to_string(),
                }),
            ));
        }
    };

    match task.state {
        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: format!(
                        "Task {} cannot be canceled in state {}",
                        task_id, task.state
                    ),
                    code: "TASK_NOT_CANCELABLE".to_string(),
                }),
            ));
        }
        _ => {}
    }

    task.state = A2ATaskState::Canceled;
    task.updated_at = Utc::now();

    Ok(Json(A2ATask {
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
    }))
}

/// Stream task updates via SSE.
pub(super) async fn stream_task(
    State(state): State<Arc<A2AState>>,
    Path(task_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<ErrorResponse>)> {
    if !state.config.enable_streaming {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Streaming not enabled".to_string(),
                code: "STREAMING_NOT_ENABLED".to_string(),
            }),
        ));
    }

    // Verify task exists
    {
        let tasks = state.tasks.read().await;
        if !tasks.contains_key(&task_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Task {} not found", task_id),
                    code: "TASK_NOT_FOUND".to_string(),
                }),
            ));
        }
    }

    let heartbeat_interval = Duration::from_millis(state.config.heartbeat_interval_ms);
    let state_clone = Arc::clone(&state);
    let task_id_clone = task_id.clone();

    // Create a stream that periodically checks task state
    let stream = stream::unfold(
        (state_clone, task_id_clone, None::<A2ATaskState>),
        |(state, task_id, last_state)| async move {
            // Wait a bit before next update
            tokio::time::sleep(Duration::from_millis(500)).await;

            let tasks = state.tasks.read().await;
            if let Some(task) = tasks.get(&task_id) {
                // Only send event if state changed
                if last_state != Some(task.state) {
                    let event = json!({
                        "type": "TaskStatusUpdate",
                        "taskId": task.id,
                        "status": {
                            "state": task.state,
                            "timestamp": task.updated_at.to_rfc3339()
                        }
                    });

                    let sse_event = Event::default()
                        .event("TaskStatusUpdate")
                        .data(event.to_string());

                    // If terminal state, end stream
                    let is_terminal = matches!(
                        task.state,
                        A2ATaskState::Completed | A2ATaskState::Failed | A2ATaskState::Canceled
                    );

                    if is_terminal {
                        return Some((Ok(sse_event), (state.clone(), task_id, Some(task.state))));
                    }

                    return Some((Ok(sse_event), (state.clone(), task_id, Some(task.state))));
                }

                // Send heartbeat if no state change
                Some((
                    Ok(Event::default().comment("heartbeat")),
                    (state.clone(), task_id, last_state),
                ))
            } else {
                None
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(heartbeat_interval)))
}


//! `federation/*` JSON-RPC handlers.
//!
//! Each function is a free function taking `Arc<A2AState>` plus the
//! parsed `JsonRpcRequest`, returning `Json<JsonRpcResponse>`. The
//! shared `require_federation` helper short-circuits with a
//! `UnsupportedOperation` error when the gateway has no
//! `FederationService` attached.

use axum::response::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

use super::{A2AErrorCode, A2AState, JsonRpcRequest, JsonRpcResponse};

/// Helper to get the federation service or return an error response.
pub(super) fn require_federation(
    state: &A2AState,
    request_id: &Option<Value>,
) -> Result<Arc<crate::services::federation::FederationService>, Box<Json<JsonRpcResponse>>> {
    state.federation_service.clone().ok_or_else(|| {
        Box::new(Json(JsonRpcResponse::error(
            request_id.clone(),
            A2AErrorCode::UnsupportedOperation,
            Some(json!({"message": "Federation is not enabled"})),
        )))
    })
}

/// Handle `federation/discover` — returns this swarm's federation card.
pub(super) async fn handle_federation_discover(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    let config = fed.config();
    Json(JsonRpcResponse::success(
        request.id,
        json!({
            "swarm_id": config.swarm_id,
            "display_name": config.display_name,
            "role": format!("{:?}", config.role),
            "capabilities": [],
            "heartbeat_interval_secs": config.heartbeat_interval_secs,
        }),
    ))
}

/// Handle `federation/register` — register a cerebrate with this swarm.
pub(super) async fn handle_federation_register(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        cerebrate_id: String,
        display_name: String,
        url: String,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    fed.register_cerebrate(&params.cerebrate_id, &params.display_name, &params.url)
        .await;
    if let Err(e) = fed.connect(&params.cerebrate_id).await {
        return Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::InternalError,
            Some(json!({"message": e})),
        ));
    }

    Json(JsonRpcResponse::success(
        request.id,
        json!({"status": "registered", "cerebrate_id": params.cerebrate_id}),
    ))
}

/// Handle `federation/disconnect` — disconnect a cerebrate.
pub(super) async fn handle_federation_disconnect(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        cerebrate_id: String,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    if let Err(e) = fed.disconnect(&params.cerebrate_id).await {
        return Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::InternalError,
            Some(json!({"message": e})),
        ));
    }

    Json(JsonRpcResponse::success(
        request.id,
        json!({"status": "disconnected", "cerebrate_id": params.cerebrate_id}),
    ))
}

/// Handle `federation/delegate` — delegate a task to a cerebrate.
pub(super) async fn handle_federation_delegate(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    let envelope: crate::domain::models::a2a::FederationTaskEnvelope =
        match serde_json::from_value(request.params.clone()) {
            Ok(e) => e,
            Err(e) => {
                return Json(JsonRpcResponse::error(
                    request.id,
                    A2AErrorCode::InvalidParams,
                    Some(json!({"message": e.to_string()})),
                ));
            }
        };

    match fed.delegate(envelope).await {
        Ok(cerebrate_id) => Json(JsonRpcResponse::success(
            request.id,
            json!({"status": "delegated", "cerebrate_id": cerebrate_id}),
        )),
        Err(e) => Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::InternalError,
            Some(json!({"message": e})),
        )),
    }
}

/// Handle `federation/accept` — cerebrate accepts a delegated task.
pub(super) async fn handle_federation_accept(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        task_id: Uuid,
        cerebrate_id: String,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    fed.handle_accept(params.task_id, &params.cerebrate_id)
        .await;
    Json(JsonRpcResponse::success(
        request.id,
        json!({"status": "accepted"}),
    ))
}

/// Handle `federation/reject` — cerebrate rejects a delegated task.
pub(super) async fn handle_federation_reject(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        task_id: Uuid,
        cerebrate_id: String,
        reason: String,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    let decision = fed
        .handle_reject(params.task_id, &params.cerebrate_id, &params.reason)
        .await;
    Json(JsonRpcResponse::success(
        request.id,
        json!({
            "status": "rejected",
            "decision": format!("{:?}", decision),
        }),
    ))
}

/// Handle `federation/progress` — progress update from a cerebrate.
pub(super) async fn handle_federation_progress(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        task_id: Uuid,
        cerebrate_id: String,
        #[serde(default)]
        phase: String,
        #[serde(default)]
        progress_pct: f64,
        #[serde(default)]
        summary: String,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    fed.handle_progress(
        params.task_id,
        &params.cerebrate_id,
        &params.phase,
        params.progress_pct,
        &params.summary,
    )
    .await;

    Json(JsonRpcResponse::success(
        request.id,
        json!({"status": "acknowledged"}),
    ))
}

/// Handle `federation/result` — final result from a cerebrate.
pub(super) async fn handle_federation_result(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    let result: crate::domain::models::a2a::FederationResult =
        match serde_json::from_value(request.params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return Json(JsonRpcResponse::error(
                    request.id,
                    A2AErrorCode::InvalidParams,
                    Some(json!({"message": e.to_string()})),
                ));
            }
        };

    let ctx = crate::services::federation::traits::ParentContext::default();
    let reactions = fed.handle_result(result, ctx).await;

    Json(JsonRpcResponse::success(
        request.id,
        json!({
            "status": "processed",
            "reactions_count": reactions.len(),
        }),
    ))
}

/// Handle `federation/heartbeat` — heartbeat from a cerebrate.
pub(super) async fn handle_federation_heartbeat(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        cerebrate_id: String,
        #[serde(default)]
        load: f64,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    fed.handle_heartbeat(&params.cerebrate_id, params.load)
        .await;
    Json(JsonRpcResponse::success(
        request.id,
        json!({"status": "ok"}),
    ))
}

/// Handle `federation/reconcile` — reconcile in-flight tasks after reconnection.
pub(super) async fn handle_federation_reconcile(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let fed = match require_federation(&state, &request.id) {
        Ok(f) => f,
        Err(e) => return *e,
    };

    #[derive(Deserialize)]
    struct Params {
        cerebrate_id: String,
        #[serde(default)]
        in_flight_task_ids: Vec<Uuid>,
    }

    let params: Params = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::InvalidParams,
                Some(json!({"message": e.to_string()})),
            ));
        }
    };

    // Get our view of in-flight tasks for this cerebrate
    let our_in_flight = fed.in_flight_for_cerebrate(&params.cerebrate_id).await;

    // Find tasks we think are in-flight but the cerebrate doesn't know about
    let orphaned: Vec<Uuid> = our_in_flight
        .iter()
        .filter(|tid| !params.in_flight_task_ids.contains(tid))
        .copied()
        .collect();

    // Find tasks the cerebrate knows about but we don't
    let unknown: Vec<Uuid> = params
        .in_flight_task_ids
        .iter()
        .filter(|tid| !our_in_flight.contains(tid))
        .copied()
        .collect();

    Json(JsonRpcResponse::success(
        request.id,
        json!({
            "status": "reconciled",
            "our_in_flight": our_in_flight.len(),
            "their_in_flight": params.in_flight_task_ids.len(),
            "orphaned_count": orphaned.len(),
            "unknown_count": unknown.len(),
        }),
    ))
}

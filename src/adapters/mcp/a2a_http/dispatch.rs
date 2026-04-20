//! Unified JSON-RPC dispatch.
//!
//! Pre-split, the gateway maintained two parallel match arms — one in
//! `handle_jsonrpc` (request) and one in `handle_jsonrpc_stream` (SSE).
//! That meant adding or renaming a method required edits in two places.
//!
//! The dispatch is now centralised here. The two HTTP entrypoints share
//! [`validate_jsonrpc_envelope`] for protocol-version checks, and
//! [`dispatch_jsonrpc`] is the single source of truth for the
//! method → handler mapping used by the non-streaming endpoint. The SSE
//! endpoint only supports `tasks/sendSubscribe`, but it likewise calls
//! into [`super::tasks::handle_tasks_send_subscribe`] from a single
//! routing site (see `handle_jsonrpc_stream` in the parent module).

use axum::response::Json;
use serde_json::json;
use std::sync::Arc;

use super::{
    A2AErrorCode, A2AState, JsonRpcRequest, JsonRpcResponse, agent, federation, tasks,
};

/// Validate the JSON-RPC envelope (currently only the `jsonrpc` version).
///
/// Returns `None` for valid `2.0` envelopes; returns `Some(error_response)`
/// suitable for either the non-streaming or streaming endpoint to wrap when
/// the envelope is malformed.
pub(super) fn validate_jsonrpc_envelope(request: &JsonRpcRequest) -> Option<JsonRpcResponse> {
    if request.jsonrpc != "2.0" {
        return Some(JsonRpcResponse::error(
            request.id.clone(),
            A2AErrorCode::ParseError,
            Some(json!({"message": "Invalid JSON-RPC version"})),
        ));
    }
    None
}

/// Dispatch a validated JSON-RPC request to its handler.
///
/// The non-streaming endpoint (`POST /` and `POST /rpc`) calls this
/// directly. The streaming endpoint handles `tasks/sendSubscribe`
/// separately because its return type is an SSE stream rather than a
/// JSON response. Every method name → handler mapping that previously
/// existed in `handle_jsonrpc` is preserved here.
pub(super) async fn dispatch_jsonrpc(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    match request.method.as_str() {
        "tasks/send" => tasks::handle_tasks_send(state, request).await,
        "tasks/sendSubscribe" => {
            // tasks/sendSubscribe requires SSE; on the non-streaming JSON-RPC
            // endpoint we fall back to a synchronous send and tell the caller
            // to use /rpc/stream for real SSE.
            tasks::handle_tasks_send(state, request).await
        }
        "tasks/get" => tasks::handle_tasks_get(state, request).await,
        "tasks/cancel" => tasks::handle_tasks_cancel(state, request).await,
        "tasks/pushNotificationConfig/set" => {
            tasks::handle_push_notification_config(state, request).await
        }
        "agent/card" => agent::handle_agent_card(state, request).await,
        "agent/skills" => agent::handle_agent_skills(state, request).await,
        // Federation methods
        "federation/discover" => federation::handle_federation_discover(state, request).await,
        "federation/register" => federation::handle_federation_register(state, request).await,
        "federation/disconnect" => federation::handle_federation_disconnect(state, request).await,
        "federation/delegate" => federation::handle_federation_delegate(state, request).await,
        "federation/accept" => federation::handle_federation_accept(state, request).await,
        "federation/reject" => federation::handle_federation_reject(state, request).await,
        "federation/progress" => federation::handle_federation_progress(state, request).await,
        "federation/result" => federation::handle_federation_result(state, request).await,
        "federation/heartbeat" => federation::handle_federation_heartbeat(state, request).await,
        "federation/reconcile" => federation::handle_federation_reconcile(state, request).await,
        _ => Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::MethodNotFound,
            Some(json!({"method": request.method})),
        )),
    }
}

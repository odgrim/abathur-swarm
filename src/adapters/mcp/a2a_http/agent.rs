//! `agent/*` JSON-RPC handlers — agent card and skills retrieval.

use axum::response::Json;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{A2AErrorCode, A2ASkill, A2AState, JsonRpcRequest, JsonRpcResponse};

pub(super) async fn handle_agent_card(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    // Get agent_id from params if provided
    let agent_id: Option<String> = serde_json::from_value(request.params.clone())
        .ok()
        .and_then(|v: Value| v.get("agentId").and_then(|a| a.as_str()).map(String::from));

    let cards = state.agent_cards.read().await;

    if let Some(id) = agent_id {
        match cards.get(&id) {
            Some(card) => match serde_json::to_value(card) {
                Ok(v) => Json(JsonRpcResponse::success(request.id, v)),
                Err(e) => Json(JsonRpcResponse::error(
                    request.id,
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Failed to serialize agent card: {}", e)})),
                )),
            },
            None => Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::AgentNotFound,
                Some(json!({"agentId": id})),
            )),
        }
    } else {
        // Return first available agent card
        match cards.values().next() {
            Some(card) => match serde_json::to_value(card) {
                Ok(v) => Json(JsonRpcResponse::success(request.id, v)),
                Err(e) => Json(JsonRpcResponse::error(
                    request.id,
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Failed to serialize agent card: {}", e)})),
                )),
            },
            None => Json(JsonRpcResponse::error(
                request.id,
                A2AErrorCode::AgentNotFound,
                Some(json!({"message": "No agents registered"})),
            )),
        }
    }
}

pub(super) async fn handle_agent_skills(
    state: Arc<A2AState>,
    request: JsonRpcRequest,
) -> Json<JsonRpcResponse> {
    let agent_id: Option<String> = serde_json::from_value(request.params.clone())
        .ok()
        .and_then(|v: Value| v.get("agentId").and_then(|a| a.as_str()).map(String::from));

    let cards = state.agent_cards.read().await;

    let card = if let Some(id) = &agent_id {
        cards.get(id)
    } else {
        cards.values().next()
    };

    match card {
        Some(c) => {
            // Convert capabilities to skills
            let skills: Vec<A2ASkill> = c
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

            match serde_json::to_value(skills) {
                Ok(v) => Json(JsonRpcResponse::success(request.id, v)),
                Err(e) => Json(JsonRpcResponse::error(
                    request.id,
                    A2AErrorCode::InternalError,
                    Some(json!({"message": format!("Failed to serialize skills: {}", e)})),
                )),
            }
        }
        None => Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::AgentNotFound,
            Some(json!({"agentId": agent_id})),
        )),
    }
}

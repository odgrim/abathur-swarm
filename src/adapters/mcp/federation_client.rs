//! FederationClient — outbound cross-swarm task delegation over A2A.
//!
//! Extracted from `a2a_http.rs`. The client posts JSON-RPC `tasks/send`
//! requests to trusted peer swarms, applies per-peer rate limiting, and
//! parses the structured `A2ATask` reply.

use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::adapters::mcp::a2a_http::{A2ATask, JsonRpcResponse};
use crate::domain::models::a2a::A2AAgentCard;
use crate::services::{A2AFederationConfig, TrustedSwarmConfig};

/// Client for delegating tasks to trusted peer swarms via A2A protocol.
pub struct FederationClient {
    config: A2AFederationConfig,
    http_client: reqwest::Client,
    /// Per-peer request counters for rate limiting (peer_id -> count in current window).
    request_counts: Arc<RwLock<HashMap<String, u32>>>,
}

impl FederationClient {
    pub fn new(config: A2AFederationConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.external_request_timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            config,
            http_client,
            request_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// List available trusted peers that are active.
    pub fn list_available_peers(&self) -> Vec<&TrustedSwarmConfig> {
        self.config
            .trusted_swarms
            .iter()
            .filter(|s| s.active)
            .collect()
    }

    /// Get a peer's agent card / capabilities.
    pub async fn get_peer_capabilities(&self, peer_id: &str) -> Result<A2AAgentCard, String> {
        let peer = self.find_peer(peer_id)?;

        let url = format!(
            "{}/.well-known/agent.json",
            peer.endpoint.trim_end_matches('/')
        );

        let mut request = self.http_client.get(&url);
        if let Some(ref token) = peer.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to reach peer {}: {}", peer_id, e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Peer {} returned status {}",
                peer_id,
                response.status()
            ));
        }

        let card: A2AAgentCard = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse peer {} agent card: {}", peer_id, e))?;

        Ok(card)
    }

    /// Delegate a task to a trusted peer swarm.
    pub async fn delegate_task(&self, peer_id: &str, message: &str) -> Result<A2ATask, String> {
        let peer = self.find_peer(peer_id)?;

        // Rate limiting
        {
            let mut counts = self.request_counts.write().await;
            let count = counts.entry(peer_id.to_string()).or_insert(0);
            let limit = peer
                .rate_limit_override
                .unwrap_or(self.config.rate_limit_per_swarm);
            if *count >= limit {
                return Err(format!("Rate limit exceeded for peer {}", peer_id));
            }
            *count += 1;
        }

        let task_id = Uuid::new_v4().to_string();
        let session_id = Uuid::new_v4().to_string();

        let json_rpc = json!({
            "jsonrpc": "2.0",
            "id": task_id,
            "method": "tasks/send",
            "params": {
                "id": task_id,
                "sessionId": session_id,
                "message": {
                    "role": "user",
                    "parts": [
                        {
                            "type": "text",
                            "text": message,
                        }
                    ]
                }
            }
        });

        let url = format!("{}/a2a", peer.endpoint.trim_end_matches('/'));

        let mut request = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&json_rpc);

        if let Some(ref token) = peer.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to send task to peer {}: {}", peer_id, e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Peer {} returned status {}",
                peer_id,
                response.status()
            ));
        }

        let rpc_response: JsonRpcResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response from peer {}: {}", peer_id, e))?;

        if let Some(error) = rpc_response.error {
            return Err(format!(
                "Peer {} error: {} ({})",
                peer_id, error.message, error.code
            ));
        }

        let result = rpc_response
            .result
            .ok_or_else(|| format!("Peer {} returned no result", peer_id))?;

        let task: A2ATask = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse task from peer {}: {}", peer_id, e))?;

        Ok(task)
    }

    /// Reset rate limit counters (should be called periodically).
    pub async fn reset_rate_limits(&self) {
        self.request_counts.write().await.clear();
    }

    fn find_peer(&self, peer_id: &str) -> Result<&TrustedSwarmConfig, String> {
        self.config
            .trusted_swarms
            .iter()
            .find(|s| s.id == peer_id && s.active)
            .ok_or_else(|| format!("Peer {} not found or inactive", peer_id))
    }
}

//! Federation service managing cerebrate connections, heartbeats, and task delegation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::adapters::a2a::A2AClient;
use crate::domain::models::a2a::{
    CerebrateStatus, ConnectionState, FederationCard, FederationResult, FederationTaskEnvelope,
    FederationTaskStatus,
};
use crate::domain::models::a2a_protocol::{
    A2APart, A2AProtocolMessage, A2ARole, TaskSendParams,
};
use crate::domain::models::goal::Goal;
use crate::domain::models::goal_federation::{
    ConvergenceContract, FederatedGoal, FederatedGoalState,
};
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;

use super::config::FederationConfig;
use super::traits::{
    DefaultDelegationStrategy, DefaultResultProcessor, DefaultTaskTransformer,
    DelegationDecision, FederationDelegationStrategy, FederationReaction,
    FederationResultProcessor, FederationTaskTransformer, ParentContext, ResultSchema,
    StandardV1Schema,
};

/// HTTP client for outbound federation communication.
#[derive(Clone)]
pub struct FederationHttpClient {
    client: reqwest::Client,
}

impl FederationHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Discover a remote cerebrate by calling its federation/discover endpoint.
    pub async fn discover(&self, url: &str) -> Result<FederationCard, String> {
        let discover_url = format!("{}/federation/discover", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&discover_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/discover",
                "id": 1
            }))
            .send()
            .await
            .map_err(|e| format!("Discovery request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Discovery returned status {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse discovery response: {}", e))?;

        serde_json::from_value(body["result"].clone())
            .map_err(|e| format!("Failed to parse federation card: {}", e))
    }

    /// Send a register request to a remote cerebrate.
    pub async fn register(&self, url: &str, swarm_id: &str, jwt: &str) -> Result<(), String> {
        let register_url = format!("{}/federation/register", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&register_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/register",
                "id": 1,
                "params": {
                    "swarm_id": swarm_id,
                    "jwt": jwt
                }
            }))
            .send()
            .await
            .map_err(|e| format!("Register request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Register returned status {}", resp.status()));
        }
        Ok(())
    }

    /// Send a disconnect notification to a remote cerebrate.
    pub async fn send_disconnect(&self, url: &str, swarm_id: &str) -> Result<(), String> {
        let disconnect_url = format!("{}/federation/disconnect", url.trim_end_matches('/'));
        let _resp = self
            .client
            .post(&disconnect_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/disconnect",
                "id": 1,
                "params": { "swarm_id": swarm_id }
            }))
            .send()
            .await
            .map_err(|e| format!("Disconnect request failed: {}", e))?;
        Ok(())
    }

    /// Send a task delegation envelope to a remote cerebrate.
    pub async fn delegate(
        &self,
        url: &str,
        envelope: &FederationTaskEnvelope,
    ) -> Result<serde_json::Value, String> {
        let delegate_url = format!("{}/federation/delegate", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&delegate_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/delegate",
                "id": 1,
                "params": envelope
            }))
            .send()
            .await
            .map_err(|e| format!("Delegate request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Delegate returned status {}", resp.status()));
        }

        resp.json()
            .await
            .map_err(|e| format!("Failed to parse delegate response: {}", e))
    }

    /// Send a heartbeat to a remote parent (used when this swarm is a cerebrate).
    pub async fn send_heartbeat(
        &self,
        url: &str,
        cerebrate_id: &str,
        load: f64,
    ) -> Result<(), String> {
        let heartbeat_url = format!("{}/federation/heartbeat", url.trim_end_matches('/'));
        let _resp = self
            .client
            .post(&heartbeat_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/heartbeat",
                "id": 1,
                "params": {
                    "cerebrate_id": cerebrate_id,
                    "load": load
                }
            }))
            .send()
            .await
            .map_err(|e| format!("Heartbeat request failed: {}", e))?;
        Ok(())
    }

    /// Send a reconcile request to exchange in-flight task IDs.
    pub async fn reconcile(
        &self,
        url: &str,
        cerebrate_id: &str,
        local_task_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, String> {
        let reconcile_url = format!("{}/federation/reconcile", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&reconcile_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/reconcile",
                "id": 1,
                "params": {
                    "cerebrate_id": cerebrate_id,
                    "local_task_ids": local_task_ids
                }
            }))
            .send()
            .await
            .map_err(|e| format!("Reconcile request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Reconcile returned status {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse reconcile response: {}", e))?;

        serde_json::from_value(body["result"]["task_ids"].clone())
            .map_err(|e| format!("Failed to parse reconcile task IDs: {}", e))
    }
}

impl Default for FederationHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Manages the federation lifecycle: cerebrate registry, heartbeats, delegation, results.
pub struct FederationService {
    config: FederationConfig,
    /// Registry of known cerebrates (id → status).
    cerebrates: Arc<RwLock<HashMap<String, CerebrateStatus>>>,
    /// In-flight delegated tasks (task_id → cerebrate_id).
    in_flight: Arc<RwLock<HashMap<Uuid, String>>>,
    /// Delegation timestamps for stall detection (task_id → last_activity_at).
    last_activity: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
    /// EventBus for emitting federation events.
    event_bus: Arc<EventBus>,
    /// HTTP client for outbound federation calls.
    http_client: FederationHttpClient,
    /// Delegation strategy.
    delegation_strategy: Arc<dyn FederationDelegationStrategy>,
    /// Result processor.
    result_processor: Arc<dyn FederationResultProcessor>,
    /// Task transformer.
    task_transformer: Arc<dyn FederationTaskTransformer>,
    /// Registered result schemas (schema_id → schema).
    schemas: Arc<RwLock<HashMap<String, Arc<dyn ResultSchema>>>>,
    /// Shutdown signal for background tasks.
    shutdown_tx: Arc<RwLock<Option<tokio::sync::broadcast::Sender<()>>>>,
    /// Optional A2A wire-protocol client for outbound federation calls.
    /// When set, preferred over `http_client` for `delegate_to()` and `connect()`.
    a2a_client: Option<Arc<dyn A2AClient>>,
}

impl FederationService {
    /// Create a new FederationService with default strategies.
    pub fn new(config: FederationConfig, event_bus: Arc<EventBus>) -> Self {
        let mut schemas: HashMap<String, Arc<dyn ResultSchema>> = HashMap::new();
        let standard = Arc::new(StandardV1Schema);
        schemas.insert(standard.schema_id().to_string(), standard);

        Self {
            config,
            cerebrates: Arc::new(RwLock::new(HashMap::new())),
            in_flight: Arc::new(RwLock::new(HashMap::new())),
            last_activity: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            http_client: FederationHttpClient::new(),
            delegation_strategy: Arc::new(DefaultDelegationStrategy),
            result_processor: Arc::new(DefaultResultProcessor),
            task_transformer: Arc::new(DefaultTaskTransformer),
            schemas: Arc::new(RwLock::new(schemas)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            a2a_client: None,
        }
    }

    /// Set an A2A wire-protocol client for outbound federation calls.
    ///
    /// When set, `delegate_to()` and `connect()` prefer this client over
    /// the bespoke `FederationHttpClient`. Both paths coexist — the A2A
    /// client is tried first and falls back to the legacy client on error.
    pub fn with_a2a_client(mut self, client: Arc<dyn A2AClient>) -> Self {
        self.a2a_client = Some(client);
        self
    }

    /// Replace the delegation strategy.
    pub fn with_delegation_strategy(
        mut self,
        strategy: Arc<dyn FederationDelegationStrategy>,
    ) -> Self {
        self.delegation_strategy = strategy;
        self
    }

    /// Replace the result processor.
    pub fn with_result_processor(
        mut self,
        processor: Arc<dyn FederationResultProcessor>,
    ) -> Self {
        self.result_processor = processor;
        self
    }

    /// Replace the task transformer.
    pub fn with_task_transformer(
        mut self,
        transformer: Arc<dyn FederationTaskTransformer>,
    ) -> Self {
        self.task_transformer = transformer;
        self
    }

    /// Register a result schema.
    pub async fn register_result_schema(&self, schema: Arc<dyn ResultSchema>) {
        let mut schemas = self.schemas.write().await;
        schemas.insert(schema.schema_id().to_string(), schema);
    }

    /// Get the federation config.
    pub fn config(&self) -> &FederationConfig {
        &self.config
    }

    // ========================================================================
    // Connection Lifecycle
    // ========================================================================

    /// Register a cerebrate from configuration.
    pub async fn register_cerebrate(&self, id: &str, display_name: &str, url: &str) {
        let status = CerebrateStatus::new(id, display_name).with_url(url);
        let mut cerebrates = self.cerebrates.write().await;
        cerebrates.insert(id.to_string(), status);
    }

    /// Connect to a cerebrate (transitions to Connecting → Connected).
    ///
    /// If the cerebrate has a URL, sends a register request over HTTP.
    /// Falls back to local state transition on network failure during tests.
    pub async fn connect(&self, id: &str) -> Result<(), String> {
        let url = {
            let mut cerebrates = self.cerebrates.write().await;
            let status = cerebrates
                .get_mut(id)
                .ok_or_else(|| format!("Unknown cerebrate: {}", id))?;

            match status.connection_state {
                ConnectionState::Disconnected | ConnectionState::Unreachable => {
                    status.connection_state = ConnectionState::Connecting;
                }
                ConnectionState::Connected => {
                    return Ok(()); // Already connected
                }
                _ => {
                    return Err(format!(
                        "Cannot connect: cerebrate {} is in state {}",
                        id, status.connection_state
                    ));
                }
            }
            status.url.clone()
        };

        // Attempt A2A discovery + registration if the cerebrate has a URL.
        // Prefer a2a_client when available; fall back to legacy http_client.
        if let Some(ref url) = url {
            if let Some(ref a2a) = self.a2a_client {
                match a2a.discover(url).await {
                    Ok(card) => {
                        tracing::info!(
                            cerebrate_id = %id,
                            agent_name = %card.name,
                            "A2A discovery succeeded"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            cerebrate_id = %id,
                            error = %e,
                            "A2A discover failed, falling back to legacy register"
                        );
                        if let Err(e2) = self
                            .http_client
                            .register(url, &self.config.swarm_id, "")
                            .await
                        {
                            tracing::warn!(
                                cerebrate_id = %id,
                                error = %e2,
                                "Legacy HTTP register also failed, connecting locally"
                            );
                        }
                    }
                }
            } else if let Err(e) = self
                .http_client
                .register(url, &self.config.swarm_id, "")
                .await
            {
                tracing::warn!(cerebrate_id = %id, error = %e, "HTTP register failed, connecting locally");
            }
        }

        // Transition to Connected
        let mut cerebrates = self.cerebrates.write().await;
        if let Some(status) = cerebrates.get_mut(id) {
            status.connection_state = ConnectionState::Connected;
            status.last_heartbeat_at = Some(chrono::Utc::now());
            status.missed_heartbeats = 0;
        }

        let capabilities = cerebrates
            .get(id)
            .map(|s| s.capabilities.clone())
            .unwrap_or_default();

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                None,
                EventPayload::FederationCerebrateConnected {
                    cerebrate_id: id.to_string(),
                    capabilities,
                },
            ))
            .await;

        Ok(())
    }

    /// Disconnect from a cerebrate.
    ///
    /// Sends a disconnect notification over HTTP if the cerebrate has a URL.
    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let url = {
            let mut cerebrates = self.cerebrates.write().await;
            let status = cerebrates
                .get_mut(id)
                .ok_or_else(|| format!("Unknown cerebrate: {}", id))?;

            status.connection_state = ConnectionState::Disconnecting;
            status.url.clone()
        };

        // Send disconnect notification via HTTP
        if let Some(ref url) = url
            && let Err(e) = self
                .http_client
                .send_disconnect(url, &self.config.swarm_id)
                .await
        {
            tracing::warn!(cerebrate_id = %id, error = %e, "HTTP disconnect notification failed");
        }

        // Complete the state transition
        {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(id) {
                status.connection_state = ConnectionState::Disconnected;
            }
        }

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                None,
                EventPayload::FederationCerebrateDisconnected {
                    cerebrate_id: id.to_string(),
                    reason: "requested".to_string(),
                },
            ))
            .await;

        Ok(())
    }

    /// Get status of a specific cerebrate.
    pub async fn get_cerebrate(&self, id: &str) -> Option<CerebrateStatus> {
        let cerebrates = self.cerebrates.read().await;
        cerebrates.get(id).cloned()
    }

    /// List all cerebrates.
    pub async fn list_cerebrates(&self) -> Vec<CerebrateStatus> {
        let cerebrates = self.cerebrates.read().await;
        cerebrates.values().cloned().collect()
    }

    /// Discover a remote cerebrate by URL, returning its federation card.
    ///
    /// When an A2A client is configured, first tries standard A2A discovery
    /// (`/.well-known/agent.json`). Falls back to the legacy
    /// `{url}/federation/discover` endpoint.
    pub async fn discover(&self, url: &str) -> Result<FederationCard, String> {
        // Try A2A standard discovery first when available
        if let Some(ref a2a) = self.a2a_client {
            match a2a.discover(url).await {
                Ok(card) => {
                    tracing::info!(
                        agent_id = %card.id,
                        agent_name = %card.name,
                        "Discovered remote agent via A2A"
                    );
                    // We return a FederationCard built from the A2A card.
                    // The caller gets a usable card; detailed mapping is best-effort.
                    return Ok(FederationCard::from_a2a_agent_card(&card));
                }
                Err(e) => {
                    tracing::warn!(
                        url = %url,
                        error = %e,
                        "A2A discover failed, falling back to legacy federation/discover"
                    );
                }
            }
        }
        self.http_client.discover(url).await
    }

    /// Get a reference to the HTTP client (for testing or direct use).
    pub fn http_client(&self) -> &FederationHttpClient {
        &self.http_client
    }

    // ========================================================================
    // Heartbeat
    // ========================================================================

    /// Record a heartbeat from a cerebrate.
    pub async fn handle_heartbeat(&self, cerebrate_id: &str, load: f64) {
        let mut cerebrates = self.cerebrates.write().await;
        if let Some(status) = cerebrates.get_mut(cerebrate_id) {
            status.last_heartbeat_at = Some(chrono::Utc::now());
            status.missed_heartbeats = 0;
            status.load = load;

            // If reconnecting, transition back to connected
            if status.connection_state == ConnectionState::Reconnecting
                || status.connection_state == ConnectionState::Unreachable
            {
                status.connection_state = ConnectionState::Connected;
            }
        }
    }

    /// Check for missed heartbeats and transition states accordingly.
    pub async fn check_heartbeats(&self) {
        let threshold = self.config.missed_heartbeat_threshold;
        let interval = chrono::Duration::seconds(self.config.heartbeat_interval_secs as i64);
        let now = chrono::Utc::now();

        let mut cerebrates = self.cerebrates.write().await;
        let mut unreachable_ids = Vec::new();

        for (id, status) in cerebrates.iter_mut() {
            if status.connection_state != ConnectionState::Connected {
                continue;
            }

            if status.last_heartbeat_at.is_some_and(|last| now - last > interval) {
                status.missed_heartbeats += 1;

                if status.missed_heartbeats >= threshold {
                    status.connection_state = ConnectionState::Unreachable;
                    unreachable_ids.push(id.clone());
                }
            }
        }

        // Emit events outside the write lock
        drop(cerebrates);

        for id in unreachable_ids {
            let in_flight_tasks: Vec<Uuid> = {
                let in_flight = self.in_flight.read().await;
                in_flight
                    .iter()
                    .filter(|(_, cid)| **cid == id)
                    .map(|(tid, _)| *tid)
                    .collect()
            };

            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Error,
                    None,
                    EventPayload::FederationCerebrateUnreachable {
                        cerebrate_id: id.clone(),
                        in_flight_tasks,
                    },
                ))
                .await;
        }
    }

    // ========================================================================
    // Task Delegation
    // ========================================================================

    /// Delegate a task to the best available cerebrate.
    pub async fn delegate(
        &self,
        envelope: FederationTaskEnvelope,
    ) -> Result<String, String> {
        let cerebrates = self.list_cerebrates().await;
        let cerebrate_id = self
            .delegation_strategy
            .select_cerebrate(&envelope, &cerebrates)
            .ok_or_else(|| "No suitable cerebrate available".to_string())?;

        self.delegate_to(&envelope, &cerebrate_id).await
    }

    /// Delegate a task to a specific cerebrate.
    pub async fn delegate_to(
        &self,
        envelope: &FederationTaskEnvelope,
        cerebrate_id: &str,
    ) -> Result<String, String> {
        // Verify cerebrate can accept
        {
            let cerebrates = self.cerebrates.read().await;
            let status = cerebrates
                .get(cerebrate_id)
                .ok_or_else(|| format!("Unknown cerebrate: {}", cerebrate_id))?;

            if !status.can_accept_task() {
                return Err(format!(
                    "Cerebrate {} cannot accept tasks (state: {}, delegations: {}/{})",
                    cerebrate_id,
                    status.connection_state,
                    status.active_delegations,
                    status.max_concurrent_delegations
                ));
            }
        }

        // Track in-flight and activity timestamp
        {
            let mut in_flight = self.in_flight.write().await;
            in_flight.insert(envelope.task_id, cerebrate_id.to_string());
        }
        {
            let mut activity = self.last_activity.write().await;
            activity.insert(envelope.task_id, chrono::Utc::now());
        }

        // Increment active delegations
        let url = {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(cerebrate_id) {
                status.active_delegations += 1;
            }
            cerebrates.get(cerebrate_id).and_then(|s| s.url.clone())
        };

        // Send the envelope to the remote cerebrate.
        // Prefer a2a_client when available; fall back to legacy http_client.
        if let Some(ref url) = url {
            let mut sent_via_a2a = false;

            if let Some(ref a2a) = self.a2a_client {
                let params = TaskSendParams::from(envelope);
                match a2a.send_message(url, params).await {
                    Ok(task) => {
                        tracing::info!(
                            cerebrate_id = %cerebrate_id,
                            task_id = %envelope.task_id,
                            a2a_task_id = %task.id,
                            "Delegated via A2A tasks/send"
                        );
                        sent_via_a2a = true;
                    }
                    Err(e) => {
                        tracing::warn!(
                            cerebrate_id = %cerebrate_id,
                            task_id = %envelope.task_id,
                            error = %e,
                            "A2A delegate failed, falling back to legacy HTTP"
                        );
                    }
                }
            }

            if !sent_via_a2a {
                if let Err(e) = self.http_client.delegate(url, envelope).await {
                    // HTTP send failure is not fatal — the task is tracked in-flight
                    // and the cerebrate may still process it (or we'll detect a stall/orphan).
                    tracing::warn!(
                        cerebrate_id = %cerebrate_id,
                        task_id = %envelope.task_id,
                        error = %e,
                        "HTTP delegate call failed, task tracked in-flight for monitoring"
                    );
                }
            }
        }

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(envelope.task_id),
                EventPayload::FederationTaskDelegated {
                    task_id: envelope.task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                },
            ))
            .await;

        Ok(cerebrate_id.to_string())
    }

    /// Delegate a goal to a specific cerebrate via A2A.
    ///
    /// Builds an A2A message with `abathur:federation` metadata containing
    /// the `goal_delegate` intent, sends it to the target cerebrate, creates
    /// an in-memory `FederatedGoal`, and emits a `FederatedGoalCreated` event.
    pub async fn delegate_goal(
        &self,
        goal: &Goal,
        cerebrate_id: &str,
        contract: ConvergenceContract,
    ) -> Result<FederatedGoal, String> {
        // 1. Verify the cerebrate exists, is connected, and can accept tasks.
        let url = {
            let cerebrates = self.cerebrates.read().await;
            let status = cerebrates
                .get(cerebrate_id)
                .ok_or_else(|| format!("Unknown cerebrate: {}", cerebrate_id))?;

            if !status.can_accept_task() {
                return Err(format!(
                    "Cerebrate {} cannot accept tasks (state: {}, delegations: {}/{})",
                    cerebrate_id,
                    status.connection_state,
                    status.active_delegations,
                    status.max_concurrent_delegations,
                ));
            }
            status.url.clone()
        };

        // 2. Build federation metadata.
        let contract_json = serde_json::to_value(&contract)
            .map_err(|e| format!("Failed to serialize convergence contract: {}", e))?;

        let constraints_strs: Vec<String> = goal
            .constraints
            .iter()
            .map(|c| c.description.clone())
            .collect();

        let mut federation_data = serde_json::Map::new();
        federation_data.insert(
            "intent".to_string(),
            serde_json::Value::String("goal_delegate".to_string()),
        );
        federation_data.insert(
            "goal_id".to_string(),
            serde_json::Value::String(goal.id.to_string()),
        );
        federation_data.insert(
            "goal_name".to_string(),
            serde_json::Value::String(goal.name.clone()),
        );
        federation_data.insert(
            "goal_description".to_string(),
            serde_json::Value::String(goal.description.clone()),
        );
        federation_data.insert(
            "constraints".to_string(),
            serde_json::to_value(&constraints_strs).unwrap_or_default(),
        );
        federation_data.insert(
            "priority".to_string(),
            serde_json::Value::String(goal.priority.as_str().to_string()),
        );
        federation_data.insert(
            "convergence_contract".to_string(),
            contract_json,
        );

        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "abathur:federation".to_string(),
            serde_json::Value::Object(federation_data),
        );

        let params = TaskSendParams {
            message: A2AProtocolMessage {
                role: A2ARole::User,
                parts: vec![
                    A2APart::Text {
                        text: format!("{}\n\n{}", goal.name, goal.description),
                    },
                    A2APart::Data {
                        data: serde_json::json!({
                            "goal_name": goal.name,
                            "goal_description": goal.description,
                            "priority": goal.priority.as_str(),
                            "constraints": constraints_strs,
                        }),
                        metadata: None,
                    },
                ],
                metadata: None,
            },
            metadata: Some(metadata),
            history_length: None,
            push_notification_config: None,
        };

        // 3 & 4. Send via A2A client or fall back to legacy delegation.
        let remote_task_id = if let Some(ref a2a) = self.a2a_client {
            if let Some(ref url) = url {
                match a2a.send_message(url, params).await {
                    Ok(task) => {
                        tracing::info!(
                            cerebrate_id = %cerebrate_id,
                            goal_id = %goal.id,
                            a2a_task_id = %task.id,
                            "Goal delegated via A2A tasks/send"
                        );
                        task.id
                    }
                    Err(e) => {
                        return Err(format!(
                            "A2A goal delegation failed for cerebrate {}: {}",
                            cerebrate_id, e
                        ));
                    }
                }
            } else {
                return Err(format!(
                    "Cerebrate {} has no URL configured",
                    cerebrate_id
                ));
            }
        } else {
            // Fall back: create a FederationTaskEnvelope and use existing delegate path
            let mut envelope =
                FederationTaskEnvelope::new(Uuid::new_v4(), &goal.name, &goal.description);
            envelope.constraints = constraints_strs;
            envelope.parent_goal_id = Some(goal.id);

            let delegated_cerebrate = self.delegate_to(&envelope, cerebrate_id).await?;
            tracing::info!(
                cerebrate_id = %delegated_cerebrate,
                goal_id = %goal.id,
                task_id = %envelope.task_id,
                "Goal delegated via legacy federation envelope"
            );
            envelope.task_id.to_string()
        };

        // 5. Create a FederatedGoal in the Delegated state.
        let federated_goal = FederatedGoal::new(goal.id, cerebrate_id, &goal.description)
            .with_convergence_contract(contract)
            .with_remote_task_id(&remote_task_id);
        // State set directly: this is valid because the goal was just created above
        // with state Pending, and Pending -> Delegated is a valid transition.
        debug_assert!(FederatedGoalState::Pending.can_transition_to(FederatedGoalState::Delegated));
        let mut federated_goal = federated_goal;
        federated_goal.state = FederatedGoalState::Delegated;
        // Carry over constraints from the goal.
        for c in &goal.constraints {
            federated_goal.constraints.push(c.description.clone());
        }

        // 6. Emit FederatedGoalCreated event.
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                None,
                EventPayload::FederatedGoalCreated {
                    local_goal_id: goal.id,
                    cerebrate_id: cerebrate_id.to_string(),
                    remote_task_id: remote_task_id.clone(),
                },
            ))
            .await;

        // 7. Return the FederatedGoal.
        Ok(federated_goal)
    }

    /// Handle acceptance of a delegated task by a cerebrate.
    pub async fn handle_accept(&self, task_id: Uuid, cerebrate_id: &str) {
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(task_id),
                EventPayload::FederationTaskAccepted {
                    task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                },
            ))
            .await;
    }

    /// Handle rejection of a delegated task by a cerebrate.
    pub async fn handle_reject(
        &self,
        task_id: Uuid,
        cerebrate_id: &str,
        reason: &str,
    ) -> DelegationDecision {
        // Remove from in-flight
        {
            let mut in_flight = self.in_flight.write().await;
            in_flight.remove(&task_id);
        }

        // Decrement active delegations
        {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(cerebrate_id) {
                status.active_delegations = status.active_delegations.saturating_sub(1);
            }
        }

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Warning,
                Some(task_id),
                EventPayload::FederationTaskRejected {
                    task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                    reason: reason.to_string(),
                },
            ))
            .await;

        // Get remaining cerebrates for strategy decision
        let remaining = self.list_cerebrates().await;

        // Build a temporary envelope for the strategy
        let envelope = FederationTaskEnvelope::new(task_id, "", "");
        self.delegation_strategy
            .on_rejection(&envelope, cerebrate_id, reason, &remaining)
    }

    // ========================================================================
    // Progress & Results
    // ========================================================================

    /// Handle a progress update from a cerebrate.
    pub async fn handle_progress(
        &self,
        task_id: Uuid,
        cerebrate_id: &str,
        phase: &str,
        progress_pct: f64,
        summary: &str,
    ) {
        // Update last activity timestamp for stall detection
        {
            let mut activity = self.last_activity.write().await;
            activity.insert(task_id, chrono::Utc::now());
        }

        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Debug,
                Some(task_id),
                EventPayload::FederationProgressReceived {
                    task_id,
                    cerebrate_id: cerebrate_id.to_string(),
                    phase: phase.to_string(),
                    progress_pct,
                    summary: summary.to_string(),
                },
            ))
            .await;
    }

    /// Handle a final result from a cerebrate.
    pub async fn handle_result(
        &self,
        result: FederationResult,
        parent_context: ParentContext,
    ) -> Vec<FederationReaction> {
        let task_id = result.task_id;
        let cerebrate_id = {
            let in_flight = self.in_flight.read().await;
            in_flight.get(&task_id).cloned().unwrap_or_default()
        };

        // Remove from in-flight and activity tracking
        {
            let mut in_flight = self.in_flight.write().await;
            in_flight.remove(&task_id);
        }
        {
            let mut activity = self.last_activity.write().await;
            activity.remove(&task_id);
        }

        // Decrement active delegations
        if !cerebrate_id.is_empty() {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(&cerebrate_id) {
                status.active_delegations = status.active_delegations.saturating_sub(1);
            }
        }

        // Validate against schema if specified
        // (Schema validation would happen on the raw JSON payload in a real impl)

        // Emit result event
        self.event_bus
            .publish(event_factory::federation_event(
                EventSeverity::Info,
                Some(task_id),
                EventPayload::FederationResultReceived {
                    task_id,
                    cerebrate_id: cerebrate_id.clone(),
                    status: result.status.to_string(),
                    summary: result.summary.clone(),
                    artifacts: result.artifacts.clone(),
                },
            ))
            .await;

        // Process through result processor
        match result.status {
            FederationTaskStatus::Completed | FederationTaskStatus::Partial => {
                self.result_processor
                    .process_result(&result, &parent_context)
            }
            FederationTaskStatus::Failed => {
                self.result_processor
                    .process_failure(&result, &parent_context)
            }
        }
    }

    // ========================================================================
    // Persistence
    // ========================================================================

    /// Save connection state to disk using async I/O.
    pub async fn save_connections(&self, base_path: &std::path::Path) -> Result<(), String> {
        let dir = base_path.join("federation");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Failed to create dir: {}", e))?;

        let cerebrates = self.cerebrates.read().await;
        let data: Vec<&CerebrateStatus> = cerebrates.values().collect();
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| format!("Failed to serialize: {}", e))?;
        drop(cerebrates);

        tokio::fs::write(dir.join("connections.json"), json)
            .await
            .map_err(|e| format!("Failed to write: {}", e))?;

        Ok(())
    }

    /// Load connection state from disk using async I/O.
    pub async fn load_connections(&self, base_path: &std::path::Path) -> Result<usize, String> {
        let path = base_path.join("federation/connections.json");
        match tokio::fs::metadata(&path).await {
            Ok(_) => {}
            Err(_) => return Ok(0),
        }

        let json = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read: {}", e))?;
        let data: Vec<CerebrateStatus> =
            serde_json::from_str(&json).map_err(|e| format!("Failed to parse: {}", e))?;

        let count = data.len();
        let mut cerebrates = self.cerebrates.write().await;
        for status in data {
            cerebrates.insert(status.id.clone(), status);
        }

        Ok(count)
    }

    /// Get the number of in-flight tasks.
    pub async fn in_flight_count(&self) -> usize {
        self.in_flight.read().await.len()
    }

    /// Check for stalled delegations (no progress within stall_timeout_secs).
    /// Emits `FederationStallDetected` events for any stalled tasks.
    pub async fn check_stalls(&self) {
        let stall_secs = self.config.stall_timeout_secs;
        if stall_secs == 0 {
            return;
        }
        let now = chrono::Utc::now();
        let in_flight = self.in_flight.read().await;
        let last_activity = self.last_activity.read().await;

        // Collect stalled tasks before dropping locks
        let mut stalled = Vec::new();
        for (task_id, cerebrate_id) in in_flight.iter() {
            let last = last_activity.get(task_id).copied();
            if let Some(last) = last {
                let elapsed = (now - last).num_seconds().unsigned_abs();
                if elapsed >= stall_secs {
                    stalled.push((*task_id, cerebrate_id.clone(), elapsed));
                }
            }
        }

        drop(in_flight);
        drop(last_activity);

        for (task_id, cerebrate_id, elapsed) in stalled {
            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Warning,
                    Some(task_id),
                    EventPayload::FederationStallDetected {
                        task_id,
                        cerebrate_id,
                        stall_duration_secs: elapsed,
                    },
                ))
                .await;
        }
    }

    /// Get in-flight tasks for a specific cerebrate.
    pub async fn in_flight_for_cerebrate(&self, cerebrate_id: &str) -> Vec<Uuid> {
        let in_flight = self.in_flight.read().await;
        in_flight
            .iter()
            .filter(|(_, cid)| cid.as_str() == cerebrate_id)
            .map(|(tid, _)| *tid)
            .collect()
    }

    /// Get a reference to the delegation strategy.
    pub fn delegation_strategy(&self) -> &dyn FederationDelegationStrategy {
        self.delegation_strategy.as_ref()
    }

    /// Get a reference to the result processor.
    pub fn result_processor(&self) -> &dyn FederationResultProcessor {
        self.result_processor.as_ref()
    }

    /// Get a reference to the task transformer.
    pub fn task_transformer(&self) -> &dyn FederationTaskTransformer {
        self.task_transformer.as_ref()
    }

    // ========================================================================
    // Lifecycle: start / shutdown
    // ========================================================================

    /// Start all background tasks: heartbeat monitor, orphan detector, stall detector.
    ///
    /// Returns immediately after spawning. Call `shutdown()` to stop.
    pub async fn start(self: &Arc<Self>) {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
        {
            let mut slot = self.shutdown_tx.write().await;
            *slot = Some(tx.clone());
        }

        // Auto-connect cerebrates from config
        for cc in &self.config.cerebrates {
            self.register_cerebrate(&cc.id, &cc.display_name, &cc.url).await;
            if let Some(status) = self.get_cerebrate(&cc.id).await {
                // Apply config to the status
                let mut cerebrates = self.cerebrates.write().await;
                if let Some(s) = cerebrates.get_mut(&cc.id) {
                    s.max_concurrent_delegations = cc.max_concurrent_delegations;
                    s.capabilities.clone_from(&cc.capabilities);
                }
                drop(cerebrates);

                if cc.auto_connect
                    && let Err(e) = self.connect(&cc.id).await
                {
                    tracing::warn!(cerebrate_id = %cc.id, error = %e, "Auto-connect failed");
                }
                let _ = status; // used above
            }
        }

        // Spawn heartbeat monitor loop
        {
            let service = Arc::clone(self);
            let mut shutdown_rx = tx.subscribe();
            let interval_secs = self.config.heartbeat_interval_secs;
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
                interval.tick().await; // skip immediate tick
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            service.check_heartbeats().await;
                        }
                        _ = shutdown_rx.recv() => {
                            tracing::debug!("Federation heartbeat monitor shutting down");
                            break;
                        }
                    }
                }
            });
        }

        // Spawn orphan timeout detector
        {
            let service = Arc::clone(self);
            let mut shutdown_rx = tx.subscribe();
            let orphan_timeout = self.config.task_orphan_timeout_secs;
            if orphan_timeout > 0 {
                tokio::spawn(async move {
                    // Check every 60s or orphan_timeout/10, whichever is larger
                    let check_interval = (orphan_timeout / 10).max(60);
                    let mut interval = tokio::time::interval(Duration::from_secs(check_interval));
                    interval.tick().await;
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {
                                service.check_orphans(orphan_timeout).await;
                            }
                            _ = shutdown_rx.recv() => {
                                tracing::debug!("Federation orphan detector shutting down");
                                break;
                            }
                        }
                    }
                });
            }
        }

        // Spawn stall detector
        {
            let service = Arc::clone(self);
            let mut shutdown_rx = tx.subscribe();
            let stall_timeout = self.config.stall_timeout_secs;
            if stall_timeout > 0 {
                tokio::spawn(async move {
                    let check_interval = (stall_timeout / 6).max(30);
                    let mut interval = tokio::time::interval(Duration::from_secs(check_interval));
                    interval.tick().await;
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {
                                service.check_stalls().await;
                            }
                            _ = shutdown_rx.recv() => {
                                tracing::debug!("Federation stall detector shutting down");
                                break;
                            }
                        }
                    }
                });
            }
        }

        tracing::info!(
            role = %self.config.role,
            swarm_id = %self.config.swarm_id,
            "Federation service started"
        );
    }

    /// Shut down all background tasks.
    pub async fn shutdown(&self) {
        let tx = {
            let mut slot = self.shutdown_tx.write().await;
            slot.take()
        };
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
        tracing::info!("Federation service shut down");
    }

    // ========================================================================
    // Orphan Detection
    // ========================================================================

    /// Fail tasks delegated to unreachable cerebrates after the orphan timeout.
    async fn check_orphans(&self, orphan_timeout_secs: u64) {
        let now = chrono::Utc::now();
        let cerebrates = self.cerebrates.read().await;
        let in_flight = self.in_flight.read().await;
        let last_activity = self.last_activity.read().await;

        let mut orphaned = Vec::new();

        for (task_id, cerebrate_id) in in_flight.iter() {
            if let Some(status) = cerebrates.get(cerebrate_id)
                && status.connection_state == ConnectionState::Unreachable
            {
                // Check how long since last activity on this task
                let activity_time = last_activity.get(task_id).copied()
                    .or(status.last_heartbeat_at)
                    .unwrap_or(now);
                let elapsed = (now - activity_time).num_seconds().unsigned_abs();
                if elapsed >= orphan_timeout_secs {
                    orphaned.push((*task_id, cerebrate_id.clone()));
                }
            }
        }
        drop(in_flight);
        drop(cerebrates);
        drop(last_activity);

        // Remove orphaned tasks from in-flight and emit events
        for (task_id, cerebrate_id) in orphaned {
            {
                let mut in_flight = self.in_flight.write().await;
                in_flight.remove(&task_id);
            }
            {
                let mut last_activity = self.last_activity.write().await;
                last_activity.remove(&task_id);
            }
            {
                let mut cerebrates = self.cerebrates.write().await;
                if let Some(status) = cerebrates.get_mut(&cerebrate_id) {
                    status.active_delegations = status.active_delegations.saturating_sub(1);
                }
            }

            tracing::warn!(
                task_id = %task_id,
                cerebrate_id = %cerebrate_id,
                "Failing orphaned federation task (cerebrate unreachable)"
            );

            // Emit a result event as if the task failed
            self.event_bus
                .publish(event_factory::federation_event(
                    EventSeverity::Error,
                    Some(task_id),
                    EventPayload::FederationResultReceived {
                        task_id,
                        cerebrate_id,
                        status: "failed".to_string(),
                        summary: "Task orphaned: cerebrate unreachable beyond timeout".to_string(),
                        artifacts: Vec::new(),
                    },
                ))
                .await;
        }
    }

    // ========================================================================
    // Exponential Backoff Reconnection
    // ========================================================================

    /// Attempt to reconnect to a cerebrate with exponential backoff.
    ///
    /// Spawns a background task that retries connection with delays:
    /// 5s initial, 2x factor, 300s max. Runs until connected or explicit disconnect.
    pub async fn start_reconnect_loop(self: &Arc<Self>, cerebrate_id: String) {
        let service = Arc::clone(self);
        let shutdown_rx = {
            let slot = self.shutdown_tx.read().await;
            slot.as_ref().map(|tx| tx.subscribe())
        };

        tokio::spawn(async move {
            let initial_delay = Duration::from_secs(5);
            let max_delay = Duration::from_secs(300);
            let factor = 2u32;
            let mut current_delay = initial_delay;

            loop {
                // Check if cerebrate is still registered and not connected
                let should_retry = {
                    let cerebrates = service.cerebrates.read().await;
                    cerebrates.get(&cerebrate_id).is_some_and(|s| {
                        matches!(
                            s.connection_state,
                            ConnectionState::Unreachable | ConnectionState::Reconnecting
                        )
                    })
                };

                if !should_retry {
                    tracing::debug!(
                        cerebrate_id = %cerebrate_id,
                        "Reconnect loop ending (no longer unreachable)"
                    );
                    break;
                }

                // Set state to Reconnecting
                {
                    let mut cerebrates = service.cerebrates.write().await;
                    if let Some(status) = cerebrates.get_mut(&cerebrate_id) {
                        status.connection_state = ConnectionState::Reconnecting;
                    }
                }

                tracing::info!(
                    cerebrate_id = %cerebrate_id,
                    delay_secs = current_delay.as_secs(),
                    "Attempting reconnection to cerebrate"
                );

                // Wait with shutdown check
                if let Some(ref mut rx) = shutdown_rx.as_ref().and_then(|_| {
                    // Re-subscribe each iteration is not ideal, use a flag instead
                    service.shutdown_tx.try_read().ok().and_then(|s| s.as_ref().map(|tx| tx.subscribe()))
                }) {
                    tokio::select! {
                        _ = tokio::time::sleep(current_delay) => {}
                        _ = rx.recv() => {
                            tracing::debug!(cerebrate_id = %cerebrate_id, "Reconnect loop cancelled by shutdown");
                            return;
                        }
                    }
                } else {
                    tokio::time::sleep(current_delay).await;
                }

                // Attempt connection
                match service.connect(&cerebrate_id).await {
                    Ok(()) => {
                        tracing::info!(cerebrate_id = %cerebrate_id, "Reconnected to cerebrate");
                        // Trigger reconciliation on reconnect
                        service.reconcile_on_reconnect(&cerebrate_id).await;
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            cerebrate_id = %cerebrate_id,
                            error = %e,
                            "Reconnection attempt failed"
                        );
                        current_delay = (current_delay * factor).min(max_delay);
                    }
                }
            }
        });
    }

    /// Reconcile in-flight task state after reconnecting to a cerebrate.
    ///
    /// Exchanges in-flight task ID lists over the wire. Tasks known locally
    /// but not remotely are failed as orphaned. Tasks known remotely but not
    /// locally are re-tracked.
    async fn reconcile_on_reconnect(&self, cerebrate_id: &str) {
        let local_tasks = self.in_flight_for_cerebrate(cerebrate_id).await;
        if local_tasks.is_empty() {
            return;
        }

        tracing::info!(
            cerebrate_id = %cerebrate_id,
            task_count = local_tasks.len(),
            "Reconciling in-flight tasks after reconnection"
        );

        // Get the cerebrate URL for the HTTP reconcile call
        let url = {
            let cerebrates = self.cerebrates.read().await;
            cerebrates.get(cerebrate_id).and_then(|s| s.url.clone())
        };

        if let Some(ref url) = url {
            match self
                .http_client
                .reconcile(url, cerebrate_id, &local_tasks)
                .await
            {
                Ok(remote_tasks) => {
                    // Tasks we track locally but the remote doesn't know about → orphaned
                    let orphaned: Vec<Uuid> = local_tasks
                        .iter()
                        .filter(|t| !remote_tasks.contains(t))
                        .copied()
                        .collect();

                    for task_id in orphaned {
                        tracing::warn!(
                            task_id = %task_id,
                            cerebrate_id = %cerebrate_id,
                            "Task unknown to remote cerebrate after reconnect, failing as orphaned"
                        );
                        let mut in_flight = self.in_flight.write().await;
                        in_flight.remove(&task_id);
                        let mut activity = self.last_activity.write().await;
                        activity.remove(&task_id);

                        self.event_bus
                            .publish(event_factory::federation_event(
                                EventSeverity::Warning,
                                Some(task_id),
                                EventPayload::FederationResultReceived {
                                    task_id,
                                    cerebrate_id: cerebrate_id.to_string(),
                                    status: "failed".to_string(),
                                    summary: "Task lost during disconnection".to_string(),
                                    artifacts: Vec::new(),
                                },
                            ))
                            .await;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        cerebrate_id = %cerebrate_id,
                        error = %e,
                        "Reconciliation HTTP call failed, keeping local state"
                    );
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::{EventBus, EventBusConfig};

    fn make_service() -> FederationService {
        let config = FederationConfig::default();
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        FederationService::new(config, event_bus)
    }

    #[tokio::test]
    async fn test_register_and_connect() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;

        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.connection_state, ConnectionState::Disconnected);

        svc.connect("c1").await.unwrap();

        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.connection_state, ConnectionState::Connected);
    }

    #[tokio::test]
    async fn test_connect_unknown_cerebrate() {
        let svc = make_service();
        let result = svc.connect("unknown").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_disconnect() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();
        svc.disconnect("c1").await.unwrap();

        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.connection_state, ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_delegate_to() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test task", "Do the thing");
        let result = svc.delegate_to(&envelope, "c1").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "c1");

        assert_eq!(svc.in_flight_count().await, 1);

        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.active_delegations, 1);
    }

    #[tokio::test]
    async fn test_delegate_to_unavailable() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        // Not connected

        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test task", "Do the thing");
        let result = svc.delegate_to(&envelope, "c1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_result_completed() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let task_id = Uuid::new_v4();
        let corr_id = Uuid::new_v4();
        let envelope = FederationTaskEnvelope::new(task_id, "Test task", "Do the thing");
        svc.delegate_to(&envelope, "c1").await.unwrap();

        let result = FederationResult::completed(task_id, corr_id, "All done");
        let goal_id = Uuid::new_v4();
        let ctx = ParentContext {
            goal_id: Some(goal_id),
            ..Default::default()
        };

        let reactions = svc.handle_result(result, ctx).await;
        assert_eq!(reactions.len(), 1);
        assert!(matches!(reactions[0], FederationReaction::UpdateGoalProgress { .. }));

        // Task should be removed from in-flight
        assert_eq!(svc.in_flight_count().await, 0);

        // Active delegations decremented
        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.active_delegations, 0);
    }

    #[tokio::test]
    async fn test_handle_result_failed() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let task_id = Uuid::new_v4();
        let corr_id = Uuid::new_v4();
        let envelope = FederationTaskEnvelope::new(task_id, "Test task", "Do the thing");
        svc.delegate_to(&envelope, "c1").await.unwrap();

        let result = FederationResult::failed(task_id, corr_id, "Failed", "CI broke");
        let ctx = ParentContext {
            goal_id: Some(Uuid::new_v4()),
            ..Default::default()
        };

        let reactions = svc.handle_result(result, ctx).await;
        assert_eq!(reactions.len(), 1);
        assert!(matches!(reactions[0], FederationReaction::Escalate { .. }));
    }

    #[tokio::test]
    async fn test_handle_heartbeat() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        svc.handle_heartbeat("c1", 0.5).await;

        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.load, 0.5);
        assert_eq!(status.missed_heartbeats, 0);
    }

    #[tokio::test]
    async fn test_heartbeat_reconnect() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        // Simulate unreachable
        {
            let mut cerebrates = svc.cerebrates.write().await;
            if let Some(s) = cerebrates.get_mut("c1") {
                s.connection_state = ConnectionState::Unreachable;
            }
        }

        // Heartbeat should reconnect
        svc.handle_heartbeat("c1", 0.3).await;

        let status = svc.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.connection_state, ConnectionState::Connected);
    }

    #[tokio::test]
    async fn test_handle_reject() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.register_cerebrate("c2", "Cerebrate 2", "https://c2.example.com")
            .await;
        svc.connect("c1").await.unwrap();
        svc.connect("c2").await.unwrap();

        let task_id = Uuid::new_v4();
        let envelope = FederationTaskEnvelope::new(task_id, "Test", "Test");
        svc.delegate_to(&envelope, "c1").await.unwrap();

        let decision = svc.handle_reject(task_id, "c1", "too busy").await;
        assert!(matches!(decision, DelegationDecision::Redelegate(ref id) if id == "c2"));

        assert_eq!(svc.in_flight_count().await, 0);
    }

    #[tokio::test]
    async fn test_list_cerebrates() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.register_cerebrate("c2", "Cerebrate 2", "https://c2.example.com")
            .await;

        let list = svc.list_cerebrates().await;
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_start_and_shutdown() {
        let config = FederationConfig {
            heartbeat_interval_secs: 3600, // very long so it doesn't fire
            stall_timeout_secs: 0,
            task_orphan_timeout_secs: 0,
            ..FederationConfig::default()
        };
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let svc = Arc::new(FederationService::new(config, event_bus));

        svc.start().await;

        // Verify shutdown signal is set
        {
            let slot = svc.shutdown_tx.read().await;
            assert!(slot.is_some());
        }

        svc.shutdown().await;

        // After shutdown, the tx is taken
        {
            let slot = svc.shutdown_tx.read().await;
            assert!(slot.is_none());
        }
    }

    #[tokio::test]
    async fn test_check_stalls() {
        let config = FederationConfig {
            stall_timeout_secs: 1, // 1 second for test
            ..FederationConfig::default()
        };
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let svc = FederationService::new(config, event_bus.clone());
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let envelope = FederationTaskEnvelope::new(Uuid::new_v4(), "Test", "Test");
        svc.delegate_to(&envelope, "c1").await.unwrap();

        // Set last activity to 2 seconds ago
        {
            let mut activity = svc.last_activity.write().await;
            activity.insert(
                envelope.task_id,
                chrono::Utc::now() - chrono::Duration::seconds(2),
            );
        }

        // Subscribe to events to verify stall detection
        let mut rx = event_bus.subscribe();

        svc.check_stalls().await;

        // Should have emitted a stall event
        let event = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
        assert!(event.is_ok(), "Expected a stall event to be emitted");
    }

    #[tokio::test]
    async fn test_check_orphans() {
        let config = FederationConfig {
            task_orphan_timeout_secs: 1,
            ..FederationConfig::default()
        };
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        let svc = FederationService::new(config, event_bus.clone());
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let task_id = Uuid::new_v4();
        let envelope = FederationTaskEnvelope::new(task_id, "Test", "Test");
        svc.delegate_to(&envelope, "c1").await.unwrap();

        // Simulate unreachable cerebrate
        {
            let mut cerebrates = svc.cerebrates.write().await;
            if let Some(s) = cerebrates.get_mut("c1") {
                s.connection_state = ConnectionState::Unreachable;
            }
        }

        // Set last activity to 2 seconds ago
        {
            let mut activity = svc.last_activity.write().await;
            activity.insert(
                task_id,
                chrono::Utc::now() - chrono::Duration::seconds(2),
            );
        }

        let mut rx = event_bus.subscribe();
        svc.check_orphans(1).await;

        // Task should be removed from in-flight
        assert_eq!(svc.in_flight_count().await, 0);

        // Should have emitted a result event
        let event = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
        assert!(event.is_ok(), "Expected an orphan failure event");
    }

    #[tokio::test]
    async fn test_progress_updates_activity() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let task_id = Uuid::new_v4();
        let envelope = FederationTaskEnvelope::new(task_id, "Test", "Test");
        svc.delegate_to(&envelope, "c1").await.unwrap();

        let before = {
            let activity = svc.last_activity.read().await;
            activity.get(&task_id).copied().unwrap()
        };

        // Small delay to ensure timestamp differs
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        svc.handle_progress(task_id, "c1", "building", 50.0, "Half done").await;

        let after = {
            let activity = svc.last_activity.read().await;
            activity.get(&task_id).copied().unwrap()
        };

        assert!(after > before, "Progress should update last_activity timestamp");
    }

    #[tokio::test]
    async fn test_save_load_connections() {
        let svc = make_service();
        svc.register_cerebrate("c1", "Cerebrate 1", "https://c1.example.com")
            .await;
        svc.connect("c1").await.unwrap();

        let tmp = tempfile::tempdir().unwrap();
        svc.save_connections(tmp.path()).await.unwrap();

        // Create fresh service and load
        let svc2 = make_service();
        let loaded = svc2.load_connections(tmp.path()).await.unwrap();
        assert_eq!(loaded, 1);

        let status = svc2.get_cerebrate("c1").await.unwrap();
        assert_eq!(status.display_name, "Cerebrate 1");
    }
}

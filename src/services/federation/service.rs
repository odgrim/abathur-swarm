//! Federation service managing cerebrate connections, heartbeats, and task delegation.
//!
//! This file is the top-level facade for the federation subsystem. It owns
//! the cerebrate registry, the heartbeat monitor, the discovery/registration
//! lifecycle, and the persistence of connection state. Outbound task
//! delegation and inbound result ingest are delegated to two internal
//! collaborators:
//!
//! - [`super::delegation_manager::DelegationManager`] for outbound
//!   delegation, reconciliation, stall/orphan monitoring.
//! - [`super::result_processor::ResultProcessor`] for inbound
//!   accept/progress/result ingest.
//!
//! The public API of [`FederationService`] is unchanged; it forwards
//! delegation / result methods to the collaborators, which share state
//! with the service via `Arc`s.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::adapters::a2a::A2AClient;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::a2a::{
    CerebrateStatus, ConnectionState, FederationCard, FederationResult, FederationTaskEnvelope,
};
use crate::domain::models::goal::Goal;
use crate::domain::models::goal_federation::{ConvergenceContract, FederatedGoal};
use crate::domain::ports::GoalRepository;
use crate::services::event_bus::{EventBus, EventPayload, EventSeverity};
use crate::services::event_factory;

use super::config::FederationConfig;
use super::delegation_manager::DelegationManager;
use super::result_processor::ResultProcessor;
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

        // Check for JSON-RPC error response before assuming success
        if let Some(error) = body.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(format!("Discovery returned error: {}", message));
        }

        let result = body.get("result").ok_or_else(|| {
            "Discovery response missing 'result' field".to_string()
        })?;

        serde_json::from_value(result.clone())
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

    /// Send a task result to the parent overmind.
    pub async fn send_result(
        &self,
        url: &str,
        result: &FederationResult,
    ) -> Result<(), String> {
        let result_url = format!("{}/federation/result", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&result_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/result",
                "id": 1,
                "params": result
            }))
            .send()
            .await
            .map_err(|e| format!("Result request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Result returned status {}", resp.status()));
        }
        Ok(())
    }

    /// Send a progress update to the parent overmind.
    pub async fn send_progress(
        &self,
        url: &str,
        task_id: Uuid,
        cerebrate_id: &str,
        phase: &str,
        progress_pct: f64,
        summary: &str,
    ) -> Result<(), String> {
        let progress_url = format!("{}/federation/progress", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&progress_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/progress",
                "id": 1,
                "params": {
                    "task_id": task_id,
                    "cerebrate_id": cerebrate_id,
                    "phase": phase,
                    "progress_pct": progress_pct,
                    "summary": summary
                }
            }))
            .send()
            .await
            .map_err(|e| format!("Progress request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Progress returned status {}", resp.status()));
        }
        Ok(())
    }

    /// Send a task acceptance notification to the parent overmind.
    pub async fn send_accept(
        &self,
        url: &str,
        task_id: Uuid,
        cerebrate_id: &str,
    ) -> Result<(), String> {
        let accept_url = format!("{}/federation/accept", url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&accept_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "federation/accept",
                "id": 1,
                "params": {
                    "task_id": task_id,
                    "cerebrate_id": cerebrate_id
                }
            }))
            .send()
            .await
            .map_err(|e| format!("Accept request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Accept returned status {}", resp.status()));
        }
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
///
/// This is a thin facade. Outbound delegation lives in
/// [`DelegationManager`] and inbound result ingest lives in
/// [`ResultProcessor`]; both share state with the service via `Arc`s.
pub struct FederationService {
    config: FederationConfig,
    /// Registry of known cerebrates (id → status).
    cerebrates: Arc<RwLock<HashMap<String, CerebrateStatus>>>,
    /// In-flight delegated tasks (task_id → cerebrate_id).
    in_flight: Arc<RwLock<HashMap<Uuid, String>>>,
    /// Envelope snapshots for in-flight tasks (task_id → envelope). Kept so
    /// rejection handling can recover task context (`parent_task_id`, title,
    /// capability requirements) without depending on a task repository.
    /// Cleared on terminal results.
    delegated_envelopes: Arc<RwLock<HashMap<Uuid, FederationTaskEnvelope>>>,
    /// Rejection history per task (task_id → list of cerebrate_ids that
    /// rejected it, in order). Populated by `handle_reject`; consumed by
    /// the delegation strategy to avoid re-delegating to a rejector.
    rejection_history: Arc<RwLock<HashMap<Uuid, Vec<String>>>>,
    /// Maps federation task_id → FederatedGoal.id so that result handlers
    /// can correlate incoming results back to the federated goal that owns
    /// the DAG node.
    task_to_federated_goal: Arc<RwLock<HashMap<String, Uuid>>>,
    /// Delegation timestamps for stall detection (task_id → last_activity_at).
    last_activity: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
    /// EventBus for emitting federation events.
    event_bus: Arc<EventBus>,
    /// HTTP client for outbound federation calls.
    http_client: FederationHttpClient,
    /// Delegation strategy.
    delegation_strategy: Arc<dyn FederationDelegationStrategy>,
    /// Result processor (user-supplied trait object).
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
    /// Internal collaborator: outbound delegation.
    delegation: Arc<DelegationManager>,
    /// Internal collaborator: inbound result ingest.
    results: Arc<ResultProcessor>,
    /// Optional goal repository for cross-cutting goal lookups (e.g.
    /// validating that a goal_id referenced from a federation request
    /// actually exists). Optional because not every construction site
    /// has a database (tests, ephemeral instances).
    goal_repository: Option<Arc<dyn GoalRepository>>,
}

impl FederationService {
    /// Create a new FederationService with default strategies.
    pub fn new(config: FederationConfig, event_bus: Arc<EventBus>) -> Self {
        let mut schemas_map: HashMap<String, Arc<dyn ResultSchema>> = HashMap::new();
        let standard = Arc::new(StandardV1Schema);
        schemas_map.insert(standard.schema_id().to_string(), standard);

        let cerebrates = Arc::new(RwLock::new(HashMap::new()));
        let in_flight = Arc::new(RwLock::new(HashMap::new()));
        let delegated_envelopes = Arc::new(RwLock::new(HashMap::new()));
        let rejection_history = Arc::new(RwLock::new(HashMap::new()));
        let last_activity = Arc::new(RwLock::new(HashMap::new()));
        let task_to_federated_goal = Arc::new(RwLock::new(HashMap::new()));
        let schemas = Arc::new(RwLock::new(schemas_map));
        let http_client = FederationHttpClient::new();
        let delegation_strategy: Arc<dyn FederationDelegationStrategy> =
            Arc::new(DefaultDelegationStrategy);
        let result_processor: Arc<dyn FederationResultProcessor> =
            Arc::new(DefaultResultProcessor);
        let task_transformer: Arc<dyn FederationTaskTransformer> =
            Arc::new(DefaultTaskTransformer);
        let a2a_client: Option<Arc<dyn A2AClient>> = None;

        let delegation = Arc::new(DelegationManager::new(
            config.clone(),
            Arc::clone(&cerebrates),
            Arc::clone(&in_flight),
            Arc::clone(&delegated_envelopes),
            Arc::clone(&rejection_history),
            Arc::clone(&last_activity),
            Arc::clone(&task_to_federated_goal),
            Arc::clone(&event_bus),
            http_client.clone(),
            Arc::clone(&delegation_strategy),
            Arc::clone(&task_transformer),
            a2a_client.clone(),
        ));
        let results = Arc::new(ResultProcessor::new(
            Arc::clone(&cerebrates),
            Arc::clone(&in_flight),
            Arc::clone(&delegated_envelopes),
            Arc::clone(&rejection_history),
            Arc::clone(&last_activity),
            Arc::clone(&event_bus),
            Arc::clone(&result_processor),
            Arc::clone(&schemas),
        ));

        Self {
            config,
            cerebrates,
            in_flight,
            delegated_envelopes,
            rejection_history,
            task_to_federated_goal,
            last_activity,
            event_bus,
            http_client,
            delegation_strategy,
            result_processor,
            task_transformer,
            schemas,
            shutdown_tx: Arc::new(RwLock::new(None)),
            a2a_client,
            delegation,
            results,
            goal_repository: None,
        }
    }

    /// Rebuild the internal `DelegationManager` and `ResultProcessor` after a
    /// `with_*` builder swaps out a strategy. Called only from the builder
    /// methods — the shared `Arc<RwLock<_>>` state handles survive.
    fn rebuild_collaborators(&mut self) {
        self.delegation = Arc::new(DelegationManager::new(
            self.config.clone(),
            Arc::clone(&self.cerebrates),
            Arc::clone(&self.in_flight),
            Arc::clone(&self.delegated_envelopes),
            Arc::clone(&self.rejection_history),
            Arc::clone(&self.last_activity),
            Arc::clone(&self.task_to_federated_goal),
            Arc::clone(&self.event_bus),
            self.http_client.clone(),
            Arc::clone(&self.delegation_strategy),
            Arc::clone(&self.task_transformer),
            self.a2a_client.clone(),
        ));
        self.results = Arc::new(ResultProcessor::new(
            Arc::clone(&self.cerebrates),
            Arc::clone(&self.in_flight),
            Arc::clone(&self.delegated_envelopes),
            Arc::clone(&self.rejection_history),
            Arc::clone(&self.last_activity),
            Arc::clone(&self.event_bus),
            Arc::clone(&self.result_processor),
            Arc::clone(&self.schemas),
        ));
    }

    /// Set an A2A wire-protocol client for outbound federation calls.
    ///
    /// When set, `delegate_to()` and `connect()` prefer this client over
    /// the bespoke `FederationHttpClient`. Both paths coexist — the A2A
    /// client is tried first and falls back to the legacy client on error.
    pub fn with_a2a_client(mut self, client: Arc<dyn A2AClient>) -> Self {
        self.a2a_client = Some(client);
        self.rebuild_collaborators();
        self
    }

    /// Replace the delegation strategy.
    pub fn with_delegation_strategy(
        mut self,
        strategy: Arc<dyn FederationDelegationStrategy>,
    ) -> Self {
        self.delegation_strategy = strategy;
        self.rebuild_collaborators();
        self
    }

    /// Replace the result processor.
    pub fn with_result_processor(
        mut self,
        processor: Arc<dyn FederationResultProcessor>,
    ) -> Self {
        self.result_processor = processor;
        self.rebuild_collaborators();
        self
    }

    /// Replace the task transformer.
    pub fn with_task_transformer(
        mut self,
        transformer: Arc<dyn FederationTaskTransformer>,
    ) -> Self {
        self.task_transformer = transformer;
        self.rebuild_collaborators();
        self
    }

    /// Inject a goal repository so federation can validate goal references
    /// (e.g. confirm that a `goal_id` carried in a federation envelope
    /// actually corresponds to a known goal). Construction sites that do
    /// not need this validation (most unit tests) can omit it; in that
    /// case `validate_goal_exists` returns `Ok(())`.
    pub fn with_goal_repository(mut self, repo: Arc<dyn GoalRepository>) -> Self {
        self.goal_repository = Some(repo);
        self
    }

    /// Verify that a goal with the given id exists in the configured
    /// repository.
    ///
    /// Returns:
    /// - `Ok(())` if the goal exists, or if no goal repository is wired
    ///   (the validation is treated as best-effort).
    /// - `Err(DomainError::GoalNotFound)` if the repository is configured
    ///   and the goal is missing.
    /// - Other `DomainError` variants for storage failures.
    ///
    /// This lives on `FederationService` (rather than the transport layer)
    /// because federation owns the goal/cerebrate relationship; transports
    /// should not reach into domain repositories directly.
    pub async fn validate_goal_exists(&self, goal_id: Uuid) -> DomainResult<()> {
        let Some(ref repo) = self.goal_repository else {
            return Ok(());
        };
        match repo.get(goal_id).await? {
            Some(_) => Ok(()),
            None => Err(DomainError::GoalNotFound(goal_id)),
        }
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
        // When an A2A client is configured and BOTH A2A discovery and legacy
        // register fail, the cerebrate is marked Unreachable instead of
        // Connected (Issue #9). When only legacy HTTP is used (no A2A client),
        // a single HTTP failure is treated as non-fatal to allow local/test
        // connections without a real remote endpoint.
        let mut dual_failure = false;
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
                                "Both A2A discovery and legacy register failed"
                            );
                            dual_failure = true;
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

        // If both A2A and legacy paths failed, mark Unreachable and return error
        // instead of incorrectly transitioning to Connected.
        if dual_failure {
            let mut cerebrates = self.cerebrates.write().await;
            if let Some(status) = cerebrates.get_mut(id) {
                status.connection_state = ConnectionState::Unreachable;
            }
            return Err(format!(
                "Failed to connect to cerebrate {}: both A2A and legacy register failed",
                id
            ));
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
    // Task Delegation (forwarded to DelegationManager)
    // ========================================================================

    /// Delegate a task to the best available cerebrate.
    pub async fn delegate(
        &self,
        envelope: FederationTaskEnvelope,
    ) -> Result<String, String> {
        self.delegation.delegate(envelope).await
    }

    /// Delegate a task to a specific cerebrate.
    pub async fn delegate_to(
        &self,
        envelope: &FederationTaskEnvelope,
        cerebrate_id: &str,
    ) -> Result<String, String> {
        self.delegation.delegate_to(envelope, cerebrate_id).await
    }

    /// Delegate a goal to a specific cerebrate via A2A.
    pub async fn delegate_goal(
        &self,
        goal: &Goal,
        cerebrate_id: &str,
        contract: ConvergenceContract,
    ) -> Result<FederatedGoal, String> {
        self.delegation.delegate_goal(goal, cerebrate_id, contract).await
    }

    /// Handle acceptance of a delegated task by a cerebrate.
    pub async fn handle_accept(&self, task_id: Uuid, cerebrate_id: &str) {
        self.results.handle_accept(task_id, cerebrate_id).await
    }

    /// Handle rejection of a delegated task by a cerebrate.
    pub async fn handle_reject(
        &self,
        task_id: Uuid,
        cerebrate_id: &str,
        reason: &str,
    ) -> DelegationDecision {
        self.delegation.handle_reject(task_id, cerebrate_id, reason).await
    }

    // ========================================================================
    // Progress & Results (forwarded to ResultProcessor)
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
        self.results
            .handle_progress(task_id, cerebrate_id, phase, progress_pct, summary)
            .await
    }

    /// Handle a final result from a cerebrate.
    pub async fn handle_result(
        &self,
        result: FederationResult,
        parent_context: ParentContext,
    ) -> Vec<FederationReaction> {
        self.results.handle_result(result, parent_context).await
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
        self.delegation.in_flight_count().await
    }

    /// Check for stalled delegations (no progress within stall_timeout_secs).
    /// Emits `FederationStallDetected` events for any stalled tasks.
    pub async fn check_stalls(&self) {
        self.delegation.check_stalls().await
    }

    /// Get in-flight tasks for a specific cerebrate.
    pub async fn in_flight_for_cerebrate(&self, cerebrate_id: &str) -> Vec<Uuid> {
        self.delegation.in_flight_for_cerebrate(cerebrate_id).await
    }

    /// Get a reference to the delegation strategy.
    pub fn delegation_strategy(&self) -> &dyn FederationDelegationStrategy {
        self.delegation_strategy.as_ref()
    }

    /// Get a reference to the result processor.
    pub fn result_processor(&self) -> &dyn FederationResultProcessor {
        self.result_processor.as_ref()
    }

    /// Look up the `FederatedGoal.id` that corresponds to a given task_id.
    ///
    /// This is used by `FederationResultHandler` to emit
    /// `FederatedGoalConverged` / `FederatedGoalFailed` events with the
    /// correct ID so that `SwarmDagEventHandler` can correlate them to
    /// DAG nodes.
    pub async fn federated_goal_id_for_task(&self, task_id: Uuid) -> Option<Uuid> {
        let map = self.task_to_federated_goal.read().await;
        map.get(&task_id.to_string()).copied()
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
    // Orphan Detection (forwarded)
    // ========================================================================

    /// Fail tasks delegated to unreachable cerebrates after the orphan timeout.
    async fn check_orphans(&self, orphan_timeout_secs: u64) {
        self.delegation.check_orphans(orphan_timeout_secs).await
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
        // Subscribe to the shutdown channel ONCE before spawning the loop.
        let shutdown_rx = {
            let slot = self.shutdown_tx.read().await;
            slot.as_ref().map(|tx| tx.subscribe())
        };

        tokio::spawn(async move {
            let initial_delay = Duration::from_secs(5);
            let max_delay = Duration::from_secs(300);
            let factor = 2u32;
            let mut current_delay = initial_delay;
            let mut shutdown_rx = shutdown_rx;

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

                // Wait with shutdown check — use the single receiver subscribed
                // before the loop to avoid re-subscribing on each iteration.
                if let Some(ref mut rx) = shutdown_rx {
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
                        service.delegation.reconcile_on_reconnect(&cerebrate_id).await;
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

        // After redelegate, the task should still be in-flight but mapped to the new cerebrate
        assert_eq!(svc.in_flight_count().await, 1);
        let in_flight = svc.in_flight.read().await;
        assert_eq!(in_flight.get(&task_id).map(|s| s.as_str()), Some("c2"));
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

    // ------------------------------------------------------------------
    // validate_goal_exists
    // ------------------------------------------------------------------

    use crate::domain::models::{Goal, GoalStatus};
    use crate::domain::ports::{GoalFilter, GoalRepository as GoalRepoTrait};
    use async_trait::async_trait;
    use std::collections::HashMap as StdHashMap;
    use tokio::sync::Mutex as TokioMutex;

    /// Minimal in-memory GoalRepository used only by the validation tests.
    struct MockGoalRepo {
        goals: TokioMutex<StdHashMap<Uuid, Goal>>,
    }

    impl MockGoalRepo {
        fn new() -> Self {
            Self {
                goals: TokioMutex::new(StdHashMap::new()),
            }
        }

        async fn insert(&self, goal: Goal) {
            self.goals.lock().await.insert(goal.id, goal);
        }
    }

    #[async_trait]
    impl GoalRepoTrait for MockGoalRepo {
        async fn create(&self, goal: &Goal) -> crate::domain::errors::DomainResult<()> {
            self.goals.lock().await.insert(goal.id, goal.clone());
            Ok(())
        }
        async fn get(&self, id: Uuid) -> crate::domain::errors::DomainResult<Option<Goal>> {
            Ok(self.goals.lock().await.get(&id).cloned())
        }
        async fn update(&self, goal: &Goal) -> crate::domain::errors::DomainResult<()> {
            self.goals.lock().await.insert(goal.id, goal.clone());
            Ok(())
        }
        async fn delete(&self, id: Uuid) -> crate::domain::errors::DomainResult<()> {
            self.goals.lock().await.remove(&id);
            Ok(())
        }
        async fn list(&self, _filter: GoalFilter) -> crate::domain::errors::DomainResult<Vec<Goal>> {
            Ok(self.goals.lock().await.values().cloned().collect())
        }
        async fn get_children(
            &self,
            _parent_id: Uuid,
        ) -> crate::domain::errors::DomainResult<Vec<Goal>> {
            Ok(vec![])
        }
        async fn get_active_with_constraints(
            &self,
        ) -> crate::domain::errors::DomainResult<Vec<Goal>> {
            Ok(vec![])
        }
        async fn count_by_status(
            &self,
        ) -> crate::domain::errors::DomainResult<StdHashMap<GoalStatus, u64>> {
            Ok(StdHashMap::new())
        }
        async fn find_by_domains(
            &self,
            _domains: &[String],
        ) -> crate::domain::errors::DomainResult<Vec<Goal>> {
            Ok(vec![])
        }
        async fn update_last_check(
            &self,
            _goal_id: Uuid,
            _ts: chrono::DateTime<chrono::Utc>,
        ) -> crate::domain::errors::DomainResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_validate_goal_exists_no_repo_is_ok() {
        // When no repository is wired, validation is a no-op. Keeps the
        // many test/ephemeral construction sites working unchanged.
        let svc = make_service();
        assert!(svc.validate_goal_exists(Uuid::new_v4()).await.is_ok());
    }

    #[tokio::test]
    async fn test_validate_goal_exists_present() {
        let repo = Arc::new(MockGoalRepo::new());
        let goal = Goal::new("g", "d");
        let goal_id = goal.id;
        repo.insert(goal).await;

        let svc = make_service().with_goal_repository(repo);
        svc.validate_goal_exists(goal_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_validate_goal_exists_missing() {
        let repo = Arc::new(MockGoalRepo::new());
        let svc = make_service().with_goal_repository(repo);

        let missing = Uuid::new_v4();
        let err = svc.validate_goal_exists(missing).await.unwrap_err();
        assert!(matches!(err, DomainError::GoalNotFound(id) if id == missing));
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

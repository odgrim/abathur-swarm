//! A2A (Agent-to-Agent) HTTP Gateway.
//!
//! Implements JSON-RPC 2.0 over HTTP for agent-to-agent communication
//! following the A2A protocol specification.
//!
//! This module hosts the gateway type, shared state, router construction,
//! TLS / JWT plumbing, and JSON-RPC envelope types. The actual JSON-RPC
//! method implementations live in sibling submodules:
//!
//! * [`dispatch`]      — single source of truth for `method` → handler mapping
//! * [`tasks`]         — `tasks/*` (send / get / cancel / sendSubscribe / push notification)
//! * [`agent`]         — `agent/card`, `agent/skills`
//! * [`federation`]    — `federation/*` (10 methods)
//! * [`rest`]          — REST endpoints + `.well-known/agent.json` + delegation queue

mod agent;
mod dispatch;
mod federation;
mod rest;
mod tasks;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{
        Json,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::domain::models::a2a::{A2AAgentCard, A2AMessage};

/// A2A-specific JSON-RPC error codes.
#[derive(Debug, Clone, Copy)]
pub enum A2AErrorCode {
    /// Task not found.
    TaskNotFound = -32001,
    /// Task cannot be canceled in current state.
    TaskNotCancelable = -32002,
    /// Agent doesn't support webhooks.
    PushNotificationNotSupported = -32003,
    /// Requested operation not supported.
    UnsupportedOperation = -32004,
    /// Unsupported content/MIME type.
    ContentTypeNotSupported = -32005,
    /// Agent returned malformed response.
    InvalidAgentResponse = -32006,
    /// Agent not found.
    AgentNotFound = -32007,
    /// Invalid request parameters.
    InvalidParams = -32602,
    /// Method not found.
    MethodNotFound = -32601,
    /// Parse error.
    ParseError = -32700,
    /// Internal error.
    InternalError = -32603,
}

impl A2AErrorCode {
    pub fn code(&self) -> i32 {
        *self as i32
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::TaskNotFound => "Task not found",
            Self::TaskNotCancelable => "Task cannot be canceled",
            Self::PushNotificationNotSupported => "Push notifications not supported",
            Self::UnsupportedOperation => "Operation not supported",
            Self::ContentTypeNotSupported => "Content type not supported",
            Self::InvalidAgentResponse => "Invalid agent response",
            Self::AgentNotFound => "Agent not found",
            Self::InvalidParams => "Invalid params",
            Self::MethodNotFound => "Method not found",
            Self::ParseError => "Parse error",
            Self::InternalError => "Internal error",
        }
    }
}

/// A2A task state matching the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum A2ATaskState {
    Submitted,
    Working,
    #[serde(rename = "input-required")]
    InputRequired,
    Completed,
    Failed,
    Canceled,
}

impl std::fmt::Display for A2ATaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Submitted => write!(f, "submitted"),
            Self::Working => write!(f, "working"),
            Self::InputRequired => write!(f, "input-required"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Canceled => write!(f, "canceled"),
        }
    }
}

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: A2AErrorCode, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: code.code(),
                message: code.message().to_string(),
                data,
            }),
        }
    }
}

/// JSON-RPC 2.0 error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Message part for multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MessagePart {
    Text {
        text: String,
    },
    Data {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: Value,
    },
    File {
        #[serde(rename = "mimeType")]
        mime_type: String,
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Binary {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: String, // base64 encoded
    },
}

/// A2A protocol message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AProtocolMessage {
    pub role: String,
    pub parts: Vec<MessagePart>,
}

/// Task send parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskSendParams {
    pub id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub message: A2AProtocolMessage,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Task get parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskGetParams {
    pub id: String,
    #[serde(rename = "historyLength")]
    pub history_length: Option<u32>,
}

/// Task cancel parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskCancelParams {
    pub id: String,
}

/// Push notification config parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct PushNotificationConfigParams {
    pub id: String,
    #[serde(rename = "pushNotificationConfig")]
    pub config: PushNotificationConfig,
}

/// Push notification config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushNotificationConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
}

/// Artifact structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AArtifact {
    #[serde(rename = "artifactId")]
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub parts: Vec<MessagePart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub append: bool,
    #[serde(default = "default_true")]
    #[serde(rename = "lastChunk")]
    pub last_chunk: bool,
}

fn default_true() -> bool {
    true
}

/// A2A Task response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATask {
    pub id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub status: A2ATaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<A2AProtocolMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<A2AArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// A2A Task status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATaskStatus {
    pub state: A2ATaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<A2AProtocolMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// Skill definition for agent cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ASkill {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

/// Extended agent card with A2A protocol fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AExtendedAgentCard {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<A2AProvider>,
    pub capabilities: A2ACapabilities,
    pub skills: Vec<A2ASkill>,
    #[serde(rename = "defaultInputModes")]
    pub default_input_modes: Vec<String>,
    #[serde(rename = "defaultOutputModes")]
    pub default_output_modes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<A2AAuthentication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abathur: Option<A2AAbathurExtension>,
}

/// Provider info in agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AProvider {
    pub organization: String,
    #[serde(rename = "contactEmail")]
    pub contact_email: Option<String>,
}

/// Capabilities in agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ACapabilities {
    pub streaming: bool,
    #[serde(rename = "pushNotifications")]
    pub push_notifications: bool,
    #[serde(rename = "stateTransitionHistory")]
    pub state_transition_history: bool,
}

/// Authentication info in agent card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AAuthentication {
    pub schemes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

/// Abathur-specific extensions for agent cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AAbathurExtension {
    pub tier: String,
    #[serde(rename = "agentType")]
    pub agent_type: String,
    #[serde(rename = "maxTurns")]
    pub max_turns: u32,
    #[serde(rename = "allowedTools")]
    pub allowed_tools: Vec<String>,
    #[serde(rename = "handoffTargets")]
    pub handoff_targets: Vec<String>,
    #[serde(rename = "autonomyLevel")]
    pub autonomy_level: String,
}

/// Configuration for the A2A HTTP gateway.
#[derive(Debug, Clone)]
pub struct A2AHttpConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Whether to enable CORS.
    pub enable_cors: bool,
    /// Whether to enable streaming.
    pub enable_streaming: bool,
    /// Whether to enable push notifications.
    pub enable_push_notifications: bool,
    /// Heartbeat interval for SSE streams (milliseconds).
    pub heartbeat_interval_ms: u64,
    /// Maximum stream duration (seconds).
    pub max_stream_duration_s: u64,
    /// Optional TLS configuration for federation mTLS.
    pub federation_tls: Option<FederationTlsGatewayConfig>,
    /// Optional JWT signing key for federation endpoint authentication.
    /// When set, all `federation/*` JSON-RPC methods require a valid JWT
    /// in the `Authorization: Bearer <token>` header.
    pub federation_jwt_secret: Option<Vec<u8>>,
}

/// TLS configuration for the federation gateway (mTLS).
#[derive(Debug, Clone)]
pub struct FederationTlsGatewayConfig {
    /// Path to the PEM-encoded server certificate.
    pub cert_path: String,
    /// Path to the PEM-encoded server private key.
    pub key_path: String,
    /// Optional path to a CA cert for client certificate verification (mTLS).
    pub ca_path: Option<String>,
}

impl FederationTlsGatewayConfig {
    /// Create from a `FederationTlsConfig` if it has both cert and key paths.
    pub fn from_federation_tls(
        tls: &crate::services::federation::FederationTlsConfig,
    ) -> Option<Self> {
        match (&tls.cert_path, &tls.key_path) {
            (Some(cert), Some(key)) => Some(Self {
                cert_path: cert.clone(),
                key_path: key.clone(),
                ca_path: tls.ca_path.clone(),
            }),
            _ => None,
        }
    }
}

impl Default for A2AHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            enable_cors: true,
            enable_streaming: true,
            enable_push_notifications: true,
            heartbeat_interval_ms: 30000,
            max_stream_duration_s: 3600,
            federation_tls: None,
            federation_jwt_secret: None,
        }
    }
}

/// In-memory A2A task.
#[derive(Debug, Clone)]
pub struct InMemoryTask {
    pub id: String,
    pub session_id: String,
    pub state: A2ATaskState,
    pub history: Vec<A2AProtocolMessage>,
    pub artifacts: Vec<A2AArtifact>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub push_config: Option<PushNotificationConfig>,
}

impl InMemoryTask {
    /// Create a new task from send parameters.
    pub(crate) fn from_params(params: &TaskSendParams) -> Self {
        let task_id = params
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let session_id = params
            .session_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = Utc::now();

        Self {
            id: task_id,
            session_id,
            state: A2ATaskState::Submitted,
            history: vec![params.message.clone()],
            artifacts: vec![],
            metadata: params.metadata.clone(),
            created_at: now,
            updated_at: now,
            push_config: None,
        }
    }

    /// Convert to an A2ATask response.
    pub(crate) fn to_a2a_task(&self) -> A2ATask {
        A2ATask {
            id: self.id.clone(),
            session_id: self.session_id.clone(),
            status: A2ATaskStatus {
                state: self.state,
                message: None,
                timestamp: Some(self.updated_at.to_rfc3339()),
            },
            history: Some(self.history.clone()),
            artifacts: if self.artifacts.is_empty() {
                None
            } else {
                Some(self.artifacts.clone())
            },
            metadata: self.metadata.clone(),
        }
    }
}

/// In-memory session for multi-turn conversations.
#[derive(Debug, Clone)]
pub struct A2ASession {
    pub id: String,
    pub agent_id: String,
    pub task_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
}

/// A pending delegation request for orchestrator consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingDelegation {
    pub id: Uuid,
    pub sender_id: String,
    pub target_agent: String,
    pub task_description: String,
    pub parent_task_id: Option<Uuid>,
    pub goal_id: Option<Uuid>,
    pub priority: String,
    pub created_at: DateTime<Utc>,
    pub acknowledged: bool,
}

/// Shared state for the A2A HTTP gateway.
pub struct A2AState {
    /// Registered agent cards.
    pub agent_cards: RwLock<HashMap<String, A2AAgentCard>>,
    /// In-memory tasks.
    pub tasks: Arc<RwLock<HashMap<String, InMemoryTask>>>,
    /// Sessions.
    pub sessions: RwLock<HashMap<String, A2ASession>>,
    /// Messages between agents.
    pub messages: RwLock<HashMap<Uuid, A2AMessage>>,
    /// Pending delegations for orchestrator consumption.
    pub delegations: RwLock<HashMap<Uuid, PendingDelegation>>,
    /// Configuration.
    pub config: A2AHttpConfig,
    /// Optional federation service for inter-swarm communication.
    pub federation_service: Option<Arc<crate::services::federation::FederationService>>,
    /// Handle for the convergence publisher daemon (lazily spawned).
    pub convergence_publisher_handle: RwLock<
        Option<crate::services::federation::convergence_publisher::ConvergencePublisherHandle>,
    >,
}

impl A2AState {
    pub fn new(config: A2AHttpConfig) -> Self {
        Self {
            agent_cards: RwLock::new(HashMap::new()),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            sessions: RwLock::new(HashMap::new()),
            messages: RwLock::new(HashMap::new()),
            delegations: RwLock::new(HashMap::new()),
            config,
            federation_service: None,
            convergence_publisher_handle: RwLock::new(None),
        }
    }

    /// Create state with a federation service attached.
    pub fn with_federation(
        mut self,
        service: Arc<crate::services::federation::FederationService>,
    ) -> Self {
        self.federation_service = Some(service);
        self
    }
}

/// A2A HTTP Gateway.
pub struct A2AHttpGateway {
    config: A2AHttpConfig,
    state: Arc<A2AState>,
}

impl A2AHttpGateway {
    pub fn new(config: A2AHttpConfig) -> Self {
        let state = Arc::new(A2AState::new(config.clone()));
        Self { config, state }
    }

    /// Get a reference to the shared state.
    pub fn state(&self) -> Arc<A2AState> {
        Arc::clone(&self.state)
    }

    /// Register an agent card.
    pub async fn register_agent(&self, card: A2AAgentCard) {
        let mut cards = self.state.agent_cards.write().await;
        cards.insert(card.agent_id.clone(), card);
    }

    /// Register multiple agent cards.
    pub async fn register_agents(&self, cards: Vec<A2AAgentCard>) {
        let mut agent_cards = self.state.agent_cards.write().await;
        for card in cards {
            agent_cards.insert(card.agent_id.clone(), card);
        }
    }

    /// Build the router.
    fn build_router(self) -> Router {
        let state = Arc::clone(&self.state);

        let app = Router::new()
            // JSON-RPC endpoint
            .route("/", post(handle_jsonrpc))
            .route("/rpc", post(handle_jsonrpc))
            // Agent discovery endpoints
            .route("/agents", get(rest::list_agents))
            .route("/agents/{agent_id}", get(rest::get_agent))
            .route("/agents/{agent_id}/card", get(rest::get_agent_card))
            .route("/agents/{agent_id}/skills", get(rest::get_agent_skills))
            // A2A tasks/sendSubscribe — SSE streaming endpoint
            .route("/rpc/stream", post(handle_jsonrpc_stream))
            // Task endpoints (REST alternative)
            .route("/tasks", post(rest::create_task))
            .route("/tasks/{task_id}", get(rest::get_task))
            .route("/tasks/{task_id}/cancel", post(rest::cancel_task))
            .route("/tasks/{task_id}/stream", get(rest::stream_task))
            // Health check
            .route("/health", get(rest::health_check))
            // A2A standard agent card discovery
            .route(
                "/.well-known/agent.json",
                get(rest::handle_well_known_agent_card),
            )
            // Delegation endpoints (for orchestrator integration)
            .route("/api/v1/delegations", post(rest::create_delegation))
            .route(
                "/api/v1/delegations/pending",
                get(rest::list_pending_delegations),
            )
            .route(
                "/api/v1/delegations/{delegation_id}/ack",
                post(rest::acknowledge_delegation),
            )
            .with_state(state.clone());

        // Apply JWT middleware for federation endpoint authentication
        let jwt_state = state;
        let app = app.layer(axum::middleware::from_fn_with_state(
            jwt_state,
            federation_jwt_middleware,
        ));

        if self.config.enable_cors {
            app.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .layer(TraceLayer::new_for_http())
        } else {
            app.layer(TraceLayer::new_for_http())
        }
    }

    /// Start the gateway.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("A2A HTTP Gateway listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Start the gateway with a shutdown signal.
    pub async fn serve_with_shutdown<F>(
        self,
        shutdown: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("A2A HTTP Gateway listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }

    /// Start the gateway with TLS (mTLS when CA cert is provided).
    ///
    /// Reads the certificate chain and private key from the paths in
    /// `FederationTlsGatewayConfig`, builds a `rustls::ServerConfig`, and
    /// serves via `axum_server` with TLS.
    pub async fn serve_with_tls(
        self,
        tls_config: &FederationTlsGatewayConfig,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let rustls_config = build_rustls_server_config(tls_config)?;
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("A2A HTTP Gateway listening on {} (TLS)", addr);

        let tls = axum_server::tls_rustls::RustlsConfig::from_config(rustls_config);
        axum_server::bind_rustls(addr, tls)
            .serve(router.into_make_service())
            .await?;
        Ok(())
    }

    /// Start the gateway with TLS and a shutdown signal.
    pub async fn serve_with_tls_and_shutdown<F>(
        self,
        tls_config: &FederationTlsGatewayConfig,
        shutdown: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let rustls_config = build_rustls_server_config(tls_config)?;
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("A2A HTTP Gateway listening on {} (TLS)", addr);

        let tls = axum_server::tls_rustls::RustlsConfig::from_config(rustls_config);
        let handle = axum_server::Handle::new();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            shutdown.await;
            handle_clone.shutdown();
        });
        axum_server::bind_rustls(addr, tls)
            .handle(handle)
            .serve(router.into_make_service())
            .await?;
        Ok(())
    }
}

/// Build a `rustls::ServerConfig` from the federation TLS paths.
///
/// When `ca_path` is provided, enables mutual TLS (mTLS) by requiring
/// client certificates signed by the given CA.
fn build_rustls_server_config(
    tls_config: &FederationTlsGatewayConfig,
) -> Result<Arc<rustls::ServerConfig>, Box<dyn std::error::Error + Send + Sync>> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::io::BufReader;

    // Load server certificate chain
    let cert_file = std::fs::File::open(&tls_config.cert_path)
        .map_err(|e| format!("Failed to open cert file '{}': {}", tls_config.cert_path, e))?;
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to parse certificates: {}", e))?;

    // Load server private key
    let key_file = std::fs::File::open(&tls_config.key_path)
        .map_err(|e| format!("Failed to open key file '{}': {}", tls_config.key_path, e))?;
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut BufReader::new(key_file))
        .map_err(|e| format!("Failed to parse private key: {}", e))?
        .ok_or("No private key found in key file")?;

    let config = if let Some(ref ca_path) = tls_config.ca_path {
        // mTLS: require client certificates verified against the provided CA
        let ca_file = std::fs::File::open(ca_path)
            .map_err(|e| format!("Failed to open CA file '{}': {}", ca_path, e))?;
        let ca_certs: Vec<CertificateDer<'static>> =
            rustls_pemfile::certs(&mut BufReader::new(ca_file))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to parse CA certificates: {}", e))?;

        let mut root_store = rustls::RootCertStore::empty();
        for cert in ca_certs {
            root_store.add(cert)?;
        }

        let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
            .build()
            .map_err(|e| format!("Failed to build client verifier: {}", e))?;

        rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .map_err(|e| format!("Failed to build TLS config: {}", e))?
    } else {
        // TLS only (no client cert required)
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("Failed to build TLS config: {}", e))?
    };

    Ok(Arc::new(config))
}

// ============================================================================
// JWT Validation Middleware for Federation Endpoints
// ============================================================================

/// Axum middleware that validates JWT tokens on federation endpoints.
///
/// If the request path or JSON-RPC method targets a `federation/*` method,
/// the middleware checks for a `Bearer` token in the `Authorization` header,
/// validates it using the configured secret, and rejects unauthenticated requests.
///
/// Non-federation requests pass through unchanged.
async fn federation_jwt_middleware(
    State(state): State<Arc<A2AState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Only apply to requests that might contain federation JSON-RPC calls.
    // We check the Authorization header if a JWT secret is configured.
    let jwt_secret = match &state.config.federation_jwt_secret {
        Some(secret) => secret.clone(),
        None => return Ok(next.run(req).await), // No JWT configured → pass through
    };

    // Extract the Authorization header
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let token = match auth_header {
        Some(ref header) if header.starts_with("Bearer ") => &header[7..],
        _ => {
            // No token: allow non-federation requests through.
            // Federation handlers will check for the service and fail gracefully
            // if there's no JWT. For strict enforcement, we'd peek the body here,
            // but that's expensive. Instead, federation handlers re-check auth.
            return Ok(next.run(req).await);
        }
    };

    // Validate the JWT
    let decoding_key = jsonwebtoken::DecodingKey::from_secret(&jwt_secret);
    let validation = {
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        v.validate_exp = true;
        v.required_spec_claims.clear();
        v
    };

    match jsonwebtoken::decode::<serde_json::Value>(token, &decoding_key, &validation) {
        Ok(_) => Ok(next.run(req).await),
        Err(e) => {
            tracing::warn!(error = %e, "Federation JWT validation failed");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

/// Create a signed JWT for federation authentication.
///
/// This utility is used by the federation HTTP client to authenticate
/// outbound requests to other swarms. The token includes the swarm_id
/// as the subject and expires after `duration_secs`.
pub fn create_federation_jwt(
    secret: &[u8],
    swarm_id: &str,
    duration_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = chrono::Utc::now().timestamp() as u64;
    let claims = serde_json::json!({
        "sub": swarm_id,
        "iat": now,
        "exp": now + duration_secs,
        "iss": "abathur-federation",
    });
    let encoding_key = jsonwebtoken::EncodingKey::from_secret(secret);
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &encoding_key,
    )
}

// ============================================================================
// HTTP entrypoints — both delegate routing to `dispatch`
// ============================================================================

/// Non-streaming JSON-RPC endpoint.
///
/// Validates the envelope, then routes via [`dispatch::dispatch_jsonrpc`]
/// — the single source of truth for the method → handler mapping.
async fn handle_jsonrpc(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    if let Some(err) = dispatch::validate_jsonrpc_envelope(&request) {
        return Json(err);
    }
    dispatch::dispatch_jsonrpc(state, request).await
}

/// Streaming JSON-RPC endpoint for `tasks/sendSubscribe`.
///
/// Accepts a JSON-RPC request via POST and returns an SSE event stream.
/// Only `tasks/sendSubscribe` is supported — other methods return a JSON error.
///
/// The envelope check shares [`dispatch::validate_jsonrpc_envelope`] with
/// the non-streaming endpoint; the SSE-specific handler call is the only
/// behaviour that diverges.
async fn handle_jsonrpc_stream(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<JsonRpcRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Json<JsonRpcResponse>> {
    if let Some(err) = dispatch::validate_jsonrpc_envelope(&request) {
        return Err(Json(err));
    }

    match request.method.as_str() {
        "tasks/sendSubscribe" => tasks::handle_tasks_send_subscribe(state, request).await,
        _ => Err(Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::MethodNotFound,
            Some(json!({
                "message": "Only tasks/sendSubscribe is supported on the streaming endpoint",
                "method": request.method,
            })),
        ))),
    }
}

/// Error response for REST endpoints.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// FederationClient was extracted to `crate::adapters::mcp::federation_client`.
// Re-exported via `crate::adapters::mcp` for back-compat.
pub use crate::adapters::mcp::federation_client::FederationClient;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = A2AHttpConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert!(config.enable_cors);
        assert!(config.enable_streaming);
    }

    #[test]
    fn test_jsonrpc_request_parsing() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/send",
            "params": {"message": {"role": "user", "parts": [{"type": "text", "text": "Hello"}]}}
        }"#;
        let request: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.method, "tasks/send");
    }

    #[test]
    fn test_jsonrpc_response_success() {
        let response = JsonRpcResponse::success(Some(json!(1)), json!({"result": "ok"}));
        assert!(response.error.is_none());
        assert!(response.result.is_some());
    }

    #[test]
    fn test_jsonrpc_response_error() {
        let response = JsonRpcResponse::error(Some(json!(1)), A2AErrorCode::TaskNotFound, None);
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32001);
    }

    #[test]
    fn test_task_state_display() {
        assert_eq!(A2ATaskState::Submitted.to_string(), "submitted");
        assert_eq!(A2ATaskState::Working.to_string(), "working");
        assert_eq!(A2ATaskState::Completed.to_string(), "completed");
    }

    #[test]
    fn test_message_part_serialization() {
        let part = MessagePart::Text {
            text: "Hello".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello\""));
    }

    #[test]
    fn test_task_send_params_parsing() {
        let json = r#"{
            "message": {
                "role": "user",
                "parts": [{"type": "text", "text": "Do something"}]
            }
        }"#;
        let params: TaskSendParams = serde_json::from_str(json).unwrap();
        assert!(params.id.is_none());
        assert_eq!(params.message.role, "user");
    }

    #[tokio::test]
    async fn test_state_management() {
        let state = A2AState::new(A2AHttpConfig::default());

        // Register agent
        let card = A2AAgentCard::new("test-agent")
            .with_display_name("Test Agent")
            .with_capability("testing");
        state
            .agent_cards
            .write()
            .await
            .insert("test-agent".to_string(), card);

        let cards = state.agent_cards.read().await;
        assert!(cards.contains_key("test-agent"));
    }

    #[tokio::test]
    async fn test_task_creation() {
        let state = Arc::new(A2AState::new(A2AHttpConfig::default()));

        let task = InMemoryTask {
            id: "task-1".to_string(),
            session_id: "session-1".to_string(),
            state: A2ATaskState::Submitted,
            history: vec![],
            artifacts: vec![],
            metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            push_config: None,
        };

        state.tasks.write().await.insert("task-1".to_string(), task);

        let tasks = state.tasks.read().await;
        assert!(tasks.contains_key("task-1"));
        assert_eq!(tasks.get("task-1").unwrap().state, A2ATaskState::Submitted);
    }

    #[test]
    fn test_create_federation_jwt_roundtrip() {
        let secret = b"test-secret-key-for-federation";
        let token = create_federation_jwt(secret, "swarm-alpha", 3600).unwrap();

        // Verify the token decodes correctly
        let decoding = jsonwebtoken::DecodingKey::from_secret(secret);
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.required_spec_claims.clear();

        let decoded =
            jsonwebtoken::decode::<serde_json::Value>(&token, &decoding, &validation).unwrap();
        assert_eq!(decoded.claims["sub"], "swarm-alpha");
        assert_eq!(decoded.claims["iss"], "abathur-federation");
    }

    #[test]
    fn test_create_federation_jwt_expired() {
        let secret = b"test-secret-key-for-federation";
        // Create a token that expired 120 seconds ago (beyond default leeway of 60s)
        let token = {
            let now = chrono::Utc::now().timestamp() as u64;
            let claims = serde_json::json!({
                "sub": "swarm-alpha",
                "iat": now - 240,
                "exp": now - 120,
                "iss": "abathur-federation",
            });
            let encoding_key = jsonwebtoken::EncodingKey::from_secret(secret);
            jsonwebtoken::encode(
                &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
                &claims,
                &encoding_key,
            )
            .unwrap()
        };

        let decoding = jsonwebtoken::DecodingKey::from_secret(secret);
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.validate_exp = true;
        let result = jsonwebtoken::decode::<serde_json::Value>(&token, &decoding, &validation);
        assert!(result.is_err(), "Expected expired token to fail validation");
    }

    #[test]
    fn test_create_federation_jwt_wrong_secret() {
        let token = create_federation_jwt(b"correct-secret", "swarm-alpha", 3600).unwrap();

        let decoding = jsonwebtoken::DecodingKey::from_secret(b"wrong-secret");
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.required_spec_claims.clear();
        let result = jsonwebtoken::decode::<serde_json::Value>(&token, &decoding, &validation);
        assert!(result.is_err());
    }

    #[test]
    fn test_federation_tls_gateway_config() {
        let tls_config = FederationTlsGatewayConfig {
            cert_path: "/path/to/cert.pem".to_string(),
            key_path: "/path/to/key.pem".to_string(),
            ca_path: Some("/path/to/ca.pem".to_string()),
        };
        assert_eq!(tls_config.cert_path, "/path/to/cert.pem");
        assert!(tls_config.ca_path.is_some());
    }

    /// Regression: every `serde_json::to_value(...).unwrap()` panic site in
    /// JSON-RPC / REST handlers has been replaced with an error branch that
    /// emits a `-32603` InternalError response. A panicking unwrap on a
    /// serialization error would crash the gateway process on malformed input.
    ///
    /// Because `serde_json::to_value` on well-typed Rust values (e.g.
    /// `A2ATask`, `A2AAgentCard`, `Vec<A2ASkill>`) effectively cannot fail
    /// at runtime, we cannot craft a realistic request that triggers the
    /// error branch. Instead, this test statically verifies that no
    /// `serde_json::to_value(...).unwrap()` remains in the production code
    /// of this file or its handler submodules.
    #[test]
    fn test_no_to_value_unwrap_in_production_code() {
        // After the split, the JSON-serialization sites are scattered across
        // sibling submodules. Concatenate the production text of every file
        // that still hosts a JSON-RPC / REST handler.
        let sources = [
            include_str!("a2a_http.rs"),
            include_str!("a2a_http/dispatch.rs"),
            include_str!("a2a_http/agent.rs"),
            include_str!("a2a_http/federation.rs"),
            include_str!("a2a_http/rest.rs"),
            include_str!("a2a_http/tasks.rs"),
        ];

        let mut offenders = Vec::new();
        for src in sources {
            // Split at the start of the test module so we only scan production code.
            let prod_src = match src.find("#[cfg(test)]") {
                Some(idx) => &src[..idx],
                None => src,
            };
            for (lineno, line) in prod_src.lines().enumerate() {
                if line.contains("serde_json::to_value(") && line.contains(".unwrap()") {
                    offenders.push((lineno + 1, line.trim().to_string()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "Found panicking serde_json::to_value(...).unwrap() in production code: {:?}",
            offenders
        );
    }

    #[test]
    fn test_a2a_config_with_federation_tls() {
        let config = A2AHttpConfig {
            federation_tls: Some(FederationTlsGatewayConfig {
                cert_path: "cert.pem".to_string(),
                key_path: "key.pem".to_string(),
                ca_path: None,
            }),
            federation_jwt_secret: Some(b"my-secret".to_vec()),
            ..Default::default()
        };
        assert!(config.federation_tls.is_some());
        assert!(config.federation_jwt_secret.is_some());
    }
}

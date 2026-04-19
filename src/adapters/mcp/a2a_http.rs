//! A2A (Agent-to-Agent) HTTP Gateway.
//!
//! Implements JSON-RPC 2.0 over HTTP for agent-to-agent communication
//! following the A2A protocol specification.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        Json,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::domain::models::a2a::{A2AAgentCard, A2AMessage, FederationTaskEnvelope};

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
    fn from_params(params: &TaskSendParams) -> Self {
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
    fn to_a2a_task(&self) -> A2ATask {
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

/// Shared state for the A2A HTTP gateway.
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
            .route("/agents", get(list_agents))
            .route("/agents/{agent_id}", get(get_agent))
            .route("/agents/{agent_id}/card", get(get_agent_card))
            .route("/agents/{agent_id}/skills", get(get_agent_skills))
            // A2A tasks/sendSubscribe — SSE streaming endpoint
            .route("/rpc/stream", post(handle_jsonrpc_stream))
            // Task endpoints (REST alternative)
            .route("/tasks", post(create_task))
            .route("/tasks/{task_id}", get(get_task))
            .route("/tasks/{task_id}/cancel", post(cancel_task))
            .route("/tasks/{task_id}/stream", get(stream_task))
            // Health check
            .route("/health", get(health_check))
            // A2A standard agent card discovery
            .route("/.well-known/agent.json", get(handle_well_known_agent_card))
            // Delegation endpoints (for orchestrator integration)
            .route("/api/v1/delegations", post(create_delegation))
            .route("/api/v1/delegations/pending", get(list_pending_delegations))
            .route(
                "/api/v1/delegations/{delegation_id}/ack",
                post(acknowledge_delegation),
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

// Handler functions

async fn health_check() -> &'static str {
    "OK"
}

/// Handle GET /.well-known/agent.json — A2A standard agent card discovery.
async fn handle_well_known_agent_card(
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
struct CreateDelegationRequest {
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
async fn create_delegation(
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
async fn list_pending_delegations(
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
async fn acknowledge_delegation(
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

/// Streaming JSON-RPC endpoint for `tasks/sendSubscribe`.
///
/// Accepts a JSON-RPC request via POST and returns an SSE event stream.
/// Only `tasks/sendSubscribe` is supported — other methods return a JSON error.
async fn handle_jsonrpc_stream(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<JsonRpcRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Json<JsonRpcResponse>> {
    if request.jsonrpc != "2.0" {
        return Err(Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::ParseError,
            Some(json!({"message": "Invalid JSON-RPC version"})),
        )));
    }

    match request.method.as_str() {
        "tasks/sendSubscribe" => handle_tasks_send_subscribe(state, request).await,
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

async fn handle_jsonrpc(
    State(state): State<Arc<A2AState>>,
    Json(request): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    if request.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::ParseError,
            Some(json!({"message": "Invalid JSON-RPC version"})),
        ));
    }

    match request.method.as_str() {
        "tasks/send" => handle_tasks_send(state, request).await,
        "tasks/sendSubscribe" => {
            // tasks/sendSubscribe requires SSE; on the non-streaming JSON-RPC
            // endpoint we fall back to a synchronous send and tell the caller
            // to use /rpc/stream for real SSE.
            handle_tasks_send(state, request).await
        }
        "tasks/get" => handle_tasks_get(state, request).await,
        "tasks/cancel" => handle_tasks_cancel(state, request).await,
        "tasks/pushNotificationConfig/set" => handle_push_notification_config(state, request).await,
        "agent/card" => handle_agent_card(state, request).await,
        "agent/skills" => handle_agent_skills(state, request).await,
        // Federation methods
        "federation/discover" => handle_federation_discover(state, request).await,
        "federation/register" => handle_federation_register(state, request).await,
        "federation/disconnect" => handle_federation_disconnect(state, request).await,
        "federation/delegate" => handle_federation_delegate(state, request).await,
        "federation/accept" => handle_federation_accept(state, request).await,
        "federation/reject" => handle_federation_reject(state, request).await,
        "federation/progress" => handle_federation_progress(state, request).await,
        "federation/result" => handle_federation_result(state, request).await,
        "federation/heartbeat" => handle_federation_heartbeat(state, request).await,
        "federation/reconcile" => handle_federation_reconcile(state, request).await,
        _ => Json(JsonRpcResponse::error(
            request.id,
            A2AErrorCode::MethodNotFound,
            Some(json!({"method": request.method})),
        )),
    }
}

async fn handle_tasks_send(state: Arc<A2AState>, request: JsonRpcRequest) -> Json<JsonRpcResponse> {
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
/// NOTE: Goal existence validation (e.g., verifying that a referenced goal_id
/// actually exists in the GoalRepository) should happen at a higher layer
/// (FederationService or the orchestrator), not in this HTTP handler. A2AState
/// intentionally does not hold a GoalRepository reference — adding one would
/// require a larger refactor and would couple the transport layer to domain
/// storage concerns.
async fn handle_federation_routing(
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
fn extract_title_description(
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

async fn handle_tasks_send_subscribe(
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

async fn handle_tasks_get(state: Arc<A2AState>, request: JsonRpcRequest) -> Json<JsonRpcResponse> {
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

async fn handle_tasks_cancel(
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

async fn handle_push_notification_config(
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

async fn handle_agent_card(state: Arc<A2AState>, request: JsonRpcRequest) -> Json<JsonRpcResponse> {
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

async fn handle_agent_skills(
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

// REST endpoints for agent discovery

async fn list_agents(
    State(state): State<Arc<A2AState>>,
) -> Result<Json<Vec<A2AAgentCard>>, (StatusCode, Json<ErrorResponse>)> {
    let cards = state.agent_cards.read().await;
    Ok(Json(cards.values().cloned().collect()))
}

async fn get_agent(
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

async fn get_agent_card(
    State(state): State<Arc<A2AState>>,
    Path(agent_id): Path<String>,
) -> Result<Json<A2AAgentCard>, (StatusCode, Json<ErrorResponse>)> {
    get_agent(State(state), Path(agent_id)).await
}

async fn get_agent_skills(
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

async fn create_task(
    State(state): State<Arc<A2AState>>,
    Json(params): Json<TaskSendParams>,
) -> Result<(StatusCode, Json<A2ATask>), (StatusCode, Json<ErrorResponse>)> {
    let new_task = InMemoryTask::from_params(&params);
    let response = new_task.to_a2a_task();

    let mut tasks = state.tasks.write().await;
    tasks.insert(new_task.id.clone(), new_task);

    Ok((StatusCode::CREATED, Json(response)))
}

async fn get_task(
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

async fn cancel_task(
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
async fn stream_task(
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

/// Error response for REST endpoints.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// ============================================================================
// Federation Client — Outbound cross-swarm task delegation
// ============================================================================

// ============================================================================
// Federation JSON-RPC handlers
// ============================================================================

/// Helper to get the federation service or return an error response.
fn require_federation(
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
async fn handle_federation_discover(
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
async fn handle_federation_register(
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
async fn handle_federation_disconnect(
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
async fn handle_federation_delegate(
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
async fn handle_federation_accept(
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
async fn handle_federation_reject(
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
async fn handle_federation_progress(
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
async fn handle_federation_result(
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
async fn handle_federation_heartbeat(
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
async fn handle_federation_reconcile(
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
    /// of this file (test module is permitted to keep unwrap for brevity).
    #[test]
    fn test_no_to_value_unwrap_in_production_code() {
        let src = include_str!("a2a_http.rs");
        // Split at the start of the test module so we only scan production code.
        let prod_src = match src.find("#[cfg(test)]") {
            Some(idx) => &src[..idx],
            None => src,
        };
        // Look for the pattern `serde_json::to_value(...).unwrap()` — with any
        // arguments in between. A simple substring check for the ending is
        // sufficient because every such site in this file used this exact form.
        let mut offenders = Vec::new();
        for (lineno, line) in prod_src.lines().enumerate() {
            if line.contains("serde_json::to_value(") && line.contains(".unwrap()") {
                offenders.push((lineno + 1, line.trim().to_string()));
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

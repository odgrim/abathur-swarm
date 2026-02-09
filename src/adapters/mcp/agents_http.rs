//! MCP Agents HTTP Server.
//!
//! Provides HTTP endpoints for creating, listing, and managing agent
//! templates at runtime. The Overmind uses this API to dynamically
//! create specialized agents as needed.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::domain::models::agent::{AgentConstraint, AgentTier, ToolCapability};
use crate::domain::ports::AgentRepository;
use crate::services::AgentService;

/// Configuration for the agents HTTP server.
#[derive(Debug, Clone)]
pub struct AgentsHttpConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Whether to enable CORS.
    pub enable_cors: bool,
}

impl Default for AgentsHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 9102,
            enable_cors: true,
        }
    }
}

/// Request to create a new agent.
#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub constraints: Vec<ConstraintSpec>,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

fn default_tier() -> String {
    "worker".to_string()
}

/// Tool specification in the create request.
#[derive(Debug, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

/// Constraint specification in the create request.
#[derive(Debug, Deserialize)]
pub struct ConstraintSpec {
    pub name: String,
    pub description: String,
}

/// Response for an agent.
#[derive(Debug, Serialize)]
pub struct AgentResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tier: String,
    pub version: u32,
    pub tools: Vec<ToolResponseItem>,
    pub constraints: Vec<ConstraintResponseItem>,
    pub capabilities: Vec<String>,
    pub status: String,
    pub max_turns: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Tool in the agent response.
#[derive(Debug, Serialize)]
pub struct ToolResponseItem {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// Constraint in the agent response.
#[derive(Debug, Serialize)]
pub struct ConstraintResponseItem {
    pub name: String,
    pub description: String,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

/// Shared state for the agents HTTP server.
struct AppState<A: AgentRepository> {
    service: AgentService<A>,
}

/// Agents HTTP Server.
pub struct AgentsHttpServer<A: AgentRepository + 'static> {
    config: AgentsHttpConfig,
    service: AgentService<A>,
}

impl<A: AgentRepository + Clone + Send + Sync + 'static> AgentsHttpServer<A> {
    pub fn new(service: AgentService<A>, config: AgentsHttpConfig) -> Self {
        Self { config, service }
    }

    /// Build the router.
    fn build_router(self) -> Router {
        let state = Arc::new(AppState {
            service: self.service,
        });

        let app = Router::new()
            .route("/api/v1/agents", post(create_agent::<A>))
            .route("/api/v1/agents", get(list_agents::<A>))
            .route("/api/v1/agents/{name}", get(get_agent::<A>))
            .route("/api/v1/agents/{name}", delete(disable_agent::<A>))
            .route("/health", get(health_check))
            .with_state(state);

        if self.config.enable_cors {
            app.layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
                .layer(TraceLayer::new_for_http())
        } else {
            app.layer(TraceLayer::new_for_http())
        }
    }

    /// Start the server.
    pub async fn serve(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("MCP Agents HTTP server listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;
        Ok(())
    }

    /// Start the server with a shutdown signal.
    pub async fn serve_with_shutdown<F>(
        self,
        shutdown: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        let router = self.build_router();

        tracing::info!("MCP Agents HTTP server listening on {}", addr);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }
}

// Handler functions

async fn health_check() -> &'static str {
    "OK"
}

async fn create_agent<A: AgentRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<A>>>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<AgentResponse>), (StatusCode, Json<ErrorResponse>)> {
    let tier = AgentTier::parse_str(&req.tier).unwrap_or(AgentTier::Worker);

    let tools: Vec<ToolCapability> = req.tools.into_iter().map(|t| {
        let mut tool = ToolCapability::new(t.name, t.description);
        if t.required {
            tool = tool.required();
        }
        tool
    }).collect();

    let constraints: Vec<AgentConstraint> = req.constraints.into_iter().map(|c| {
        AgentConstraint::new(c.name, c.description)
    }).collect();

    match state.service.register_template(
        req.name,
        req.description,
        tier,
        req.system_prompt,
        tools,
        constraints,
        req.max_turns,
    ).await {
        Ok(template) => Ok((StatusCode::CREATED, Json(to_response(&template)))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "CREATE_ERROR".to_string(),
            }),
        )),
    }
}

async fn list_agents<A: AgentRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<A>>>,
) -> Result<Json<Vec<AgentResponse>>, (StatusCode, Json<ErrorResponse>)> {
    use crate::domain::ports::AgentFilter;

    match state.service.list_templates(AgentFilter::default()).await {
        Ok(templates) => {
            let responses: Vec<AgentResponse> = templates.iter().map(to_response).collect();
            Ok(Json(responses))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "LIST_ERROR".to_string(),
            }),
        )),
    }
}

async fn get_agent<A: AgentRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<A>>>,
    Path(name): Path<String>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.get_template(&name).await {
        Ok(Some(template)) => Ok(Json(to_response(&template))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Agent '{}' not found", name),
                code: "NOT_FOUND".to_string(),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "GET_ERROR".to_string(),
            }),
        )),
    }
}

async fn disable_agent<A: AgentRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<A>>>,
    Path(name): Path<String>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.set_template_status(&name, crate::domain::models::agent::AgentStatus::Disabled).await {
        Ok(template) => Ok(Json(to_response(&template))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "DISABLE_ERROR".to_string(),
            }),
        )),
    }
}

fn to_response(template: &crate::domain::models::agent::AgentTemplate) -> AgentResponse {
    AgentResponse {
        id: template.id.to_string(),
        name: template.name.clone(),
        description: template.description.clone(),
        tier: template.tier.as_str().to_string(),
        version: template.version,
        tools: template.tools.iter().map(|t| ToolResponseItem {
            name: t.name.clone(),
            description: t.description.clone(),
            required: t.required,
        }).collect(),
        constraints: template.constraints.iter().map(|c| ConstraintResponseItem {
            name: c.name.clone(),
            description: c.description.clone(),
        }).collect(),
        capabilities: template.agent_card.capabilities.clone(),
        status: template.status.as_str().to_string(),
        max_turns: template.max_turns,
        created_at: template.created_at.to_rfc3339(),
        updated_at: template.updated_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = AgentsHttpConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 9102);
        assert!(config.enable_cors);
    }

    #[test]
    fn test_create_request_deserialization() {
        let json = r#"{
            "name": "test-agent",
            "description": "A test agent",
            "system_prompt": "You are a test agent.",
            "tools": [{"name": "read", "description": "Read files", "required": true}],
            "constraints": [{"name": "safe", "description": "Be safe"}]
        }"#;
        let req: CreateAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "test-agent");
        assert_eq!(req.tier, "worker"); // default
        assert_eq!(req.tools.len(), 1);
        assert!(req.tools[0].required);
        assert_eq!(req.constraints.len(), 1);
    }

    #[test]
    fn test_create_request_with_tier() {
        let json = r#"{
            "name": "specialist",
            "description": "A specialist",
            "tier": "specialist",
            "system_prompt": "You are a specialist.",
            "max_turns": 35
        }"#;
        let req: CreateAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tier, "specialist");
        assert_eq!(req.max_turns, Some(35));
        assert!(req.tools.is_empty());
    }

    #[test]
    fn test_agent_response_serialization() {
        let response = AgentResponse {
            id: "test-id".to_string(),
            name: "test-agent".to_string(),
            description: "Test".to_string(),
            tier: "worker".to_string(),
            version: 1,
            tools: vec![ToolResponseItem {
                name: "read".to_string(),
                description: "Read files".to_string(),
                required: true,
            }],
            constraints: vec![],
            capabilities: vec!["coding".to_string()],
            status: "active".to_string(),
            max_turns: 25,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"name\":\"test-agent\""));
        assert!(json.contains("\"tier\":\"worker\""));
    }
}

//! MCP Memory HTTP Server.
//!
//! Provides HTTP endpoints for Claude Code agents to interact with
//! the memory system. Supports querying, storing, and updating memories.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::domain::models::{Memory, MemoryQuery, MemoryTier, MemoryType};
use crate::domain::ports::MemoryRepository;
use crate::services::MemoryService;

/// Configuration for the memory HTTP server.
#[derive(Debug, Clone)]
pub struct MemoryHttpConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
    /// Whether to enable CORS.
    pub enable_cors: bool,
}

impl Default for MemoryHttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 9100,
            enable_cors: true,
        }
    }
}

/// Request to store a new memory.
#[derive(Debug, Deserialize)]
pub struct StoreMemoryRequest {
    pub key: String,
    pub content: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
}

/// Request to update a memory.
#[derive(Debug, Deserialize)]
pub struct UpdateMemoryRequest {
    #[serde(default)]
    pub content: Option<String>,
}

/// Query parameters for memory search.
#[derive(Debug, Deserialize)]
pub struct MemoryQueryParams {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub key_pattern: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Response with a memory.
#[derive(Debug, Serialize)]
pub struct MemoryResponse {
    pub id: Uuid,
    pub key: String,
    pub content: String,
    pub namespace: String,
    pub memory_type: String,
    pub tier: String,
    pub tags: Vec<String>,
    pub access_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Memory> for MemoryResponse {
    fn from(m: Memory) -> Self {
        Self {
            id: m.id,
            key: m.key,
            content: m.content,
            namespace: m.namespace,
            memory_type: m.memory_type.as_str().to_string(),
            tier: m.tier.as_str().to_string(),
            tags: m.metadata.tags,
            access_count: m.access_count,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

/// Response with memory statistics.
#[derive(Debug, Serialize)]
pub struct MemoryStatsResponse {
    pub total: u64,
    pub working: u64,
    pub episodic: u64,
    pub semantic: u64,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

/// Shared state for the memory HTTP server.
struct AppState<M: MemoryRepository> {
    service: MemoryService<M>,
}

/// Memory HTTP Server.
pub struct MemoryHttpServer<M: MemoryRepository + 'static> {
    config: MemoryHttpConfig,
    service: MemoryService<M>,
}

impl<M: MemoryRepository + Clone + Send + Sync + 'static> MemoryHttpServer<M> {
    pub fn new(service: MemoryService<M>, config: MemoryHttpConfig) -> Self {
        Self { config, service }
    }

    /// Build the router.
    fn build_router(self) -> Router {
        let state = Arc::new(AppState {
            service: self.service,
        });

        let app = Router::new()
            // Memory CRUD operations
            .route("/api/v1/memory", get(list_memories::<M>))
            .route("/api/v1/memory", post(store_memory::<M>))
            .route("/api/v1/memory/{id}", get(get_memory::<M>))
            .route("/api/v1/memory/{id}", put(update_memory::<M>))
            .route("/api/v1/memory/{id}", delete(delete_memory::<M>))
            // Key-based operations
            .route("/api/v1/memory/key/{namespace}/{key}", get(get_by_key::<M>))
            // Search
            .route("/api/v1/memory/search", get(search_memories::<M>))
            // Statistics
            .route("/api/v1/memory/stats", get(get_stats::<M>))
            // Health check
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

        tracing::info!("MCP Memory HTTP server listening on {}", addr);

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

        tracing::info!("MCP Memory HTTP server listening on {}", addr);

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

async fn list_memories<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Query(params): Query<MemoryQueryParams>,
) -> Result<Json<Vec<MemoryResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let mut query = MemoryQuery::new();

    if let Some(ns) = &params.namespace {
        query = query.namespace(ns);
    }
    if let Some(t) = &params.tier {
        if let Some(tier) = parse_tier(t) {
            query = query.tier(tier);
        }
    }
    if let Some(pattern) = &params.key_pattern {
        query = query.key_like(pattern);
    }
    if let Some(tag) = &params.tag {
        query = query.with_tag(tag);
    }
    if let Some(search_term) = &params.search {
        query = query.search(search_term);
    }

    match state.service.query(query).await {
        Ok(memories) => {
            let memories: Vec<_> = memories
                .into_iter()
                .take(params.limit)
                .map(MemoryResponse::from)
                .collect();
            Ok(Json(memories))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "QUERY_ERROR".to_string(),
            }),
        )),
    }
}

async fn store_memory<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Json(req): Json<StoreMemoryRequest>,
) -> Result<(StatusCode, Json<MemoryResponse>), (StatusCode, Json<ErrorResponse>)> {
    let namespace = req.namespace.unwrap_or_else(|| "default".to_string());
    let tier = req
        .tier
        .as_ref()
        .and_then(|t| parse_tier(t))
        .unwrap_or(MemoryTier::Working);
    let memory_type = req
        .memory_type
        .as_ref()
        .and_then(|t| parse_memory_type(t))
        .unwrap_or(MemoryType::Fact);

    match state
        .service
        .store(
            req.key,
            req.content,
            namespace,
            tier,
            memory_type,
            None,
        )
        .await
    {
        Ok(memory) => Ok((StatusCode::CREATED, Json(MemoryResponse::from(memory)))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "STORE_ERROR".to_string(),
            }),
        )),
    }
}

async fn get_memory<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.recall(id).await {
        Ok(Some(memory)) => Ok(Json(MemoryResponse::from(memory))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Memory {} not found", id),
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

async fn get_by_key<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Path((namespace, key)): Path<(String, String)>,
) -> Result<Json<MemoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.recall_by_key(&key, &namespace).await {
        Ok(Some(memory)) => Ok(Json(MemoryResponse::from(memory))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Memory with key '{}' in namespace '{}' not found", key, namespace),
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

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    q: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

async fn search_memories<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<MemoryResponse>>, (StatusCode, Json<ErrorResponse>)> {
    match state
        .service
        .search(&params.q, params.namespace.as_deref(), params.limit)
        .await
    {
        Ok(memories) => Ok(Json(memories.into_iter().map(MemoryResponse::from).collect())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "SEARCH_ERROR".to_string(),
            }),
        )),
    }
}

async fn update_memory<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMemoryRequest>,
) -> Result<Json<MemoryResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Get existing memory
    let memory = match state.service.recall(id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Memory {} not found", id),
                    code: "NOT_FOUND".to_string(),
                }),
            ))
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "GET_ERROR".to_string(),
                }),
            ))
        }
    };

    // Update content if provided
    if let Some(content) = req.content {
        match state
            .service
            .store(
                memory.key,
                content,
                memory.namespace,
                memory.tier,
                memory.memory_type,
                Some(memory.metadata),
            )
            .await
        {
            Ok(updated) => return Ok(Json(MemoryResponse::from(updated))),
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                        code: "UPDATE_ERROR".to_string(),
                    }),
                ))
            }
        }
    }

    Ok(Json(MemoryResponse::from(memory)))
}

async fn delete_memory<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    match state.service.forget(id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "DELETE_ERROR".to_string(),
            }),
        )),
    }
}

async fn get_stats<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
) -> Result<Json<MemoryStatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.get_stats().await {
        Ok(stats) => Ok(Json(MemoryStatsResponse {
            total: stats.total(),
            working: stats.working_count,
            episodic: stats.episodic_count,
            semantic: stats.semantic_count,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "STATS_ERROR".to_string(),
            }),
        )),
    }
}

fn parse_memory_type(s: &str) -> Option<MemoryType> {
    match s.to_lowercase().as_str() {
        "fact" => Some(MemoryType::Fact),
        "code" => Some(MemoryType::Code),
        "decision" => Some(MemoryType::Decision),
        "error" => Some(MemoryType::Error),
        "pattern" => Some(MemoryType::Pattern),
        "reference" => Some(MemoryType::Reference),
        "context" => Some(MemoryType::Context),
        _ => None,
    }
}

fn parse_tier(s: &str) -> Option<MemoryTier> {
    match s.to_lowercase().as_str() {
        "working" => Some(MemoryTier::Working),
        "episodic" => Some(MemoryTier::Episodic),
        "semantic" => Some(MemoryTier::Semantic),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = MemoryHttpConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 9100);
        assert!(config.enable_cors);
    }

    #[test]
    fn test_parse_memory_type() {
        assert_eq!(parse_memory_type("fact"), Some(MemoryType::Fact));
        assert_eq!(parse_memory_type("CODE"), Some(MemoryType::Code));
        assert_eq!(parse_memory_type("Pattern"), Some(MemoryType::Pattern));
        assert_eq!(parse_memory_type("invalid"), None);
    }

    #[test]
    fn test_parse_tier() {
        assert_eq!(parse_tier("working"), Some(MemoryTier::Working));
        assert_eq!(parse_tier("EPISODIC"), Some(MemoryTier::Episodic));
        assert_eq!(parse_tier("semantic"), Some(MemoryTier::Semantic));
        assert_eq!(parse_tier("invalid"), None);
    }

    #[test]
    fn test_store_request_deserialization() {
        let json = r#"{"key": "test", "content": "hello"}"#;
        let req: StoreMemoryRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.key, "test");
        assert_eq!(req.content, "hello");
        assert!(req.namespace.is_none());
    }

    #[test]
    fn test_memory_response_serialization() {
        let response = MemoryResponse {
            id: Uuid::new_v4(),
            key: "test".to_string(),
            content: "content".to_string(),
            namespace: "default".to_string(),
            memory_type: "fact".to_string(),
            tier: "episodic".to_string(),
            tags: vec!["tag1".to_string()],
            access_count: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"key\":\"test\""));
        assert!(json.contains("\"access_count\":5"));
    }
}

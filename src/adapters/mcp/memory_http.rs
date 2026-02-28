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

use crate::domain::models::{AccessorId, Memory, MemoryQuery, MemoryTier, MemoryType};
use crate::domain::ports::MemoryRepository;
use crate::services::command_bus::{
    CommandBus, CommandEnvelope, CommandResult, CommandSource, DomainCommand, MemoryCommand,
};
use crate::services::event_bus::EventBus;
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

/// Response representing a detected memory conflict.
#[derive(Debug, Serialize)]
pub struct MemoryConflictResponse {
    pub memory_a: Uuid,
    pub memory_b: Uuid,
    pub key: String,
    pub similarity: f64,
    pub resolved: bool,
    pub resolution: Option<String>,
}

/// Response for search with conflict detection.
#[derive(Debug, Serialize)]
pub struct SearchWithConflictsResponse {
    pub memories: Vec<MemoryResponse>,
    pub conflicts: Vec<MemoryConflictResponse>,
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
    command_bus: Arc<CommandBus>,
    event_bus: Option<Arc<EventBus>>,
}

/// Memory HTTP Server.
pub struct MemoryHttpServer<M: MemoryRepository + 'static> {
    config: MemoryHttpConfig,
    service: MemoryService<M>,
    command_bus: Arc<CommandBus>,
    event_bus: Option<Arc<EventBus>>,
}

impl<M: MemoryRepository + Clone + Send + Sync + 'static> MemoryHttpServer<M> {
    pub fn new(service: MemoryService<M>, command_bus: Arc<CommandBus>, config: MemoryHttpConfig) -> Self {
        Self { config, service, command_bus, event_bus: None }
    }

    /// Set the event bus for publishing memory recall events.
    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Build the router.
    fn build_router(self) -> Router {
        let state = Arc::new(AppState {
            service: self.service,
            command_bus: self.command_bus,
            event_bus: self.event_bus,
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
            // Search with conflict detection
            .route("/api/v1/memory/search/with-conflicts", get(search_with_conflicts::<M>))
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
    if let Some(t) = &params.tier
        && let Some(tier) = MemoryTier::from_str(t) {
            query = query.tier(tier);
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
        .and_then(|t| MemoryTier::from_str(t))
        .unwrap_or(MemoryTier::Working);
    let memory_type = req
        .memory_type
        .as_ref()
        .and_then(|t| MemoryType::from_str(t))
        .unwrap_or(MemoryType::Fact);

    let cmd = DomainCommand::Memory(MemoryCommand::Store {
        key: req.key,
        content: req.content,
        namespace,
        tier,
        memory_type,
        metadata: None,
    });
    let envelope = CommandEnvelope::new(CommandSource::Mcp("memory-http".into()), cmd);

    match state.command_bus.dispatch(envelope).await {
        Ok(CommandResult::Memory(memory)) => {
            Ok((StatusCode::CREATED, Json(MemoryResponse::from(memory))))
        }
        Ok(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Unexpected command result type".to_string(),
                code: "INTERNAL_ERROR".to_string(),
            }),
        )),
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
    match state.service.recall(id, AccessorId::system("mcp-http")).await {
        Ok((Some(memory), events)) => {
            // Publish recall events via EventBus
            if let Some(ref bus) = state.event_bus {
                for event in events {
                    bus.publish(event).await;
                }
            }
            Ok(Json(MemoryResponse::from(memory)))
        }
        Ok((None, _)) => Err((
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
    match state.service.recall_by_key(&key, &namespace, AccessorId::system("mcp-http")).await {
        Ok((Some(memory), events)) => {
            // Publish recall events via EventBus
            if let Some(ref bus) = state.event_bus {
                for event in events {
                    bus.publish(event).await;
                }
            }
            Ok(Json(MemoryResponse::from(memory)))
        }
        Ok((None, _)) => Err((
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

/// Search with conflict detection - returns memories and any detected conflicts.
async fn search_with_conflicts<M: MemoryRepository + Clone + Send + Sync + 'static>(
    State(state): State<Arc<AppState<M>>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchWithConflictsResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state
        .service
        .search_with_conflict_detection(&params.q, params.namespace.as_deref(), params.limit)
        .await
    {
        Ok(result) => {
            let memories = result.memories.into_iter().map(MemoryResponse::from).collect();
            let conflicts = result.conflicts.into_iter().map(|c| {
                MemoryConflictResponse {
                    memory_a: c.memory_a,
                    memory_b: c.memory_b,
                    key: c.key,
                    similarity: c.similarity,
                    resolved: c.resolved,
                    resolution: c.resolution.map(|r| format!("{:?}", r)),
                }
            }).collect();
            Ok(Json(SearchWithConflictsResponse { memories, conflicts }))
        }
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
    let memory = match state.service.recall(id, AccessorId::system("mcp-http")).await {
        Ok((Some(m), events)) => {
            // Publish recall events via EventBus
            if let Some(ref bus) = state.event_bus {
                for event in events {
                    bus.publish(event).await;
                }
            }
            m
        }
        Ok((None, _)) => {
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
        let cmd = DomainCommand::Memory(MemoryCommand::Store {
            key: memory.key,
            content,
            namespace: memory.namespace,
            tier: memory.tier,
            memory_type: memory.memory_type,
            metadata: Some(memory.metadata),
        });
        let envelope = CommandEnvelope::new(CommandSource::Mcp("memory-http".into()), cmd);

        match state.command_bus.dispatch(envelope).await {
            Ok(CommandResult::Memory(updated)) => return Ok(Json(MemoryResponse::from(updated))),
            Ok(_) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Unexpected command result type".to_string(),
                        code: "INTERNAL_ERROR".to_string(),
                    }),
                ))
            }
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
    let cmd = DomainCommand::Memory(MemoryCommand::Forget { id });
    let envelope = CommandEnvelope::new(CommandSource::Mcp("memory-http".into()), cmd);

    match state.command_bus.dispatch(envelope).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
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

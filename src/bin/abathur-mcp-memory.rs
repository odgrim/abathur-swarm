//! MCP server for Abathur memory management
//!
//! This MCP server exposes memory management operations via the Model Context Protocol.
//! It uses stdio transport (stdin/stdout for JSON-RPC communication).
//!
//! # Usage
//!
//! ```bash
//! abathur-mcp-memory --db-path /path/to/abathur.db
//! ```
//!
//! # Tools Exposed
//!
//! - `memory_add` - Add a new memory entry
//! - `memory_get` - Get memory by namespace and key
//! - `memory_search` - Search memories by namespace prefix and type
//! - `memory_update` - Update an existing memory value
//! - `memory_delete` - Soft delete a memory entry

use abathur_cli::{
    domain::models::{Memory, MemoryType},
    infrastructure::database::{connection::DatabaseConnection, memory_repo::MemoryRepositoryImpl},
    services::MemoryService,
};
use anyhow::{Context, Result};
use clap::Parser;
use rmcp::{
    handler::server::wrapper::Parameters, model::ServerInfo, tool, tool_handler, tool_router,
    ErrorData as McpError, Json, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "abathur-mcp-memory")]
#[command(about = "MCP server for Abathur memory management")]
struct Args {
    /// Path to SQLite database file
    #[arg(long, default_value = ".abathur/abathur.db")]
    db_path: String,
}

/// Request parameters for adding a memory
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemoryAddRequest {
    /// Hierarchical namespace (e.g., "user:alice:preferences")
    namespace: String,
    /// Unique key within the namespace
    key: String,
    /// JSON value to store
    value: serde_json::Value,
    /// Type of memory (semantic, episodic, procedural)
    memory_type: String,
    /// Identifier of who is creating this memory
    created_by: String,
}

/// Request parameters for getting a memory
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemoryGetRequest {
    /// Hierarchical namespace
    namespace: String,
    /// Key within the namespace
    key: String,
}

/// Request parameters for searching memories
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemorySearchRequest {
    /// Namespace prefix to match (e.g., "user:alice" matches "user:alice:*")
    namespace_prefix: String,
    /// Optional filter by memory type
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_type: Option<String>,
    /// Maximum number of results to return
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Request parameters for updating a memory
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemoryUpdateRequest {
    /// Hierarchical namespace
    namespace: String,
    /// Key within the namespace
    key: String,
    /// New JSON value
    value: serde_json::Value,
    /// Identifier of who is updating
    updated_by: String,
}

/// Request parameters for deleting a memory
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemoryDeleteRequest {
    /// Hierarchical namespace
    namespace: String,
    /// Key within the namespace
    key: String,
}

/// Search results
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemorySearchResult {
    /// Number of memories found
    count: usize,
    /// List of memories
    memories: Vec<Memory>,
}

/// Memory MCP Server implementation
#[derive(Clone)]
struct MemoryServer {
    memory_service: Arc<MemoryService>,
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
}

impl MemoryServer {
    fn new(memory_service: Arc<MemoryService>) -> Self {
        Self {
            memory_service,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl MemoryServer {
    /// Add a new memory entry
    #[tool(description = "Add a new memory entry to the memory system")]
    async fn memory_add(&self, params: Parameters<MemoryAddRequest>) -> Result<String, McpError> {
        let params = params.0;

        let memory_type: MemoryType = params
            .memory_type
            .parse()
            .map_err(|e| McpError::invalid_params(format!("Invalid memory_type: {}", e), None))?;

        let memory = Memory::new(
            params.namespace,
            params.key,
            params.value,
            memory_type,
            params.created_by,
        );

        match self.memory_service.add(memory).await {
            Ok(id) => {
                info!("Memory added successfully with ID: {}", id);
                Ok(format!("Memory added successfully with ID: {}", id))
            }
            Err(e) => {
                error!("Failed to add memory: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to add memory: {}",
                    e
                ), None))
            }
        }
    }

    /// Get memory by namespace and key
    #[tool(description = "Get memory by namespace and key")]
    async fn memory_get(
        &self,
        params: Parameters<MemoryGetRequest>,
    ) -> Result<Json<Memory>, McpError> {
        let params = params.0;

        match self
            .memory_service
            .get(&params.namespace, &params.key)
            .await
        {
            Ok(Some(memory)) => {
                info!("Memory found: {}", memory.namespace_path());
                Ok(Json(memory))
            }
            Ok(None) => Err(McpError::invalid_params(format!(
                "Memory not found: {}:{}",
                params.namespace, params.key
            ), None)),
            Err(e) => {
                error!("Failed to get memory: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to get memory: {}",
                    e
                ), None))
            }
        }
    }

    /// Search memories by namespace prefix
    #[tool(description = "Search memories by namespace prefix and optional type filter")]
    async fn memory_search(
        &self,
        params: Parameters<MemorySearchRequest>,
    ) -> Result<Json<MemorySearchResult>, McpError> {
        let params = params.0;

        let memory_type: Option<MemoryType> = params
            .memory_type
            .as_ref()
            .and_then(|s| s.parse().ok());

        match self
            .memory_service
            .search(&params.namespace_prefix, memory_type, Some(params.limit))
            .await
        {
            Ok(memories) => {
                info!("Found {} memories", memories.len());
                Ok(Json(MemorySearchResult {
                    count: memories.len(),
                    memories,
                }))
            }
            Err(e) => {
                error!("Failed to search memories: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to search memories: {}",
                    e
                ), None))
            }
        }
    }

    /// Update an existing memory
    #[tool(description = "Update an existing memory value")]
    async fn memory_update(
        &self,
        params: Parameters<MemoryUpdateRequest>,
    ) -> Result<String, McpError> {
        let params = params.0;

        match self
            .memory_service
            .update(
                &params.namespace,
                &params.key,
                params.value,
                &params.updated_by,
            )
            .await
        {
            Ok(()) => {
                info!("Memory updated successfully: {}:{}", params.namespace, params.key);
                Ok(format!(
                    "Memory updated successfully: {}:{}",
                    params.namespace, params.key
                ))
            }
            Err(e) => {
                error!("Failed to update memory: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to update memory: {}",
                    e
                ), None))
            }
        }
    }

    /// Delete a memory entry
    #[tool(description = "Soft delete a memory entry")]
    async fn memory_delete(
        &self,
        params: Parameters<MemoryDeleteRequest>,
    ) -> Result<String, McpError> {
        let params = params.0;

        match self
            .memory_service
            .delete(&params.namespace, &params.key)
            .await
        {
            Ok(()) => {
                info!("Memory deleted successfully: {}:{}", params.namespace, params.key);
                Ok(format!(
                    "Memory deleted successfully: {}:{}",
                    params.namespace, params.key
                ))
            }
            Err(e) => {
                error!("Failed to delete memory: {}", e);
                Err(McpError::internal_error(format!(
                    "Failed to delete memory: {}",
                    e
                ), None))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: rmcp::model::ProtocolVersion::default(),
            capabilities: rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: rmcp::model::Implementation {
                name: "abathur-memory".to_string(),
                title: Some("Abathur Memory Management Server".to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Memory management server for Abathur. Use tools to add, get, search, update, and delete memory entries.".to_string()
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing to stderr (stdout is reserved for MCP JSON-RPC)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    info!("Starting Abathur Memory MCP server");
    info!("Database path: {}", args.db_path);

    // Initialize database connection
    let database_url = format!("sqlite:{}", args.db_path);
    let db = DatabaseConnection::new(&database_url)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    db.migrate()
        .await
        .context("Failed to run database migrations")?;

    // Initialize memory repository and service
    let memory_repo = Arc::new(MemoryRepositoryImpl::new(db.pool().clone()));
    let memory_service = Arc::new(MemoryService::new(memory_repo));

    info!("Database initialized successfully");

    // Create MCP server
    let server = MemoryServer::new(memory_service);

    info!("MCP server ready, listening on stdio");

    // Run server with stdio transport
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let service = server
        .serve((stdin, stdout))
        .await
        .context("Failed to initialize MCP server")?;

    // Keep running until the connection is closed
    service.waiting().await.context("Server error during execution")?;

    Ok(())
}

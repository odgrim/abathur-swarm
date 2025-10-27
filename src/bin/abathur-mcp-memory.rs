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
    router::RouterService, CallToolRequestParams, CallToolResult, ListToolsResult, Resource,
    Role, Server, ServerCapabilities, Tool, ToolInputSchema,
};
use serde_json::{json, Value};
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
    let server = Server::new(
        ServerCapabilities {
            tools: Some(json!({
                "supported": true
            })),
            resources: None,
            prompts: None,
        },
        "abathur-memory".to_string(),
        "1.0.0".to_string(),
    );

    // Create router service
    let router = MemoryRouter::new(memory_service);

    info!("MCP server ready, listening on stdio");

    // Run server with stdio transport
    server
        .start_with_stdio(router)
        .await
        .context("MCP server failed")?;

    Ok(())
}

/// Router implementation for memory MCP tools
struct MemoryRouter {
    memory_service: Arc<MemoryService>,
}

impl MemoryRouter {
    fn new(memory_service: Arc<MemoryService>) -> Self {
        Self { memory_service }
    }

    /// Define the tools exposed by this MCP server
    fn define_tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "memory_add".to_string(),
                description: Some("Add a new memory entry".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "namespace": {
                            "type": "string",
                            "description": "Hierarchical namespace (e.g., 'user:alice:preferences')"
                        },
                        "key": {
                            "type": "string",
                            "description": "Unique key within the namespace"
                        },
                        "value": {
                            "type": "object",
                            "description": "JSON value to store"
                        },
                        "memory_type": {
                            "type": "string",
                            "enum": ["semantic", "episodic", "procedural"],
                            "description": "Type of memory"
                        },
                        "created_by": {
                            "type": "string",
                            "description": "Identifier of who is creating this memory"
                        }
                    })),
                    required: Some(vec![
                        "namespace".to_string(),
                        "key".to_string(),
                        "value".to_string(),
                        "memory_type".to_string(),
                        "created_by".to_string(),
                    ]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "memory_get".to_string(),
                description: Some("Get memory by namespace and key".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "namespace": {
                            "type": "string",
                            "description": "Hierarchical namespace"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key within the namespace"
                        }
                    })),
                    required: Some(vec!["namespace".to_string(), "key".to_string()]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "memory_search".to_string(),
                description: Some("Search memories by namespace prefix and optional type filter".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "namespace_prefix": {
                            "type": "string",
                            "description": "Namespace prefix to match (e.g., 'user:alice' matches 'user:alice:*')"
                        },
                        "memory_type": {
                            "type": "string",
                            "enum": ["semantic", "episodic", "procedural"],
                            "description": "Optional filter by memory type"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results to return (default: 50)",
                            "default": 50
                        }
                    })),
                    required: Some(vec!["namespace_prefix".to_string()]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "memory_update".to_string(),
                description: Some("Update an existing memory value".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "namespace": {
                            "type": "string",
                            "description": "Hierarchical namespace"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key within the namespace"
                        },
                        "value": {
                            "type": "object",
                            "description": "New JSON value"
                        },
                        "updated_by": {
                            "type": "string",
                            "description": "Identifier of who is updating"
                        }
                    })),
                    required: Some(vec![
                        "namespace".to_string(),
                        "key".to_string(),
                        "value".to_string(),
                        "updated_by".to_string(),
                    ]),
                    additional_properties: None,
                },
            },
            Tool {
                name: "memory_delete".to_string(),
                description: Some("Soft delete a memory entry".to_string()),
                input_schema: ToolInputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(json!({
                        "namespace": {
                            "type": "string",
                            "description": "Hierarchical namespace"
                        },
                        "key": {
                            "type": "string",
                            "description": "Key within the namespace"
                        }
                    })),
                    required: Some(vec!["namespace".to_string(), "key".to_string()]),
                    additional_properties: None,
                },
            },
        ]
    }

    /// Handle tool calls
    async fn handle_tool_call(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        match name {
            "memory_add" => self.handle_memory_add(arguments).await,
            "memory_get" => self.handle_memory_get(arguments).await,
            "memory_search" => self.handle_memory_search(arguments).await,
            "memory_update" => self.handle_memory_update(arguments).await,
            "memory_delete" => self.handle_memory_delete(arguments).await,
            _ => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Unknown tool: {}", name)),
                    data: None,
                    annotations: None,
                }],
                is_error: Some(true),
            }),
        }
    }

    async fn handle_memory_add(&self, arguments: Value) -> Result<CallToolResult> {
        let namespace = arguments["namespace"]
            .as_str()
            .context("missing namespace")?
            .to_string();
        let key = arguments["key"]
            .as_str()
            .context("missing key")?
            .to_string();
        let value = arguments["value"].clone();
        let memory_type: MemoryType = arguments["memory_type"]
            .as_str()
            .context("missing memory_type")?
            .parse()
            .context("invalid memory_type")?;
        let created_by = arguments["created_by"]
            .as_str()
            .context("missing created_by")?
            .to_string();

        let memory = Memory::new(namespace, key, value, memory_type, created_by);

        match self.memory_service.add(memory).await {
            Ok(id) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Memory added successfully with ID: {}", id)),
                    data: Some(json!({"id": id})),
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to add memory: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to add memory: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_memory_get(&self, arguments: Value) -> Result<CallToolResult> {
        let namespace = arguments["namespace"]
            .as_str()
            .context("missing namespace")?;
        let key = arguments["key"].as_str().context("missing key")?;

        match self.memory_service.get(namespace, key).await {
            Ok(Some(memory)) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Memory found: {}", memory.namespace_path())),
                    data: Some(serde_json::to_value(&memory).unwrap()),
                    annotations: None,
                }],
                is_error: None,
            }),
            Ok(None) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Memory not found: {}:{}", namespace, key)),
                    data: None,
                    annotations: None,
                }],
                is_error: Some(true),
            }),
            Err(e) => {
                error!("Failed to get memory: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to get memory: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_memory_search(&self, arguments: Value) -> Result<CallToolResult> {
        let namespace_prefix = arguments["namespace_prefix"]
            .as_str()
            .context("missing namespace_prefix")?;
        let memory_type: Option<MemoryType> = arguments["memory_type"]
            .as_str()
            .and_then(|s| s.parse().ok());
        let limit = arguments["limit"].as_u64().map(|l| l as usize);

        match self
            .memory_service
            .search(namespace_prefix, memory_type, limit)
            .await
        {
            Ok(memories) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Found {} memories", memories.len())),
                    data: Some(json!({
                        "count": memories.len(),
                        "memories": memories
                    })),
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to search memories: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to search memories: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_memory_update(&self, arguments: Value) -> Result<CallToolResult> {
        let namespace = arguments["namespace"]
            .as_str()
            .context("missing namespace")?;
        let key = arguments["key"].as_str().context("missing key")?;
        let value = arguments["value"].clone();
        let updated_by = arguments["updated_by"]
            .as_str()
            .context("missing updated_by")?;

        match self
            .memory_service
            .update(namespace, key, value, updated_by)
            .await
        {
            Ok(()) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Memory updated successfully: {}:{}", namespace, key)),
                    data: None,
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to update memory: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to update memory: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }

    async fn handle_memory_delete(&self, arguments: Value) -> Result<CallToolResult> {
        let namespace = arguments["namespace"]
            .as_str()
            .context("missing namespace")?;
        let key = arguments["key"].as_str().context("missing key")?;

        match self.memory_service.delete(namespace, key).await {
            Ok(()) => Ok(CallToolResult {
                content: vec![rmcp::Content {
                    content_type: "text".to_string(),
                    text: Some(format!("Memory deleted successfully: {}:{}", namespace, key)),
                    data: None,
                    annotations: None,
                }],
                is_error: None,
            }),
            Err(e) => {
                error!("Failed to delete memory: {}", e);
                Ok(CallToolResult {
                    content: vec![rmcp::Content {
                        content_type: "text".to_string(),
                        text: Some(format!("Failed to delete memory: {}", e)),
                        data: None,
                        annotations: None,
                    }],
                    is_error: Some(true),
                })
            }
        }
    }
}

#[async_trait::async_trait]
impl RouterService for MemoryRouter {
    async fn list_tools(&self) -> Result<ListToolsResult> {
        Ok(ListToolsResult {
            tools: Self::define_tools(),
        })
    }

    async fn call_tool(&self, params: CallToolRequestParams) -> Result<CallToolResult> {
        info!("Tool call: {}", params.name);
        self.handle_tool_call(&params.name, params.arguments.unwrap_or(json!({})))
            .await
    }

    async fn list_resources(&self) -> Result<Vec<Resource>> {
        // Memory server doesn't expose resources
        Ok(vec![])
    }

    async fn read_resource(&self, _uri: String) -> Result<String> {
        Err(anyhow::anyhow!("Resources not supported"))
    }

    async fn list_prompts(&self) -> Result<Vec<rmcp::Prompt>> {
        // Memory server doesn't expose prompts
        Ok(vec![])
    }

    async fn get_prompt(
        &self,
        _name: String,
        _arguments: Option<Value>,
    ) -> Result<rmcp::GetPromptResult> {
        Err(anyhow::anyhow!("Prompts not supported"))
    }

    async fn set_logging_level(&self, _level: rmcp::LoggingLevel) -> Result<()> {
        // Could implement dynamic log level changes here
        Ok(())
    }
}

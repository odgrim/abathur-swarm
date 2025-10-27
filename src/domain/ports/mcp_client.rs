use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// MCP tool invocation request
#[derive(Debug, Clone)]
pub struct McpToolRequest {
    pub task_id: Uuid,
    pub server_name: String,
    pub tool_name: String,
    pub arguments: Value,
}

/// MCP tool invocation response
#[derive(Debug, Clone)]
pub struct McpToolResponse {
    pub task_id: Uuid,
    pub result: Value,
    pub is_error: bool,
}

/// Detailed information about an MCP tool including its input schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    /// The name of the tool
    pub name: String,

    /// Optional human-readable description of what the tool does
    pub description: Option<String>,

    /// JSON Schema describing the tool's input parameters
    /// This allows for runtime validation and UI generation
    pub input_schema: Value,
}

/// Information about an MCP resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceInfo {
    /// Unique URI identifying the resource (e.g., `file:///path/to/file`)
    pub uri: String,

    /// Human-readable name for the resource
    pub name: String,

    /// Optional description of the resource's purpose
    pub description: Option<String>,

    /// MIME type of the resource (e.g., "text/plain", "application/json")
    pub mime_type: Option<String>,
}

/// Content retrieved from an MCP resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContent {
    /// The URI of the resource
    pub uri: String,

    /// MIME type of the content
    pub mime_type: Option<String>,

    /// Text content (for text-based resources)
    pub text: Option<String>,

    /// Binary content (for non-text resources)
    pub blob: Option<Vec<u8>>,
}

/// Error types specific to MCP operations
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("Server not found: {0}")]
    ServerNotFound(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Tool execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Timeout error")]
    Timeout,
}

/// Port trait for MCP (Model Context Protocol) client
///
/// Defines the interface for interacting with MCP servers, tools, and resources.
/// This trait follows hexagonal architecture principles by abstracting MCP
/// protocol details from the domain layer.
///
/// # Design Rationale
/// - **Tool Operations**: Enable dynamic discovery and invocation of MCP tools
/// - **Resource Operations**: Support reading structured data from MCP servers
/// - **Technology Agnostic**: Implementations can use stdio, HTTP, or other transports
/// - **Error Handling**: Uses thiserror for type-safe error variants
///
/// # Thread Safety
/// Implementations must be Send + Sync for use in multi-threaded async contexts.
/// The trait is designed to be wrapped in `Arc<dyn McpClient>` for dependency injection.
///
/// # Implementation Notes
/// Adapters implementing this trait should:
/// - Handle server lifecycle (start/stop processes if using stdio transport)
/// - Implement connection pooling and health monitoring
/// - Apply rate limiting and timeout policies
/// - Provide proper error context for debugging
///
/// # Example Usage
/// ```ignore
/// let client: Arc<dyn McpClient> = Arc::new(StdioMcpClient::new(config));
///
/// // Discover available tools
/// let tools = client.list_tools("github").await?;
///
/// // Invoke a tool
/// let request = McpToolRequest {
///     task_id: Uuid::new_v4(),
///     server_name: "github".to_string(),
///     tool_name: "create_issue".to_string(),
///     arguments: json!({ "title": "Bug report" }),
/// };
/// let response = client.invoke_tool(request).await?;
/// ```
#[async_trait]
pub trait McpClient: Send + Sync {
    /// Invoke an MCP tool with structured request/response
    ///
    /// This is the primary method for executing MCP tools with full metadata.
    ///
    /// # Arguments
    /// * `request` - The MCP tool request with server, tool name, and arguments
    ///
    /// # Returns
    /// * `Ok(McpToolResponse)` - Successful tool execution result
    /// * `Err(McpError)` - Tool error, connection error, or invalid request
    ///
    /// # Errors
    /// - `McpError::ServerNotFound` - MCP server not configured (non-retryable)
    /// - `McpError::ToolNotFound` - Tool doesn't exist on server (non-retryable)
    /// - `McpError::InvalidArguments` - Invalid tool arguments (non-retryable)
    /// - `McpError::ExecutionFailed` - Tool execution failed (check message)
    /// - `McpError::ConnectionError` - Connection to server failed (retryable)
    /// - `McpError::Timeout` - Tool execution timed out (retryable)
    async fn invoke_tool(&self, request: McpToolRequest) -> Result<McpToolResponse, McpError>;

    /// Call an MCP tool with simplified arguments (convenience method)
    ///
    /// This is a lighter-weight alternative to `invoke_tool` for simple use cases
    /// where you don't need the full request/response metadata.
    ///
    /// # Arguments
    /// * `server` - Name of the MCP server
    /// * `tool` - Name of the tool to invoke
    /// * `args` - JSON arguments for the tool
    ///
    /// # Returns
    /// * `Ok(Value)` - JSON result from tool execution
    /// * `Err(McpError)` - Error during tool execution
    ///
    /// # Errors
    /// Same error variants as `invoke_tool`
    async fn call_tool(&self, server: &str, tool: &str, args: Value) -> Result<Value, McpError>;

    /// List available tools from a specific MCP server with full metadata
    ///
    /// Returns detailed information about each tool including its input schema,
    /// which enables runtime validation and automatic UI generation.
    ///
    /// # Arguments
    /// * `server` - Name of the MCP server to query
    ///
    /// # Returns
    /// * `Ok(Vec<ToolInfo>)` - List of available tools with schemas
    /// * `Err(McpError)` - Server not found or connection error
    ///
    /// # Errors
    /// - `McpError::ServerNotFound` - Server doesn't exist
    /// - `McpError::ConnectionError` - Failed to connect to server
    /// - `McpError::Timeout` - Query timed out
    async fn list_tools(&self, server: &str) -> Result<Vec<ToolInfo>, McpError>;

    /// Read content from an MCP resource
    ///
    /// Resources provide access to structured data from MCP servers.
    /// This could be file contents, API responses, or other data sources.
    ///
    /// # Arguments
    /// * `server` - Name of the MCP server
    /// * `uri` - URI of the resource to read
    ///
    /// # Returns
    /// * `Ok(ResourceContent)` - Resource content (text or binary)
    /// * `Err(McpError)` - Resource not found or read error
    ///
    /// # Errors
    /// - `McpError::ServerNotFound` - Server doesn't exist
    /// - `McpError::ResourceNotFound` - Resource URI not found
    /// - `McpError::ConnectionError` - Failed to connect to server
    /// - `McpError::Timeout` - Read operation timed out
    async fn read_resource(&self, server: &str, uri: &str) -> Result<ResourceContent, McpError>;

    /// List available resources from a specific MCP server
    ///
    /// Returns metadata about all resources exposed by the server.
    ///
    /// # Arguments
    /// * `server` - Name of the MCP server to query
    ///
    /// # Returns
    /// * `Ok(Vec<ResourceInfo>)` - List of available resources
    /// * `Err(McpError)` - Server not found or connection error
    ///
    /// # Errors
    /// - `McpError::ServerNotFound` - Server doesn't exist
    /// - `McpError::ConnectionError` - Failed to connect to server
    /// - `McpError::Timeout` - Query timed out
    async fn list_resources(&self, server: &str) -> Result<Vec<ResourceInfo>, McpError>;

    /// Health check for MCP server connectivity
    ///
    /// Verifies that the server is reachable and responding.
    /// Can be used for monitoring and circuit breaker patterns.
    ///
    /// # Arguments
    /// * `server_name` - Name of the MCP server to check
    ///
    /// # Returns
    /// * `Ok(())` - Server is reachable and healthy
    /// * `Err(McpError)` - Server is unreachable or unhealthy
    ///
    /// # Errors
    /// - `McpError::ServerNotFound` - Server not configured
    /// - `McpError::ConnectionError` - Cannot connect to server
    /// - `McpError::Timeout` - Health check timed out
    async fn health_check(&self, server_name: &str) -> Result<(), McpError>;
}

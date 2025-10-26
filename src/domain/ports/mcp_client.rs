use async_trait::async_trait;
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

/// Error types specific to MCP operations
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("Server not found: {0}")]
    ServerNotFound(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

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
/// Defines the interface for interacting with MCP servers and tools.
/// Implementations must handle:
/// - Server lifecycle management
/// - Tool discovery and invocation
/// - Error handling and recovery
#[async_trait]
pub trait McpClient: Send + Sync {
    /// Invoke an MCP tool
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

    /// List available tools from a specific MCP server
    ///
    /// # Arguments
    /// * `server_name` - Name of the MCP server to query
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of available tool names
    /// * `Err(McpError)` - Server not found or connection error
    async fn list_tools(&self, server_name: &str) -> Result<Vec<String>, McpError>;

    /// Health check for MCP server connectivity
    ///
    /// # Arguments
    /// * `server_name` - Name of the MCP server to check
    ///
    /// # Returns
    /// * `Ok(())` - Server is reachable and healthy
    /// * `Err(McpError)` - Server is unreachable or unhealthy
    async fn health_check(&self, server_name: &str) -> Result<(), McpError>;
}

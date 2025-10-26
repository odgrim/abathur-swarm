use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Represents an MCP tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// JSON Schema for tool input
    pub input_schema: Value,
}

/// Represents an MCP resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Resource URI
    pub uri: String,
    /// Resource name
    pub name: String,
    /// MIME type of resource
    pub mime_type: Option<String>,
}

/// Client interface for Model Context Protocol (MCP) operations
///
/// Provides methods to interact with MCP servers via stdio transport.
/// Implementations should handle server lifecycle, health monitoring,
/// and JSON-RPC communication.
///
/// # Example
///
/// ```rust,no_run
/// use abathur::domain::ports::McpClient;
/// use anyhow::Result;
/// use serde_json::json;
///
/// async fn example(client: &dyn McpClient) -> Result<()> {
///     // List available tools from GitHub MCP server
///     let tools = client.list_tools("github-mcp").await?;
///
///     // Call a tool
///     let result = client.call_tool(
///         "github-mcp",
///         "list_repositories",
///         json!({"org": "anthropics"})
///     ).await?;
///
///     // List available resources
///     let resources = client.list_resources("github-mcp").await?;
///
///     // Read a resource
///     let content = client.read_resource(
///         "github-mcp",
///         "repo://anthropics/claude-code"
///     ).await?;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait McpClient: Send + Sync {
    /// List available tools from an MCP server
    ///
    /// # Arguments
    /// * `server` - The name of the MCP server
    ///
    /// # Returns
    /// * `Ok(Vec<Tool>)` - List of available tools
    /// * `Err(_)` - If the server is not found, not running, or the request fails
    async fn list_tools(&self, server: &str) -> Result<Vec<Tool>>;

    /// Call a tool on an MCP server
    ///
    /// # Arguments
    /// * `server` - The name of the MCP server
    /// * `tool` - The name of the tool to call
    /// * `args` - JSON arguments for the tool
    ///
    /// # Returns
    /// * `Ok(Value)` - The tool result
    /// * `Err(_)` - If the server is not found, tool not found, or the call fails
    async fn call_tool(&self, server: &str, tool: &str, args: Value) -> Result<Value>;

    /// List available resources from an MCP server
    ///
    /// # Arguments
    /// * `server` - The name of the MCP server
    ///
    /// # Returns
    /// * `Ok(Vec<Resource>)` - List of available resources
    /// * `Err(_)` - If the server is not found, not running, or the request fails
    async fn list_resources(&self, server: &str) -> Result<Vec<Resource>>;

    /// Read a resource from an MCP server
    ///
    /// # Arguments
    /// * `server` - The name of the MCP server
    /// * `uri` - The URI of the resource to read
    ///
    /// # Returns
    /// * `Ok(String)` - The resource content
    /// * `Err(_)` - If the server is not found, resource not found, or the read fails
    async fn read_resource(&self, server: &str, uri: &str) -> Result<String>;
}

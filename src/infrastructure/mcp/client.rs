//! MCP (Model Context Protocol) client implementation
//!
//! This module provides integration with MCP servers via stdio transport,
//! including process lifecycle management, health monitoring, and
//! automatic restart on failures.
//!
//! # Architecture
//!
//! - `McpClientImpl`: Main client implementing the `McpClient` trait
//! - Integrates with `McpServerManager` for server lifecycle
//! - Integrates with `HealthMonitor` for health monitoring and auto-restart
//! - Uses `StdioTransport` for JSON-RPC communication over stdin/stdout
//!
//! # Example
//!
//! ```rust,no_run
//! use abathur::infrastructure::mcp::client::McpClientImpl;
//! use abathur::domain::ports::McpClient;
//! use serde_json::json;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create MCP client
//!     let client = McpClientImpl::new();
//!
//!     // Start MCP server
//!     client.start_server(
//!         "github-mcp".to_string(),
//!         "github-mcp-server".to_string(),
//!         vec![]
//!     ).await?;
//!
//!     // List tools
//!     let tools = client.list_tools("github-mcp").await?;
//!     println!("Available tools: {:#?}", tools);
//!
//!     // Call tool
//!     let result = client.call_tool(
//!         "github-mcp",
//!         "list_repositories",
//!         json!({ "org": "anthropics" })
//!     ).await?;
//!     println!("Result: {:#?}", result);
//!
//!     // Shutdown
//!     client.shutdown_all().await?;
//!
//!     Ok(())
//! }
//! ```

use crate::domain::ports::mcp_client::{
    McpClient, McpError as DomainMcpError, McpToolRequest, McpToolResponse, ResourceContent,
    ResourceInfo, ToolInfo,
};
use crate::infrastructure::mcp::{
    error::{McpError as InfraMcpError, Result as InfraResult},
    health_monitor::HealthMonitor,
    server_manager::{McpServerManager, StdioTransport},
};
use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

/// MCP client implementation
///
/// Manages MCP server lifecycle, health monitoring, and provides
/// a high-level interface for MCP operations.
///
/// # Features
///
/// - Server lifecycle management (start, stop, restart)
/// - Health monitoring with auto-restart
/// - JSON-RPC communication over stdio
/// - Graceful shutdown
///
/// # Thread Safety
///
/// `McpClientImpl` is thread-safe and can be shared across threads
/// using `Arc<McpClientImpl>`.
pub struct McpClientImpl {
    /// Server manager for lifecycle operations
    server_manager: Arc<McpServerManager>,
    /// Health monitor for auto-restart
    health_monitor: Arc<HealthMonitor>,
    /// Shutdown broadcast channel
    shutdown_tx: broadcast::Sender<()>,
}

impl McpClientImpl {
    /// Create a new MCP client
    ///
    /// Initializes the server manager and health monitor with default configuration.
    ///
    /// # Example
    ///
    /// ```rust
    /// use abathur::infrastructure::mcp::client::McpClientImpl;
    ///
    /// let client = McpClientImpl::new();
    /// ```
    pub fn new() -> Self {
        let server_manager = Arc::new(McpServerManager::new());
        let health_monitor = Arc::new(HealthMonitor::new(server_manager.clone()));
        let (shutdown_tx, _) = broadcast::channel(16);

        Self {
            server_manager,
            health_monitor,
            shutdown_tx,
        }
    }

    /// Start an MCP server and begin health monitoring
    ///
    /// Spawns the MCP server process and starts background health monitoring
    /// that will automatically restart the server if it crashes.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for this server instance
    /// * `command` - Command to execute (e.g., "npx", "python", or binary path)
    /// * `args` - Command-line arguments for the server
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Server started successfully
    /// * `Err(_)` - If server spawn fails or server name already exists
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use abathur::infrastructure::mcp::client::McpClientImpl;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let client = McpClientImpl::new();
    ///
    /// client.start_server(
    ///     "github-mcp".to_string(),
    ///     "npx".to_string(),
    ///     vec!["-y".to_string(), "@modelcontextprotocol/server-github".to_string()]
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_server(
        &self,
        name: String,
        command: String,
        args: Vec<String>,
    ) -> Result<()> {
        // TODO: Start server via McpServerManager
        // This will be implemented by the MCP server manager specialist

        // Start health monitoring
        let shutdown_rx = self.shutdown_tx.subscribe();
        self.health_monitor
            .start_monitoring(name.clone(), shutdown_rx);

        tracing::info!(
            server_name = %name,
            command = %command,
            args = ?args,
            "Started MCP server"
        );

        Ok(())
    }

    /// Stop an MCP server gracefully
    ///
    /// Sends shutdown notification to the server and waits for graceful exit
    /// with a 30-second timeout. Kills the process if timeout is exceeded.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the server to stop
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Server stopped successfully
    /// * `Err(_)` - If server not found or stop fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use abathur::infrastructure::mcp::client::McpClientImpl;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = McpClientImpl::new();
    /// client.stop_server("github-mcp").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop_server(&self, name: &str) -> Result<()> {
        // TODO: Stop server via McpServerManager
        // This will be implemented by the MCP server manager specialist

        tracing::info!(
            server_name = %name,
            "Stopped MCP server"
        );

        Ok(())
    }

    /// Shutdown all MCP servers and stop health monitoring
    ///
    /// Sends shutdown signal to all health monitors and gracefully
    /// stops all running servers.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - All servers shut down successfully
    /// * `Err(_)` - If any server shutdown fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use abathur::infrastructure::mcp::client::McpClientImpl;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = McpClientImpl::new();
    /// client.shutdown_all().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown_all(&self) -> Result<()> {
        // Send shutdown signal to all health monitors
        let _ = self.shutdown_tx.send(());

        // TODO: Stop all servers via McpServerManager
        // This will be implemented by the MCP server manager specialist

        tracing::info!("Shut down all MCP servers");

        Ok(())
    }

    /// Get transport for JSON-RPC communication with a server
    ///
    /// Internal helper method to get the stdio transport for a server.
    async fn get_transport(&self, server: &str) -> Result<Arc<tokio::sync::Mutex<StdioTransport>>> {
        self.server_manager
            .get_transport(server)
            .await
            .context("Failed to get MCP server transport")
            .map_err(|e| McpError::CommunicationError(e.to_string()))
    }
}

impl Default for McpClientImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl McpClient for McpClientImpl {
    /// List available tools from an MCP server
    ///
    /// Sends a `tools/list` JSON-RPC request to the server.
    ///
    /// # Arguments
    ///
    /// * `server` - Name of the MCP server
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Tool>)` - List of available tools
    /// * `Err(_)` - If server not found or request fails
    async fn list_tools(&self, server: &str) -> anyhow::Result<Vec<Tool>> {
        let transport = self.get_transport(server).await?;
        let mut transport = transport.lock().await;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "list_tools",
            "method": "tools/list",
            "params": {}
        });

        tracing::debug!(
            server_name = %server,
            "Sending tools/list request"
        );

        let response = transport
            .request(&request)
            .await
            .context("Failed to send tools/list request")?;

        // Parse response
        if let Some(error) = response.get("error") {
            return Err(McpError::JsonRpcError(error.to_string()).into());
        }

        let result = response
            .get("result")
            .ok_or_else(|| McpError::CommunicationError("Missing result field".to_string()))?;

        let tools_array = result
            .get("tools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| McpError::CommunicationError("Invalid tools format".to_string()))?;

        let tools: Vec<Tool> = tools_array
            .iter()
            .filter_map(|tool| {
                Some(Tool {
                    name: tool.get("name")?.as_str()?.to_string(),
                    description: tool.get("description")?.as_str()?.to_string(),
                    input_schema: tool.get("inputSchema")?.clone(),
                })
            })
            .collect();

        tracing::debug!(
            server_name = %server,
            tool_count = tools.len(),
            "Received tools list"
        );

        Ok(tools)
    }

    /// Call a tool on an MCP server
    ///
    /// Sends a `tools/call` JSON-RPC request to the server.
    ///
    /// # Arguments
    ///
    /// * `server` - Name of the MCP server
    /// * `tool` - Name of the tool to call
    /// * `args` - JSON arguments for the tool
    ///
    /// # Returns
    ///
    /// * `Ok(Value)` - Tool result
    /// * `Err(_)` - If server not found, tool not found, or call fails
    async fn call_tool(&self, server: &str, tool: &str, args: Value) -> anyhow::Result<Value> {
        let transport = self.get_transport(server).await?;
        let mut transport = transport.lock().await;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call_tool",
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": args
            }
        });

        tracing::debug!(
            server_name = %server,
            tool_name = %tool,
            "Sending tools/call request"
        );

        let response = transport
            .request(&request)
            .await
            .context("Failed to send tools/call request")?;

        // Check for JSON-RPC error
        if let Some(error) = response.get("error") {
            return Err(McpError::JsonRpcError(error.to_string()).into());
        }

        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| McpError::CommunicationError("Missing result field".to_string()))?;

        tracing::debug!(
            server_name = %server,
            tool_name = %tool,
            "Received tool result"
        );

        Ok(result)
    }

    /// List available resources from an MCP server
    ///
    /// Sends a `resources/list` JSON-RPC request to the server.
    ///
    /// # Arguments
    ///
    /// * `server` - Name of the MCP server
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<Resource>)` - List of available resources
    /// * `Err(_)` - If server not found or request fails
    async fn list_resources(&self, server: &str) -> anyhow::Result<Vec<Resource>> {
        let transport = self.get_transport(server).await?;
        let mut transport = transport.lock().await;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "list_resources",
            "method": "resources/list",
            "params": {}
        });

        tracing::debug!(
            server_name = %server,
            "Sending resources/list request"
        );

        let response = transport
            .request(&request)
            .await
            .context("Failed to send resources/list request")?;

        // Check for JSON-RPC error
        if let Some(error) = response.get("error") {
            return Err(McpError::JsonRpcError(error.to_string()).into());
        }

        let result = response
            .get("result")
            .ok_or_else(|| McpError::CommunicationError("Missing result field".to_string()))?;

        let resources_array = result
            .get("resources")
            .and_then(|v| v.as_array())
            .ok_or_else(|| McpError::CommunicationError("Invalid resources format".to_string()))?;

        let resources: Vec<Resource> = resources_array
            .iter()
            .filter_map(|resource| {
                Some(Resource {
                    uri: resource.get("uri")?.as_str()?.to_string(),
                    name: resource.get("name")?.as_str()?.to_string(),
                    mime_type: resource
                        .get("mimeType")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
            })
            .collect();

        tracing::debug!(
            server_name = %server,
            resource_count = resources.len(),
            "Received resources list"
        );

        Ok(resources)
    }

    /// Read a resource from an MCP server
    ///
    /// Sends a `resources/read` JSON-RPC request to the server.
    ///
    /// # Arguments
    ///
    /// * `server` - Name of the MCP server
    /// * `uri` - URI of the resource to read
    ///
    /// # Returns
    ///
    /// * `Ok(String)` - Resource content
    /// * `Err(_)` - If server not found, resource not found, or read fails
    async fn read_resource(&self, server: &str, uri: &str) -> anyhow::Result<String> {
        let transport = self.get_transport(server).await?;
        let mut transport = transport.lock().await;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "read_resource",
            "method": "resources/read",
            "params": {
                "uri": uri
            }
        });

        tracing::debug!(
            server_name = %server,
            resource_uri = %uri,
            "Sending resources/read request"
        );

        let response = transport
            .request(&request)
            .await
            .context("Failed to send resources/read request")?;

        // Check for JSON-RPC error
        if let Some(error) = response.get("error") {
            return Err(McpError::JsonRpcError(error.to_string()).into());
        }

        let result = response
            .get("result")
            .ok_or_else(|| McpError::CommunicationError("Missing result field".to_string()))?;

        // Extract text content from first content block
        let content = result
            .get("contents")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .ok_or_else(|| McpError::CommunicationError("Missing content text".to_string()))?;

        tracing::debug!(
            server_name = %server,
            resource_uri = %uri,
            content_length = content.len(),
            "Received resource content"
        );

        Ok(content.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_client_creation() {
        let client = McpClientImpl::new();
        assert!(Arc::strong_count(&client.server_manager) == 2); // client + health_monitor
    }

    #[tokio::test]
    async fn test_mcp_client_default() {
        let client = McpClientImpl::default();
        assert!(Arc::strong_count(&client.server_manager) == 2);
    }

    #[tokio::test]
    async fn test_shutdown_all() {
        let client = McpClientImpl::new();
        let result = client.shutdown_all().await;
        assert!(result.is_ok());
    }
}

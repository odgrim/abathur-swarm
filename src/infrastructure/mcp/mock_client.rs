//! Mock MCP client for testing

use crate::domain::ports::mcp_client::{
    McpClient, McpError, McpToolRequest, McpToolResponse, ResourceContent, ResourceInfo, ToolInfo,
};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Mock MCP client implementation for testing
pub struct MockMcpClient;

impl MockMcpClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockMcpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl McpClient for MockMcpClient {
    async fn invoke_tool(&self, request: McpToolRequest) -> Result<McpToolResponse, McpError> {
        Ok(McpToolResponse {
            task_id: request.task_id,
            result: json!({"status": "success", "message": "Mock tool execution"}),
            is_error: false,
        })
    }

    async fn call_tool(&self, _server: &str, _tool: &str, _args: Value) -> Result<Value, McpError> {
        Ok(json!({"status": "success", "data": "Mock tool result"}))
    }

    async fn list_tools(&self, _server: &str) -> Result<Vec<ToolInfo>, McpError> {
        Ok(vec![
            ToolInfo {
                name: "mock_tool".to_string(),
                description: Some("A mock tool for testing".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "input": {"type": "string"}
                    }
                }),
            }
        ])
    }

    async fn read_resource(&self, _server: &str, uri: &str) -> Result<ResourceContent, McpError> {
        Ok(ResourceContent {
            uri: uri.to_string(),
            mime_type: Some("text/plain".to_string()),
            text: Some("Mock resource content".to_string()),
            blob: None,
        })
    }

    async fn list_resources(&self, _server: &str) -> Result<Vec<ResourceInfo>, McpError> {
        Ok(vec![
            ResourceInfo {
                uri: "mock://resource/1".to_string(),
                name: "Mock Resource".to_string(),
                description: Some("A mock resource for testing".to_string()),
                mime_type: Some("text/plain".to_string()),
            }
        ])
    }

    async fn health_check(&self, _server_name: &str) -> Result<(), McpError> {
        Ok(())
    }
}

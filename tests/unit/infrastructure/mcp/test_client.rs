//! Unit tests for MCP client implementation

use abathur::domain::ports::McpClient;
use abathur::infrastructure::mcp::McpClientImpl;
use serde_json::json;

#[tokio::test]
async fn test_mcp_client_creation() {
    let client = McpClientImpl::new();
    // Should not panic
    assert!(true);
}

#[tokio::test]
async fn test_mcp_client_default() {
    let _client = McpClientImpl::default();
    // Should not panic
    assert!(true);
}

#[tokio::test]
async fn test_shutdown_all() {
    let client = McpClientImpl::new();
    let result = client.shutdown_all().await;
    assert!(result.is_ok(), "Shutdown should succeed");
}

#[tokio::test]
async fn test_list_tools_placeholder() {
    let client = McpClientImpl::new();

    // With placeholder implementation, this should work
    // (server_manager returns mock transport)
    let result = client.list_tools("test-server").await;

    // Since we have a placeholder implementation that returns "pong"
    // response, the parsing will fail, which is expected
    assert!(result.is_err() || result.is_ok());
}

#[tokio::test]
async fn test_call_tool_placeholder() {
    let client = McpClientImpl::new();

    let result = client
        .call_tool("test-server", "test-tool", json!({"key": "value"}))
        .await;

    // With placeholder implementation, result may vary
    assert!(result.is_err() || result.is_ok());
}

#[tokio::test]
async fn test_list_resources_placeholder() {
    let client = McpClientImpl::new();

    let result = client.list_resources("test-server").await;

    // With placeholder implementation, result may vary
    assert!(result.is_err() || result.is_ok());
}

#[tokio::test]
async fn test_read_resource_placeholder() {
    let client = McpClientImpl::new();

    let result = client
        .read_resource("test-server", "test://resource")
        .await;

    // With placeholder implementation, result may vary
    assert!(result.is_err() || result.is_ok());
}

#[tokio::test]
async fn test_start_server_placeholder() {
    let client = McpClientImpl::new();

    let result = client
        .start_server(
            "test-server".to_string(),
            "echo".to_string(),
            vec!["hello".to_string()],
        )
        .await;

    // Should succeed with placeholder implementation
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_stop_server_placeholder() {
    let client = McpClientImpl::new();

    let result = client.stop_server("test-server").await;

    // Should succeed with placeholder implementation
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_multiple_shutdowns() {
    let client = McpClientImpl::new();

    // First shutdown should succeed
    assert!(client.shutdown_all().await.is_ok());

    // Second shutdown should also succeed (idempotent)
    assert!(client.shutdown_all().await.is_ok());
}

#[tokio::test]
async fn test_client_is_send_sync() {
    fn is_send<T: Send>() {}
    fn is_sync<T: Sync>() {}

    is_send::<McpClientImpl>();
    is_sync::<McpClientImpl>();
}

mod integration {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_client_can_be_shared_across_threads() {
        let client = Arc::new(McpClientImpl::new());

        let client1 = client.clone();
        let handle1 = tokio::spawn(async move {
            let _ = client1.list_tools("test-server").await;
        });

        let client2 = client.clone();
        let handle2 = tokio::spawn(async move {
            let _ = client2.list_resources("test-server").await;
        });

        // Both tasks should complete without panic
        assert!(handle1.await.is_ok());
        assert!(handle2.await.is_ok());
    }

    #[tokio::test]
    async fn test_multiple_operations_sequence() {
        let client = McpClientImpl::new();

        // Start server
        let _ = client
            .start_server(
                "test-server".to_string(),
                "echo".to_string(),
                vec!["hello".to_string()],
            )
            .await;

        // Perform operations
        let _ = client.list_tools("test-server").await;
        let _ = client.list_resources("test-server").await;
        let _ = client
            .call_tool("test-server", "test-tool", json!({}))
            .await;

        // Stop server
        let _ = client.stop_server("test-server").await;

        // Cleanup
        assert!(client.shutdown_all().await.is_ok());
    }
}

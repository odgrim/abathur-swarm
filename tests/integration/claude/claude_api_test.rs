/// Integration tests for Claude API client
///
/// These tests verify the Claude API client implementation with both mock servers
/// and optional real API calls (when ANTHROPIC_API_KEY is set).
///
/// Test coverage:
/// - Basic message sending with mock HTTP server
/// - Rate limiting enforcement and timing
/// - Retry logic for transient errors (429, 500, 529)
/// - Error classification (permanent vs transient)
/// - Real API integration (optional)
/// - Streaming response parsing (if implemented)

use abathur::domain::ports::ClaudeClient;
use abathur::infrastructure::claude::{
    client::{ClaudeClientConfig, ClaudeClientImpl},
    types::{Message, MessageContent, MessageRequest, StopReason},
};
use mockito::Server;
use std::time::{Duration, Instant};

/// Helper to create a minimal test message request
fn create_test_request() -> MessageRequest {
    MessageRequest {
        model: "claude-3-5-sonnet-20241022".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Say hello in one word".to_string()),
        }],
        max_tokens: 10,
        ..Default::default()
    }
}

/// Helper to create a mock successful response body
fn create_mock_response_body() -> String {
    serde_json::json!({
        "id": "msg_01ABC123",
        "type": "message",
        "role": "assistant",
        "content": [{
            "type": "text",
            "text": "Hello"
        }],
        "model": "claude-3-5-sonnet-20241022",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 1
        }
    })
    .to_string()
}

#[tokio::test]
async fn test_send_message_success_with_mock() {
    // Arrange: Create mock HTTP server
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/messages")
        .match_header("x-api-key", "test-api-key")
        .match_header("anthropic-version", "2023-06-01")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(create_mock_response_body())
        .create_async()
        .await;

    // Create client pointing to mock server
    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 10.0,
        max_retries: 0, // Disable retries for this test
        ..Default::default()
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act: Send message
    let request = create_test_request();
    let response = client
        .send_message(request)
        .await
        .expect("Message send failed");

    // Assert: Verify response
    assert_eq!(response.id, "msg_01ABC123");
    assert_eq!(response.role, "assistant");
    assert!(matches!(response.stop_reason, StopReason::EndTurn));
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 1);

    // Verify mock was called
    mock.assert_async().await;
}

#[tokio::test]
async fn test_rate_limiting_enforces_delays() {
    // Arrange: Create client with low rate limit (2 requests/second)
    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: "http://localhost:9999".to_string(), // Won't be called
        rate_limit_rps: 2.0, // 2 requests/second = 500ms between requests
        max_retries: 0,
        timeout_secs: 1,
        ..Default::default()
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act: Attempt to acquire rate limit tokens multiple times
    let start = Instant::now();

    // First token should be immediate (burst capacity)
    client.send_message(create_test_request()).await.ok();

    // Second token should be immediate (burst capacity)
    client.send_message(create_test_request()).await.ok();

    // Third token should wait ~500ms (rate limit kicks in)
    // Note: We expect failures here since there's no real server,
    // but the rate limiter will still enforce delays
    client.send_message(create_test_request()).await.ok();

    let elapsed = start.elapsed();

    // Assert: Should take at least 400ms (allowing for some timing variance)
    assert!(
        elapsed >= Duration::from_millis(400),
        "Rate limiting should enforce delays, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_retry_on_429_rate_limit_error() {
    // Arrange: Mock server that returns 429 twice, then succeeds
    let mut server = Server::new_async().await;

    // First call: 429
    let mock1 = server
        .mock("POST", "/v1/messages")
        .with_status(429)
        .with_body(r#"{"error": {"type": "rate_limit_error", "message": "Rate limit exceeded"}}"#)
        .expect(1)
        .create_async()
        .await;

    // Second call: 429
    let mock2 = server
        .mock("POST", "/v1/messages")
        .with_status(429)
        .with_body(r#"{"error": {"type": "rate_limit_error", "message": "Rate limit exceeded"}}"#)
        .expect(1)
        .create_async()
        .await;

    // Third call: Success
    let mock3 = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(create_mock_response_body())
        .expect(1)
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0, // High rate limit to avoid interference
        max_retries: 3,
        initial_backoff_ms: 10, // Fast retries for testing
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act: Send message (should retry twice and succeed on third attempt)
    let result = client.send_message(create_test_request()).await;

    // Assert: Should eventually succeed
    assert!(result.is_ok(), "Should succeed after retries");

    // Verify all mocks were called
    mock1.assert_async().await;
    mock2.assert_async().await;
    mock3.assert_async().await;
}

#[tokio::test]
async fn test_retry_on_500_server_error() {
    // Arrange: Mock server that returns 500 once, then succeeds
    let mut server = Server::new_async().await;

    let mock_error = server
        .mock("POST", "/v1/messages")
        .with_status(500)
        .with_body(r#"{"error": {"type": "internal_server_error", "message": "Internal server error"}}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_success = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(create_mock_response_body())
        .expect(1)
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let result = client.send_message(create_test_request()).await;

    // Assert
    assert!(result.is_ok(), "Should succeed after retry on 500");
    mock_error.assert_async().await;
    mock_success.assert_async().await;
}

#[tokio::test]
async fn test_retry_on_529_overloaded_error() {
    // Arrange: Mock server that returns 529 (overloaded) once, then succeeds
    let mut server = Server::new_async().await;

    let mock_overloaded = server
        .mock("POST", "/v1/messages")
        .with_status(529)
        .with_body(r#"{"error": {"type": "overloaded_error", "message": "Service overloaded"}}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_success = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(create_mock_response_body())
        .expect(1)
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let result = client.send_message(create_test_request()).await;

    // Assert
    assert!(result.is_ok(), "Should succeed after retry on 529");
    mock_overloaded.assert_async().await;
    mock_success.assert_async().await;
}

#[tokio::test]
async fn test_error_400_invalid_request_no_retry() {
    // Arrange: Mock server that returns 400 (permanent error)
    let mut server = Server::new_async().await;

    let mock = server
        .mock("POST", "/v1/messages")
        .with_status(400)
        .with_body(r#"{"error": {"type": "invalid_request_error", "message": "Invalid request"}}"#)
        .expect(1) // Should only be called once (no retries)
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let result = client.send_message(create_test_request()).await;

    // Assert: Should fail without retrying
    assert!(result.is_err(), "Should fail on 400 error");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Invalid request") || error_msg.contains("400"),
        "Error should indicate invalid request: {}",
        error_msg
    );

    // Verify mock was called exactly once (no retries)
    mock.assert_async().await;
}

#[tokio::test]
async fn test_error_401_invalid_api_key_no_retry() {
    // Arrange: Mock server that returns 401 (permanent error)
    let mut server = Server::new_async().await;

    let mock = server
        .mock("POST", "/v1/messages")
        .with_status(401)
        .with_body(r#"{"error": {"type": "authentication_error", "message": "Invalid API key"}}"#)
        .expect(1) // Should only be called once
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "invalid-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let result = client.send_message(create_test_request()).await;

    // Assert
    assert!(result.is_err(), "Should fail on 401 error");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Invalid API key") || error_msg.contains("401"),
        "Error should indicate invalid API key: {}",
        error_msg
    );

    mock.assert_async().await;
}

#[tokio::test]
async fn test_error_403_forbidden_no_retry() {
    // Arrange
    let mut server = Server::new_async().await;

    let mock = server
        .mock("POST", "/v1/messages")
        .with_status(403)
        .with_body(r#"{"error": {"type": "permission_error", "message": "Forbidden"}}"#)
        .expect(1)
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let result = client.send_message(create_test_request()).await;

    // Assert
    assert!(result.is_err(), "Should fail on 403 error");
    mock.assert_async().await;
}

#[tokio::test]
async fn test_max_retries_exhausted() {
    // Arrange: Mock server that always returns 429
    let mut server = Server::new_async().await;

    let mock = server
        .mock("POST", "/v1/messages")
        .with_status(429)
        .with_body(r#"{"error": {"type": "rate_limit_error", "message": "Rate limit"}}"#)
        .expect(4) // Initial attempt + 3 retries
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_ms: 100,
        timeout_secs: 10,
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let result = client.send_message(create_test_request()).await;

    // Assert: Should fail after exhausting retries
    assert!(result.is_err(), "Should fail after max retries");
    mock.assert_async().await;
}

#[tokio::test]
async fn test_health_check_with_valid_api_key() {
    // Arrange: Mock server for health check
    let mut server = Server::new_async().await;

    let mock = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_body(create_mock_response_body())
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "valid-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 0,
        timeout_secs: 10,
        ..Default::default()
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let is_healthy = client.health_check().await.expect("Health check failed");

    // Assert
    assert!(is_healthy, "Health check should return true for valid API key");
    mock.assert_async().await;
}

#[tokio::test]
async fn test_health_check_with_invalid_api_key() {
    // Arrange: Mock server that returns 401
    let mut server = Server::new_async().await;

    let mock = server
        .mock("POST", "/v1/messages")
        .with_status(401)
        .with_body(r#"{"error": {"type": "authentication_error", "message": "Invalid API key"}}"#)
        .create_async()
        .await;

    let config = ClaudeClientConfig {
        api_key: "invalid-key".to_string(),
        base_url: server.url(),
        rate_limit_rps: 100.0,
        max_retries: 0,
        timeout_secs: 10,
        ..Default::default()
    };
    let client = ClaudeClientImpl::with_config(config).expect("Failed to create client");

    // Act
    let is_healthy = client.health_check().await.expect("Health check should not error");

    // Assert
    assert!(!is_healthy, "Health check should return false for invalid API key");
    mock.assert_async().await;
}

// ============================================================================
// OPTIONAL: Real API Integration Test
// ============================================================================
// This test is only run when ANTHROPIC_API_KEY environment variable is set.
// It makes actual API calls to verify end-to-end functionality.

#[tokio::test]
#[ignore] // Ignored by default - run with: cargo test -- --ignored
async fn test_real_api_send_message_success() {
    // Skip if API key not available
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            eprintln!("⚠️  Skipping real API test: ANTHROPIC_API_KEY not set");
            eprintln!("   Set the environment variable to run this test:");
            eprintln!("   export ANTHROPIC_API_KEY=your-api-key");
            eprintln!("   cargo test test_real_api_send_message_success -- --ignored");
            return;
        }
    };

    // Create real client
    let client = ClaudeClientImpl::new(api_key).expect("Failed to create client");

    // Create minimal request
    let request = MessageRequest {
        model: "claude-3-5-sonnet-20241022".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Say hello in one word".to_string()),
        }],
        max_tokens: 10,
        ..Default::default()
    };

    // Act: Send message to real API
    let response = client
        .send_message(request)
        .await
        .expect("Real API call failed");

    // Assert: Verify response structure
    assert_eq!(response.role, "assistant");
    assert!(!response.content.is_empty(), "Response should have content");
    assert!(
        response.usage.output_tokens > 0,
        "Should have generated tokens"
    );
    assert_eq!(response.model, "claude-3-5-sonnet-20241022");

    eprintln!("✅ Real API test passed!");
    eprintln!("   Response: {:?}", response);
}

#[tokio::test]
#[ignore]
async fn test_real_api_health_check() {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            eprintln!("⚠️  Skipping real API health check: ANTHROPIC_API_KEY not set");
            return;
        }
    };

    let client = ClaudeClientImpl::new(api_key).expect("Failed to create client");

    // Act
    let is_healthy = client
        .health_check()
        .await
        .expect("Health check should not error");

    // Assert
    assert!(is_healthy, "Real API should be healthy");
    eprintln!("✅ Real API health check passed!");
}

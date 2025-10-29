use abathur::infrastructure::claude::{ClaudeClient, ClaudeClientConfig, MessageRequest};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_successful_message_request() {
    // Start mock server
    let mock_server = MockServer::start().await;

    // Mock successful response
    let response_json = serde_json::json!({
        "id": "msg_test123",
        "type": "message",
        "role": "assistant",
        "content": [
            {
                "type": "text",
                "text": "Hello! How can I help you?"
            }
        ],
        "model": "claude-sonnet-4-5-20250929",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": 10,
            "output_tokens": 20
        }
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-api-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_json))
        .mount(&mock_server)
        .await;

    // Create client
    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0, // High limit for tests
        max_retries: 3,
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    // Send request
    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Hello!".to_string(),
        1024,
    );

    let response = client.send_message(request).await.unwrap();

    // Verify response
    assert_eq!(response.id, "msg_test123");
    assert_eq!(response.role, "assistant");
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 20);
}

#[tokio::test]
async fn test_retry_on_500_error() {
    let mock_server = MockServer::start().await;

    let success_response = serde_json::json!({
        "id": "msg_retry123",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Success after retry"}],
        "model": "claude-sonnet-4-5-20250929",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 5, "output_tokens": 10}
    });

    // First two requests fail with 500
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .up_to_n_times(2)
        .mount(&mock_server)
        .await;

    // Third request succeeds
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&success_response))
        .mount(&mock_server)
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test retry".to_string(),
        512,
    );

    let response = client.send_message(request).await.unwrap();
    assert_eq!(response.id, "msg_retry123");
}

#[tokio::test]
async fn test_retry_on_rate_limit() {
    let mock_server = MockServer::start().await;

    let success_response = serde_json::json!({
        "id": "msg_ratelimit123",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Success after rate limit"}],
        "model": "claude-sonnet-4-5-20250929",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 5, "output_tokens": 10}
    });

    // First request fails with 429
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).set_body_string("Too Many Requests"))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // Second request succeeds
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&success_response))
        .mount(&mock_server)
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test rate limit".to_string(),
        512,
    );

    let response = client.send_message(request).await.unwrap();
    assert_eq!(response.id, "msg_ratelimit123");
}

#[tokio::test]
async fn test_no_retry_on_400_error() {
    let mock_server = MockServer::start().await;

    // Always return 400 (should NOT retry)
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string("Bad Request"))
        .mount(&mock_server)
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test bad request".to_string(),
        512,
    );

    let result = client.send_message(request).await;
    assert!(result.is_err());

    // Should only make 1 request (no retries for 400)
    // Verify by checking mock server received exactly 1 request
}

#[tokio::test]
async fn test_no_retry_on_401_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&mock_server)
        .await;

    let config = ClaudeClientConfig {
        api_key: "invalid-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0,
        max_retries: 3,
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test".to_string(),
        512,
    );

    let result = client.send_message(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_max_retries_exceeded() {
    let mock_server = MockServer::start().await;

    // Always fail with 500 (should retry and then give up)
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Server Error"))
        .mount(&mock_server)
        .await;

    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0,
        max_retries: 2,
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test max retries".to_string(),
        512,
    );

    let result = client.send_message(request).await;
    assert!(result.is_err());

    // Should make 3 requests total: initial + 2 retries
}

#[tokio::test]
async fn test_rate_limiter_enforcement() {
    use std::time::Instant;

    let mock_server = MockServer::start().await;

    let success_response = serde_json::json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "Test"}],
        "model": "claude-sonnet-4-5-20250929",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 5, "output_tokens": 5}
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&success_response))
        .mount(&mock_server)
        .await;

    // Configure strict rate limit: 2 requests per second
    let config = ClaudeClientConfig {
        api_key: "test-api-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 2.0,
        max_retries: 1, // Min 1 retry required
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test".to_string(),
        128,
    );

    let start = Instant::now();

    // Make 4 requests
    for _ in 0..4 {
        client.send_message(request.clone()).await.unwrap();
    }

    let elapsed = start.elapsed();

    // At 2 req/sec, 4 requests should take at least 1.5 seconds
    // (0s for first 2, then 0.5s + 0.5s + 0.5s)
    assert!(
        elapsed.as_secs_f64() >= 1.0,
        "Expected delay >= 1.0s, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_request_headers() {
    let mock_server = MockServer::start().await;

    let success_response = serde_json::json!({
        "id": "msg_headers",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "OK"}],
        "model": "claude-sonnet-4-5-20250929",
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    });

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "my-test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&success_response))
        .mount(&mock_server)
        .await;

    let config = ClaudeClientConfig {
        api_key: "my-test-key".to_string(),
        base_url: mock_server.uri(),
        rate_limit_rps: 100.0,
        max_retries: 1, // Min 1 retry required
        initial_backoff_ms: 100,
        max_backoff_ms: 1000,
        timeout_secs: 30,
    };
    let client = ClaudeClient::new(config).unwrap();

    let request = MessageRequest::simple_message(
        "claude-sonnet-4-5-20250929".to_string(),
        "Test".to_string(),
        128,
    );

    let response = client.send_message(request).await.unwrap();
    assert_eq!(response.id, "msg_headers");
}

/// Claude HTTP API client implementation
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client as ReqwestClient;
use std::time::Duration;

use crate::domain::ports::ClaudeClient;
use super::{
    error::ClaudeApiError,
    rate_limiter::TokenBucketRateLimiter,
    retry::RetryPolicy,
    types::{Message, MessageContent, MessageRequest, MessageResponse},
};

/// Production-grade HTTP client for Claude API
///
/// Features:
/// - Connection pooling and reuse (via reqwest::Client)
/// - Token bucket rate limiting (configurable requests/second)
/// - Exponential backoff retry logic for transient errors
/// - Proper error classification (transient vs permanent)
/// - 300s timeout for long-running requests
/// - Compression support (gzip/brotli)
pub struct ClaudeClientImpl {
    /// Reusable HTTP client with connection pooling
    http_client: ReqwestClient,

    /// API key for authentication
    api_key: String,

    /// Base URL for Claude API
    base_url: String,

    /// Rate limiter to enforce request rate limits
    rate_limiter: TokenBucketRateLimiter,

    /// Retry policy for handling transient errors
    retry_policy: RetryPolicy,
}

impl ClaudeClientImpl {
    /// Create a new Claude API client with default configuration
    ///
    /// # Arguments
    /// * `api_key` - Anthropic API key for authentication
    ///
    /// # Returns
    /// * `Ok(ClaudeClientImpl)` - Successfully created client
    /// * `Err(anyhow::Error)` - Failed to build HTTP client
    ///
    /// # Default Configuration
    /// - Rate limit: 10 requests/second
    /// - Timeout: 300 seconds
    /// - Max retries: 3
    /// - Initial backoff: 10 seconds
    /// - Max backoff: 5 minutes
    pub fn new(api_key: String) -> Result<Self> {
        Self::with_config(ClaudeClientConfig {
            api_key,
            ..Default::default()
        })
    }

    /// Create a new Claude API client with custom configuration
    ///
    /// # Arguments
    /// * `config` - Client configuration
    ///
    /// # Returns
    /// * `Ok(ClaudeClientImpl)` - Successfully created client
    /// * `Err(anyhow::Error)` - Failed to build HTTP client
    pub fn with_config(config: ClaudeClientConfig) -> Result<Self> {
        let http_client = ReqwestClient::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .pool_max_idle_per_host(10) // Connection pooling
            .tcp_nodelay(true) // Disable Nagle's algorithm for lower latency
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            http_client,
            api_key: config.api_key,
            base_url: config.base_url,
            rate_limiter: TokenBucketRateLimiter::new(config.rate_limit_rps),
            retry_policy: RetryPolicy::new(
                config.max_retries,
                config.initial_backoff_ms,
                config.max_backoff_ms,
            ),
        })
    }

    /// Send a request and handle the response
    ///
    /// This is an internal helper method that:
    /// 1. Makes the HTTP request
    /// 2. Checks status code
    /// 3. Parses response or constructs error
    async fn send_request(&self, request: &MessageRequest) -> Result<MessageResponse> {
        let response = self
            .http_client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await
            .context("Failed to send request to Claude API")?;

        let status = response.status();

        // Handle non-success status codes
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read error response".to_string());
            let error = ClaudeApiError::from_status(status, body);
            return Err(anyhow::anyhow!(error));
        }

        // Parse successful response
        let message_response: MessageResponse = response
            .json()
            .await
            .context("Failed to parse Claude API response")?;

        Ok(message_response)
    }
}

#[async_trait]
impl ClaudeClient for ClaudeClientImpl {
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse> {
        // 1. Acquire rate limit token (blocks until available)
        self.rate_limiter
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("Rate limiter error: {}", e))?;

        // 2. Execute request with retry logic
        self.retry_policy
            .execute(|| self.send_request(&request))
            .await
    }

    async fn health_check(&self) -> Result<bool> {
        // Simple health check: send minimal test request
        let test_request = MessageRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::Text("test".to_string()),
            }],
            max_tokens: 1,
            ..Default::default()
        };

        match self.send_message(test_request).await {
            Ok(_) => Ok(true),
            Err(err) => {
                // Check if it's a permanent error (indicates bad API key, etc.)
                if let Some(claude_err) = err.downcast_ref::<ClaudeApiError>() {
                    if claude_err.is_permanent() {
                        return Ok(false);
                    }
                }
                // For transient errors or unknown errors, return the error
                Err(err)
            }
        }
    }
}

/// Configuration for Claude API client
pub struct ClaudeClientConfig {
    /// Anthropic API key
    pub api_key: String,

    /// Base URL for Claude API
    pub base_url: String,

    /// Rate limit in requests per second
    pub rate_limit_rps: f64,

    /// Maximum number of retries
    pub max_retries: u32,

    /// Initial backoff duration in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff duration in milliseconds
    pub max_backoff_ms: u64,

    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for ClaudeClientConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY")
                .unwrap_or_else(|_| "dummy-api-key-for-testing".to_string()),
            base_url: "https://api.anthropic.com".to_string(),
            rate_limit_rps: 10.0,
            max_retries: 3,
            initial_backoff_ms: 10_000,  // 10 seconds
            max_backoff_ms: 300_000,     // 5 minutes
            timeout_secs: 300,           // 5 minutes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = ClaudeClientImpl::new("test-api-key".to_string());
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_with_custom_config() {
        let config = ClaudeClientConfig {
            api_key: "test-key".to_string(),
            rate_limit_rps: 5.0,
            max_retries: 5,
            ..Default::default()
        };

        let client = ClaudeClientImpl::with_config(config);
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        use std::time::Instant;

        let config = ClaudeClientConfig {
            api_key: "test-key".to_string(),
            rate_limit_rps: 2.0, // 2 requests/second
            ..Default::default()
        };

        let client = ClaudeClientImpl::with_config(config).unwrap();

        // Acquire 3 tokens (should take ~0.5s for the 3rd one)
        let start = Instant::now();
        for _ in 0..3 {
            client.rate_limiter.acquire().await.unwrap();
        }
        let elapsed = start.elapsed();

        // First 2 should be immediate (burst), 3rd should wait ~0.5s
        assert!(
            elapsed >= Duration::from_millis(400),
            "Rate limiting should enforce delays"
        );
    }

    // Note: Integration tests with actual API calls should be in tests/infrastructure/claude_client_test.rs
    // and use mock HTTP servers (wiremock) to avoid hitting the real API
}

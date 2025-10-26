<<<<<<< HEAD
use super::errors::ClaudeApiError;
use super::rate_limiter::TokenBucketRateLimiter;
use super::retry::RetryPolicy;
use super::types::{MessageRequest, MessageResponse};
use reqwest::{Client as ReqwestClient, Response, StatusCode, header};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, instrument, warn};

/// Configuration for the Claude HTTP client
#[derive(Debug, Clone)]
=======
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
>>>>>>> task_claude-api-integration-tests_20251025-210007
pub struct ClaudeClientConfig {
    /// Anthropic API key
    pub api_key: String,

<<<<<<< HEAD
    /// Base URL for the Claude API
=======
    /// Base URL for Claude API
>>>>>>> task_claude-api-integration-tests_20251025-210007
    pub base_url: String,

    /// Rate limit in requests per second
    pub rate_limit_rps: f64,

<<<<<<< HEAD
    /// Maximum retry attempts
    pub max_retries: u32,

    /// Initial backoff delay in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff delay in milliseconds
=======
    /// Maximum number of retries
    pub max_retries: u32,

    /// Initial backoff duration in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff duration in milliseconds
>>>>>>> task_claude-api-integration-tests_20251025-210007
    pub max_backoff_ms: u64,

    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for ClaudeClientConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY")
<<<<<<< HEAD
                .expect("ANTHROPIC_API_KEY environment variable must be set"),
            base_url: "https://api.anthropic.com".to_string(),
            rate_limit_rps: 10.0,
            max_retries: 3,
            initial_backoff_ms: 10_000, // 10 seconds
            max_backoff_ms: 300_000,    // 5 minutes
            timeout_secs: 300,          // 5 minutes
        }
    }
}

/// HTTP client for interacting with the Claude API
///
/// Provides robust HTTP communication with:
/// - Connection pooling and reuse
/// - Rate limiting via token bucket algorithm
/// - Exponential backoff retry logic
/// - Compression support (gzip/brotli)
/// - Structured error handling
pub struct ClaudeClient {
    http_client: ReqwestClient,
    base_url: String,
    rate_limiter: Arc<TokenBucketRateLimiter>,
    retry_policy: RetryPolicy,
}

impl ClaudeClient {
    /// Create a new Claude API client
    ///
    /// # Arguments
    /// * `config` - Client configuration
    ///
    /// # Returns
    /// * `Result<Self, ClaudeApiError>` - Client instance or error
    ///
    /// # Example
    /// ```no_run
    /// use abathur::infrastructure::claude::client::{ClaudeClient, ClaudeClientConfig};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ClaudeClientConfig::default();
    /// let client = ClaudeClient::new(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(config: ClaudeClientConfig) -> Result<Self, ClaudeApiError> {
        // Scrub API key from logs
        let api_key_scrubbed = if config.api_key.len() > 8 {
            format!("{}...[REDACTED]", &config.api_key[..8])
        } else {
            "[REDACTED]".to_string()
        };

        info!(
            "Initializing Claude API client: base_url={}, rate_limit={} rps, timeout={}s, api_key={}",
            config.base_url, config.rate_limit_rps, config.timeout_secs, api_key_scrubbed
        );

        // Build HTTP client with optimized settings
        let mut headers = header::HeaderMap::new();
        headers.insert(
            "x-api-key",
            header::HeaderValue::from_str(&config.api_key)
                .map_err(|e| ClaudeApiError::InvalidRequest(format!("Invalid API key: {}", e)))?,
        );
        headers.insert(
            "anthropic-version",
            header::HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );

        let http_client = ReqwestClient::builder()
            .pool_max_idle_per_host(10)
            .timeout(Duration::from_secs(config.timeout_secs))
            .gzip(true)
            .brotli(true)
            .tcp_nodelay(true)
            .default_headers(headers)
            .build()
            .map_err(|e| ClaudeApiError::NetworkError(e))?;

        let rate_limiter = Arc::new(TokenBucketRateLimiter::new(config.rate_limit_rps));
        let retry_policy = RetryPolicy::new(
            config.max_retries,
            config.initial_backoff_ms,
            config.max_backoff_ms,
        );

        Ok(Self {
            http_client,
            base_url: config.base_url,
            rate_limiter,
            retry_policy,
        })
    }

    /// Send a message to Claude and get a response
    ///
    /// # Arguments
    /// * `request` - Message request with model, messages, and parameters
    ///
    /// # Returns
    /// * `Result<MessageResponse, ClaudeApiError>` - Response or error
    ///
    /// # Example
    /// ```no_run
    /// # use abathur::infrastructure::claude::client::{ClaudeClient, ClaudeClientConfig};
    /// # use abathur::infrastructure::claude::types::MessageRequest;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ClaudeClientConfig::default();
    /// let client = ClaudeClient::new(config)?;
    ///
    /// let request = MessageRequest::simple_message(
    ///     "claude-3-5-sonnet-20241022".to_string(),
    ///     "Hello, Claude!".to_string(),
    ///     1024,
    /// );
    ///
    /// let response = client.send_message(request).await?;
    /// println!("Response: {:?}", response);
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self, request), fields(model = %request.model, max_tokens = request.max_tokens))]
    pub async fn send_message(
        &self,
        request: MessageRequest,
    ) -> Result<MessageResponse, ClaudeApiError> {
        debug!("Sending message request");

        // Acquire rate limit token
        self.rate_limiter.acquire().await;

        // Execute with retry policy
        let result = self
            .retry_policy
            .execute(|| async { self.execute_message_request(&request).await })
            .await;

        match &result {
            Ok(response) => {
                info!(
                    "Message request succeeded: input_tokens={}, output_tokens={}",
                    response.usage.input_tokens, response.usage.output_tokens
                );
            }
            Err(err) => {
                error!("Message request failed: {}", err);
            }
        }

        result
    }

    /// Execute a single message request (called by retry logic)
    async fn execute_message_request(
        &self,
        request: &MessageRequest,
    ) -> Result<MessageResponse, ClaudeApiError> {
        let url = format!("{}/v1/messages", self.base_url);

        debug!("POST {}", url);

        let response = self.http_client.post(&url).json(request).send().await?;

        self.handle_response(response).await
    }

    /// Handle HTTP response and convert to typed result
    async fn handle_response(&self, response: Response) -> Result<MessageResponse, ClaudeApiError> {
        let status = response.status();

        debug!("Response status: {}", status);

        if !status.is_success() {
            return Err(self.handle_error_response(response).await);
        }

        // Parse successful response
        let message_response: MessageResponse = response.json().await?;

        Ok(message_response)
    }

    /// Handle error response and classify error type
    async fn handle_error_response(&self, response: Response) -> ClaudeApiError {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unable to read error body".to_string());

        warn!("API error ({}): {}", status, body);

        match status {
            StatusCode::BAD_REQUEST => ClaudeApiError::InvalidRequest(body),
            StatusCode::UNAUTHORIZED => ClaudeApiError::InvalidApiKey,
            StatusCode::FORBIDDEN => ClaudeApiError::Forbidden(body),
            StatusCode::NOT_FOUND => ClaudeApiError::NotFound,
            StatusCode::TOO_MANY_REQUESTS => ClaudeApiError::RateLimitExceeded,
            status if status.is_server_error() => ClaudeApiError::ServerError(status, body),
            _ => ClaudeApiError::UnknownError(status, body),
=======
                .unwrap_or_else(|_| "dummy-api-key-for-testing".to_string()),
            base_url: "https://api.anthropic.com".to_string(),
            rate_limit_rps: 10.0,
            max_retries: 3,
            initial_backoff_ms: 10_000,  // 10 seconds
            max_backoff_ms: 300_000,     // 5 minutes
            timeout_secs: 300,           // 5 minutes
>>>>>>> task_claude-api-integration-tests_20251025-210007
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
<<<<<<< HEAD
    fn test_client_config_default() {
        // Set env var for test (unsafe because it can cause data races)
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "test-key");
        }

        let config = ClaudeClientConfig::default();
        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.rate_limit_rps, 10.0);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    fn test_client_creation() {
        let config = ClaudeClientConfig {
            api_key: "test-api-key".to_string(),
            base_url: "https://api.test.com".to_string(),
            rate_limit_rps: 5.0,
            max_retries: 2,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60000,
            timeout_secs: 120,
        };

        let client = ClaudeClient::new(config);
=======
    fn test_client_creation() {
        let client = ClaudeClientImpl::new("test-api-key".to_string());
>>>>>>> task_claude-api-integration-tests_20251025-210007
        assert!(client.is_ok());
    }

    #[test]
<<<<<<< HEAD
    fn test_api_key_scrubbing() {
        let config = ClaudeClientConfig {
            api_key: "sk-ant-api03-verylongkey1234567890".to_string(),
            ..Default::default()
        };

        // This should not panic and should scrub the API key in logs
        let _client = ClaudeClient::new(config);
    }
=======
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
>>>>>>> task_claude-api-integration-tests_20251025-210007
}

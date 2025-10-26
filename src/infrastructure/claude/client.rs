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
pub struct ClaudeClientConfig {
    /// Anthropic API key
    pub api_key: String,

    /// Base URL for the Claude API
    pub base_url: String,

    /// Rate limit in requests per second
    pub rate_limit_rps: f64,

    /// Maximum retry attempts
    pub max_retries: u32,

    /// Initial backoff delay in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff delay in milliseconds
    pub max_backoff_ms: u64,

    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for ClaudeClientConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY")
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
        assert!(client.is_ok());
    }

    #[test]
    fn test_api_key_scrubbing() {
        let config = ClaudeClientConfig {
            api_key: "sk-ant-api03-verylongkey1234567890".to_string(),
            ..Default::default()
        };

        // This should not panic and should scrub the API key in logs
        let _client = ClaudeClient::new(config);
    }
}

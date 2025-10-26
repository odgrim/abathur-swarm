use async_trait::async_trait;
use uuid::Uuid;

/// Request to Claude API for task execution
#[derive(Debug, Clone)]
pub struct ClaudeRequest {
    pub task_id: Uuid,
    pub agent_type: String,
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

/// Response from Claude API
#[derive(Debug, Clone)]
pub struct ClaudeResponse {
    pub task_id: Uuid,
    pub content: String,
    pub stop_reason: String,
    pub usage: TokenUsage,
}

/// Token usage information
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Error types specific to Claude API
#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Timeout error")]
    Timeout,
}

/// Port trait for Claude API client
///
/// Defines the interface for interacting with Claude's API.
/// Implementations must handle:
/// - Rate limiting and backoff
/// - Retry logic for transient errors
/// - Proper error propagation
#[async_trait]
pub trait ClaudeClient: Send + Sync {
    /// Execute a prompt via Claude API
    ///
    /// # Arguments
    /// * `request` - The Claude API request with prompt and parameters
    ///
    /// # Returns
    /// * `Ok(ClaudeResponse)` - Successful response from Claude
    /// * `Err(ClaudeError)` - API error, rate limit, or network error
    ///
    /// # Errors
    /// - `ClaudeError::RateLimitExceeded` - Rate limit hit, caller should retry with backoff
    /// - `ClaudeError::InvalidApiKey` - Authentication failed (non-retryable)
    /// - `ClaudeError::NetworkError` - Network failure (retryable)
    /// - `ClaudeError::ApiError` - API error (check message for retryability)
    /// - `ClaudeError::Timeout` - Request timed out (retryable)
    async fn execute(&self, request: ClaudeRequest) -> Result<ClaudeResponse, ClaudeError>;

    /// Health check for Claude API connectivity
    ///
    /// # Returns
    /// * `Ok(())` - API is reachable and healthy
    /// * `Err(ClaudeError)` - API is unreachable or unhealthy
    async fn health_check(&self) -> Result<(), ClaudeError>;
}

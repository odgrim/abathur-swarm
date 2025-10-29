use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request to Claude API for task execution (simplified)
///
/// This is a high-level request type for task execution.
/// For more detailed API access, use `MessageRequest`.
#[derive(Debug, Clone)]
pub struct ClaudeRequest {
    pub task_id: Uuid,
    pub agent_type: String,
    pub prompt: String,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

/// Response from Claude API (simplified)
///
/// This is a high-level response type for task execution.
/// For more detailed API access, use `MessageResponse`.
#[derive(Debug, Clone)]
pub struct ClaudeResponse {
    pub task_id: Uuid,
    pub content: String,
    pub stop_reason: String,
    pub usage: TokenUsage,
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Detailed message request matching Claude's Messages API
///
/// This structure corresponds to Claude's `/v1/messages` endpoint.
/// Use this for fine-grained control over API interactions.
///
/// # Examples
/// ```ignore
/// let request = MessageRequest {
///     model: "claude-sonnet-4-5-20250929".to_string(),
///     messages: vec![
///         Message {
///             role: "user".to_string(),
///             content: "Hello, Claude!".to_string(),
///         }
///     ],
///     max_tokens: 1024,
///     temperature: Some(0.7),
///     system: Some("You are a helpful assistant.".to_string()),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRequest {
    /// The model to use (e.g., "claude-sonnet-4-5-20250929")
    pub model: String,

    /// The conversational messages (user/assistant turns)
    pub messages: Vec<Message>,

    /// Maximum tokens to generate in the response
    pub max_tokens: usize,

    /// Sampling temperature (0.0 to 1.0). Optional, defaults to 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// System prompt to guide the assistant's behavior. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

/// A single message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "user" or "assistant"
    pub role: String,

    /// The message content text
    pub content: String,
}

/// Detailed response from Claude's Messages API
///
/// Contains the full response including content blocks, stop reason, and usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Unique identifier for this message
    pub id: String,

    /// Content blocks in the response
    pub content: Vec<ContentBlock>,

    /// Reason the model stopped generating (e.g., `end_turn`, `max_tokens`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,

    /// Token usage statistics
    pub usage: Usage,
}

/// A content block in the response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    /// Type of content block (e.g., "text")
    #[serde(rename = "type")]
    pub content_type: String,

    /// Text content (present when `content_type` is "text")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Token usage information for the request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the input
    pub input_tokens: usize,

    /// Number of tokens in the output
    pub output_tokens: usize,
}

/// Streaming chunk from Claude's Messages API
///
/// When streaming is enabled, the response is delivered as a series of chunks.
/// Each chunk contains a delta (incremental content) and optional stop reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageChunk {
    /// Incremental text content (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,

    /// Stop reason (present in final chunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Error types specific to Claude API
///
/// These errors cover common failure modes when interacting with Claude's API.
/// Implementations should map API errors to these types for consistent handling.
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
/// This trait defines the interface for interacting with Claude's Messages API
/// following hexagonal architecture principles. The domain layer depends on
/// this abstraction, while the infrastructure layer provides concrete implementations
/// (HTTP adapter, mock adapter for testing, etc.).
///
/// # Design Rationale
///
/// This port provides two levels of abstraction:
/// 1. **High-level API** (`execute`, `health_check`): Simplified interface for common task execution
/// 2. **Low-level API** (`send_message`, `stream_message`): Direct access to Claude's Messages API
///
/// The high-level API is useful for simple task execution where you just need to
/// send a prompt and get a response. The low-level API gives fine-grained control
/// over the request structure, enabling advanced use cases like multi-turn conversations,
/// system prompts, and streaming responses.
///
/// # Architectural Role
///
/// - **Domain Layer**: Depends on this trait for LLM interactions
/// - **Infrastructure Layer**: Implements this trait with HTTP client, rate limiting, retries
/// - **Testing**: Implements mock/fake versions for unit tests
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` for safe use across async tasks.
/// The trait uses `&self` (shared reference) to enable concurrent usage.
///
/// # Error Handling
///
/// All methods return `Result<T, ClaudeError>` to handle API failures gracefully.
/// Callers should handle rate limiting, transient errors, and retry logic as needed.
///
/// # Examples
///
/// ```ignore
/// // High-level API (simplified)
/// let response = client.execute(ClaudeRequest {
///     task_id: task_id,
///     agent_type: "analyst".to_string(),
///     prompt: "Analyze this code...".to_string(),
///     max_tokens: Some(2048),
///     temperature: Some(0.7),
/// }).await?;
///
/// // Low-level API (detailed)
/// let response = client.send_message(MessageRequest {
///     model: "claude-sonnet-4-5-20250929".to_string(),
///     messages: vec![
///         Message {
///             role: "user".to_string(),
///             content: "Hello!".to_string(),
///         }
///     ],
///     max_tokens: 1024,
///     temperature: Some(0.7),
///     system: Some("You are a helpful assistant.".to_string()),
/// }).await?;
///
/// // Streaming API
/// let mut stream = client.stream_message(request).await?;
/// while let Some(chunk) = stream.next().await {
///     let chunk = chunk?;
///     if let Some(delta) = chunk.delta {
///         print!("{}", delta);
///     }
/// }
/// ```
#[async_trait]
pub trait ClaudeClient: Send + Sync {
    /// Execute a prompt via Claude API (high-level interface)
    ///
    /// This is a simplified interface for executing task prompts.
    /// It handles the conversion from `ClaudeRequest` to the underlying API format.
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

    /// Send a message to Claude's Messages API (low-level interface)
    ///
    /// This method provides direct access to Claude's Messages API with full control
    /// over request parameters. Use this for advanced use cases like multi-turn
    /// conversations, custom system prompts, or precise parameter tuning.
    ///
    /// # Arguments
    /// * `request` - Detailed message request matching Claude's API format
    ///
    /// # Returns
    /// * `Ok(MessageResponse)` - Successful response with content and metadata
    /// * `Err(ClaudeError)` - API error, rate limit, or network error
    ///
    /// # Errors
    /// Same error variants as `execute()`, plus:
    /// - `ClaudeError::ApiError` - Invalid request format or parameters
    ///
    /// # Examples
    /// ```ignore
    /// let response = client.send_message(MessageRequest {
    ///     model: "claude-sonnet-4-5-20250929".to_string(),
    ///     messages: vec![Message {
    ///         role: "user".to_string(),
    ///         content: "Explain async Rust".to_string(),
    ///     }],
    ///     max_tokens: 2048,
    ///     temperature: Some(0.7),
    ///     system: Some("You are a Rust expert.".to_string()),
    /// }).await?;
    /// ```
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse, ClaudeError>;

    /// Stream a message response from Claude's Messages API
    ///
    /// This method enables streaming responses where content is delivered incrementally
    /// as it's generated. This is useful for long-form content, real-time UX, or
    /// early cancellation based on partial results.
    ///
    /// # Arguments
    /// * `request` - Detailed message request (same format as `send_message`)
    ///
    /// # Returns
    /// * `Ok(Stream)` - Async stream of message chunks
    /// * `Err(ClaudeError)` - Error initiating the stream
    ///
    /// # Stream Items
    /// Each stream item is `Result<MessageChunk, ClaudeError>`:
    /// - `Ok(MessageChunk)` - Incremental content delta
    /// - `Err(ClaudeError)` - Error during streaming
    ///
    /// The final chunk will contain `stop_reason` indicating why generation stopped.
    ///
    /// # Errors
    /// Same error variants as `send_message()`, plus:
    /// - `ClaudeError::NetworkError` - Connection interrupted during streaming
    ///
    /// # Examples
    /// ```ignore
    /// let mut stream = client.stream_message(request).await?;
    ///
    /// while let Some(result) = stream.next().await {
    ///     match result {
    ///         Ok(chunk) => {
    ///             if let Some(delta) = chunk.delta {
    ///                 print!("{}", delta); // Print incremental content
    ///             }
    ///             if let Some(reason) = chunk.stop_reason {
    ///                 println!("\nStopped: {}", reason);
    ///                 break;
    ///             }
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Stream error: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// ```
    async fn stream_message(
        &self,
        request: MessageRequest,
    ) -> Result<Box<dyn Stream<Item = Result<MessageChunk, ClaudeError>> + Send + Unpin>, ClaudeError>;

    /// Health check for Claude API connectivity
    ///
    /// Verifies that the API is reachable and the client is properly configured.
    /// This is useful for startup checks and monitoring.
    ///
    /// # Returns
    /// * `Ok(())` - API is reachable and healthy
    /// * `Err(ClaudeError)` - API is unreachable or unhealthy
    ///
    /// # Errors
    /// - `ClaudeError::InvalidApiKey` - API key is invalid or missing
    /// - `ClaudeError::NetworkError` - Cannot reach API endpoint
    /// - `ClaudeError::Timeout` - Health check timed out
    async fn health_check(&self) -> Result<(), ClaudeError>;
}

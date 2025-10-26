use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// Result type for Claude client operations
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Request to Claude API for message completion
///
/// This structure represents a complete request to the Claude Messages API.
/// It includes the model selection, conversation history, token limits,
/// and optional parameters for controlling response generation.
///
/// # Example
/// ```
/// use abathur::domain::ports::{MessageRequest, Message};
///
/// let request = MessageRequest {
///     model: "claude-3-5-sonnet-20241022".to_string(),
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
    /// Model identifier (e.g., "claude-3-5-sonnet-20241022")
    pub model: String,

    /// Conversation history (alternating user/assistant messages)
    pub messages: Vec<Message>,

    /// Maximum tokens to generate in the response
    pub max_tokens: usize,

    /// Sampling temperature (0.0 to 1.0). Higher values produce more random output.
    /// Default: 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// System prompt that sets the context for the conversation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

/// A single message in the conversation
///
/// Messages have a role (user or assistant) and content.
/// The Messages API expects an alternating sequence of user and assistant messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message author ("user" or "assistant")
    pub role: String,

    /// Content of the message
    pub content: String,
}

/// Response from Claude API message completion
///
/// Contains the generated response, usage statistics, and metadata
/// about how the response was generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Unique identifier for this message
    pub id: String,

    /// Content blocks in the response (typically text, but can include other types)
    pub content: Vec<ContentBlock>,

    /// Reason why generation stopped (e.g., "end_turn", "max_tokens", "stop_sequence")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,

    /// Token usage statistics for this request
    pub usage: Usage,
}

/// A block of content in the response
///
/// Claude can return different types of content blocks. The most common
/// is text, but future versions may support other types like images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    /// Type of content block (e.g., "text")
    #[serde(rename = "type")]
    pub content_type: String,

    /// Text content (present when content_type is "text")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Token usage statistics for a request/response
///
/// Tracks both input tokens (from the request) and output tokens
/// (generated in the response) for billing and rate limiting purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the input (request)
    pub input_tokens: usize,

    /// Number of tokens in the output (response)
    pub output_tokens: usize,
}

/// Chunk of a streaming response
///
/// When using streaming, the response is delivered incrementally
/// as a series of chunks. Each chunk contains a delta (partial text)
/// and optionally a stop reason when the stream completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageChunk {
    /// Incremental text content (partial response)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,

    /// Stop reason (present in the final chunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Port trait for Claude API client
///
/// This trait defines the interface for interacting with Anthropic's Claude API.
/// It abstracts the HTTP client implementation, enabling:
/// - Dependency injection for testing (mock implementations)
/// - Different HTTP client backends (reqwest, hyper, etc.)
/// - Rate limiting and retry logic in adapters
/// - Request/response transformation
///
/// # Hexagonal Architecture
///
/// This is a **port** in hexagonal architecture terminology. The domain layer
/// depends on this trait, not on concrete HTTP client implementations.
/// **Adapters** (in the infrastructure layer) implement this trait using
/// specific technologies (e.g., reqwest for HTTP calls).
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` for use in async contexts with
/// multi-threaded runtimes like Tokio. The trait methods take `&self` to
/// allow concurrent requests without mutable state.
///
/// # Design Rationale
///
/// - **send_message**: Synchronous (non-streaming) API for single responses
/// - **stream_message**: Streaming API for real-time token-by-token responses
/// - Both methods use the same request type for consistency
/// - Error handling is unified via `Result<T>` (Box<dyn Error>)
/// - Serde-compatible types for easy serialization/deserialization
///
/// # Error Handling
///
/// Methods return `Result<T>` where the error is a trait object. This allows
/// adapters to define specific error types (e.g., network errors, rate limits,
/// authentication failures) while maintaining a common interface.
///
/// Common error scenarios:
/// - Rate limit exceeded (429 status)
/// - Invalid API key (401 status)
/// - Network failures (connection timeout, DNS errors)
/// - Invalid request format (400 status)
/// - Server errors (5xx status)
///
/// Adapters should implement retry logic for transient errors (rate limits,
/// network failures, 5xx errors) and fail fast for non-retryable errors
/// (invalid API key, malformed requests).
///
/// # Example Implementation
///
/// ```ignore
/// use async_trait::async_trait;
/// use futures::Stream;
///
/// struct ReqwestClaudeClient {
///     api_key: String,
///     base_url: String,
///     http_client: reqwest::Client,
/// }
///
/// #[async_trait]
/// impl ClaudeClient for ReqwestClaudeClient {
///     async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse> {
///         let response = self.http_client
///             .post(&format!("{}/v1/messages", self.base_url))
///             .header("x-api-key", &self.api_key)
///             .header("anthropic-version", "2023-06-01")
///             .json(&request)
///             .send()
///             .await?;
///
///         let message_response = response.json::<MessageResponse>().await?;
///         Ok(message_response)
///     }
///
///     async fn stream_message(&self, request: MessageRequest) -> Result<Box<dyn Stream<Item = Result<MessageChunk>> + Send + Unpin>> {
///         // Implementation would use SSE (Server-Sent Events) to stream chunks
///         todo!("Streaming implementation")
///     }
/// }
/// ```
///
/// # Usage in Domain Layer
///
/// ```ignore
/// use std::sync::Arc;
///
/// async fn execute_task(claude: Arc<dyn ClaudeClient>, prompt: String) -> Result<String> {
///     let request = MessageRequest {
///         model: "claude-3-5-sonnet-20241022".to_string(),
///         messages: vec![Message {
///             role: "user".to_string(),
///             content: prompt,
///         }],
///         max_tokens: 4096,
///         temperature: Some(0.7),
///         system: Some("You are an AI assistant.".to_string()),
///     };
///
///     let response = claude.send_message(request).await?;
///
///     // Extract text from content blocks
///     let text = response.content
///         .iter()
///         .filter_map(|block| block.text.clone())
///         .collect::<Vec<_>>()
///         .join("\n");
///
///     Ok(text)
/// }
/// ```
#[async_trait]
pub trait ClaudeClient: Send + Sync {
    /// Send a message to Claude and receive a complete response
    ///
    /// This method sends a request to Claude's Messages API and waits for
    /// the complete response. Use this for non-streaming requests where
    /// you need the full response before processing.
    ///
    /// # Arguments
    ///
    /// * `request` - The message request containing model, messages, and parameters
    ///
    /// # Returns
    ///
    /// * `Ok(MessageResponse)` - Successful response with generated content
    /// * `Err(Box<dyn Error>)` - Request failed (network, rate limit, auth, etc.)
    ///
    /// # Errors
    ///
    /// This method can fail for several reasons:
    /// - Network errors (connection timeout, DNS failure, etc.)
    /// - Authentication errors (invalid API key)
    /// - Rate limiting (too many requests)
    /// - Invalid request (malformed parameters, unsupported model, etc.)
    /// - Server errors (Claude API internal errors)
    ///
    /// Implementations should include detailed error messages to help diagnose issues.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let request = MessageRequest {
    ///     model: "claude-3-5-sonnet-20241022".to_string(),
    ///     messages: vec![Message {
    ///         role: "user".to_string(),
    ///         content: "What is 2+2?".to_string(),
    ///     }],
    ///     max_tokens: 100,
    ///     temperature: None,
    ///     system: None,
    /// };
    ///
    /// let response = client.send_message(request).await?;
    /// println!("Claude: {}", response.content[0].text.as_ref().unwrap());
    /// ```
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse>;

    /// Stream a message response from Claude token-by-token
    ///
    /// This method sends a request to Claude's streaming API and returns
    /// a stream of chunks that arrive incrementally. Use this for real-time
    /// response generation where you want to display tokens as they're generated.
    ///
    /// # Arguments
    ///
    /// * `request` - The message request containing model, messages, and parameters
    ///
    /// # Returns
    ///
    /// * `Ok(Stream)` - Stream of message chunks arriving incrementally
    /// * `Err(Box<dyn Error>)` - Request failed before streaming started
    ///
    /// # Errors
    ///
    /// This method can fail for the same reasons as `send_message`. Additionally:
    /// - Stream interruptions (connection closed mid-response)
    /// - SSE parsing errors (malformed event stream)
    ///
    /// Note that errors can occur both when initiating the stream (returned
    /// as `Err`) and during streaming (yielded as `Item = Result<MessageChunk>`).
    ///
    /// # Example
    ///
    /// ```ignore
    /// use futures::StreamExt;
    ///
    /// let request = MessageRequest {
    ///     model: "claude-3-5-sonnet-20241022".to_string(),
    ///     messages: vec![Message {
    ///         role: "user".to_string(),
    ///         content: "Write a poem".to_string(),
    ///     }],
    ///     max_tokens: 1024,
    ///     temperature: Some(0.9),
    ///     system: None,
    /// };
    ///
    /// let mut stream = client.stream_message(request).await?;
    /// while let Some(chunk) = stream.next().await {
    ///     match chunk {
    ///         Ok(chunk) => {
    ///             if let Some(delta) = chunk.delta {
    ///                 print!("{}", delta);
    ///             }
    ///             if chunk.stop_reason.is_some() {
    ///                 break;
    ///             }
    ///         }
    ///         Err(e) => eprintln!("Stream error: {}", e),
    ///     }
    /// }
    /// ```
    async fn stream_message(
        &self,
        request: MessageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<MessageChunk>> + Send>>>;

    /// Check if the Claude API is reachable and the API key is valid
    ///
    /// # Returns
    /// * `Ok(true)` - API is healthy and accessible
    /// * `Ok(false)` - API is unreachable or API key is invalid
    /// * `Err` - Unexpected error during health check
    ///
    /// # Implementation Note
    /// This typically sends a minimal test request to verify connectivity.
    async fn health_check(&self) -> Result<bool>;
}

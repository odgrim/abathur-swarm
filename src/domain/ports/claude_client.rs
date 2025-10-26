/// Claude API client trait (port interface)
use async_trait::async_trait;
use anyhow::Result;

// Import types from infrastructure layer
// Note: In real Clean Architecture, domain should not depend on infrastructure
// These types would ideally be in domain/models, but for simplicity we use infrastructure types
use crate::infrastructure::claude::types::{MessageRequest, MessageResponse};

/// Port interface for Claude API client
///
/// This trait defines the contract for interacting with the Claude API.
/// Implementations handle HTTP communication, rate limiting, and retry logic.
#[async_trait]
pub trait ClaudeClient: Send + Sync {
    /// Send a message to Claude and receive a response
    ///
    /// # Arguments
    /// * `request` - The message request containing model, messages, and options
    ///
    /// # Returns
    /// * `Ok(MessageResponse)` - Successful response from Claude
    /// * `Err(anyhow::Error)` - Error occurred during request (network, API error, etc.)
    ///
    /// # Errors
    /// This function will return an error if:
    /// - The API key is invalid (401)
    /// - The request is malformed (400)
    /// - Rate limits are exceeded (429)
    /// - Server errors occur (500, 502, 503, 504, 529)
    /// - Network errors occur (timeout, connection failure)
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse>;

    /// Check if the Claude API is reachable and the API key is valid
    ///
    /// # Returns
    /// * `Ok(true)` - API is healthy and accessible
    /// * `Ok(false)` - API is unreachable or API key is invalid
    /// * `Err(anyhow::Error)` - Unexpected error during health check
    ///
    /// # Implementation Note
    /// This typically sends a minimal test request to verify connectivity.
    async fn health_check(&self) -> Result<bool>;
}

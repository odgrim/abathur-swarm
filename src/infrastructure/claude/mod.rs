/// Claude API client infrastructure module
///
/// This module provides the production implementation of the ClaudeClient trait,
/// including HTTP communication, rate limiting, and retry logic.

pub mod client;
pub mod error;
pub mod rate_limiter;
pub mod retry;
pub mod types;

// Re-export main types for convenience
pub use client::{ClaudeClientConfig, ClaudeClientImpl};
pub use error::ClaudeApiError;
pub use rate_limiter::TokenBucketRateLimiter;
pub use retry::RetryPolicy;
pub use types::{
    ContentBlock, ImageSource, Message, MessageContent, MessageRequest, MessageResponse,
    StopReason, Tool, Usage,
};

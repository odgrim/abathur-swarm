//! Claude API client infrastructure
//!
//! HTTP client for Claude API with:
//! - Request/response handling
//! - Rate limiting (token bucket)
//! - Retry logic with exponential backoff
//! - Streaming support

pub mod adapter;
pub mod client;
pub mod error;
pub mod errors;
pub mod rate_limiter;
pub mod retry;
pub mod streaming;
pub mod types;

pub use adapter::ClaudeClientAdapter;
pub use client::{ClaudeClient, ClaudeClientConfig};
pub use error::ClaudeApiError;
pub use errors::ClaudeApiError as ClaudeError;
pub use rate_limiter::TokenBucketRateLimiter;
pub use retry::RetryPolicy;
pub use types::{MessageRequest, MessageResponse};

//! Claude API client infrastructure
//!
//! HTTP client for Claude API with:
//! - Request/response handling
//! - Rate limiting (token bucket)
//! - Retry logic with exponential backoff
//! - Streaming support

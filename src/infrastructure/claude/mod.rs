//! Claude API infrastructure components
//!
//! This module contains implementations for interacting with Anthropic's Claude API,
//! including HTTP client, rate limiting, retry logic, and streaming support.

pub mod streaming;

pub use streaming::{parse_sse_event, SseEventStream, StreamEvent};

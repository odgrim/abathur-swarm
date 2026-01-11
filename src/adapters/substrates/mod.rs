//! Substrate adapter implementations.

pub mod anthropic_api;
pub mod claude_code;
pub mod mock;
pub mod registry;

pub use anthropic_api::{AnthropicApiConfig, AnthropicApiSubstrate};
pub use claude_code::ClaudeCodeSubstrate;
pub use mock::MockSubstrate;
pub use registry::SubstrateRegistry;

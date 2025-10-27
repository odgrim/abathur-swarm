///! LLM Substrate Implementations
///!
///! This module contains concrete implementations of the LlmSubstrate trait
///! for different LLM backends.

pub mod anthropic_api;
pub mod claude_code;
pub mod registry;

pub use anthropic_api::AnthropicApiSubstrate;
pub use claude_code::ClaudeCodeSubstrate;
pub use registry::SubstrateRegistry;

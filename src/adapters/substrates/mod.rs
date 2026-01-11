//! Substrate adapter implementations.

pub mod claude_code;
pub mod mock;
pub mod registry;

pub use claude_code::ClaudeCodeSubstrate;
pub use mock::MockSubstrate;
pub use registry::SubstrateRegistry;

//! MCP (Model Context Protocol) infrastructure module
//!
//! Provides MCP client implementations:
//! - `DirectMcpClient` - For internal agents (in-process, efficient)
//! - `MockMcpClient` - For testing
//! - Stdio clients - For external clients (Claude Code, IDEs)
//!
//! # Design Philosophy
//!
//! **For Internal Agents (hundreds):**
//! Use `DirectMcpClient` which calls services directly without spawning processes.
//! This avoids resource exhaustion and provides efficient shared access.
//!
//! **For External Clients (few):**
//! Use stdio MCP servers that external tools spawn per `.mcp.json` config.
//! Each external client gets its own isolated server instance.

pub mod direct_client;
pub mod error;
pub mod mock_client;

pub use direct_client::DirectMcpClient;
pub use error::{McpError, Result};
pub use mock_client::MockMcpClient;

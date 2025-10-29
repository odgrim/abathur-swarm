//! MCP (Model Context Protocol) infrastructure module
//!
//! Provides MCP client implementations and HTTP servers:
//! - `MockMcpClient` - For testing
//! - HTTP servers for memory and task queue management
//!
//! # Design Philosophy
//!
//! MCP server access is provided via HTTP servers for external clients.
//! External LLM instances (Claude Code, Anthropic API) connect to HTTP MCP servers
//! on ports 45678 (memory) and 45679 (tasks).

pub mod error;
pub mod handlers;
pub mod http_server;
pub mod mock_client;
pub mod types;

pub use error::{McpError, Result};
pub use http_server::{start_memory_server, start_tasks_server};
pub use mock_client::MockMcpClient;

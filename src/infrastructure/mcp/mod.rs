//! MCP (Model Context Protocol) infrastructure module
//!
//! Provides integration with MCP servers via stdio transport, including:
//! - Server lifecycle management
//! - Health monitoring with auto-restart
//! - JSON-RPC communication
//! - Error handling
//!
//! # Components
//!
//! - `client` - High-level MCP client implementing the McpClient trait
//! - `server_manager` - Server process lifecycle management
//! - `health_monitor` - Background health checking and auto-restart
//! - `error` - MCP-specific error types

pub mod client;
pub mod error;
pub mod health_monitor;
pub mod server_manager;

pub use client::McpClientImpl;
pub use error::{McpError, Result};
pub use health_monitor::HealthMonitor;
pub use server_manager::{McpServerManager, StdioTransport};

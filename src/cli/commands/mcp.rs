//! MCP server command handlers
//!
//! Thin adapters for running HTTP MCP servers for memory and task queue management.
//! Delegates to infrastructure layer for actual implementation.

use anyhow::Result;
use crate::infrastructure::mcp::{start_memory_server, start_tasks_server};

/// Handle memory HTTP MCP server
pub async fn handle_memory_http(db_path: String, port: u16) -> Result<()> {
    start_memory_server(db_path, port).await
}

/// Handle tasks HTTP MCP server
pub async fn handle_tasks_http(db_path: String, port: u16) -> Result<()> {
    start_tasks_server(db_path, port).await
}

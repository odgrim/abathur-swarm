//! Abathur - AI Agent Orchestration Framework
//!
//! A Rust rewrite of the Abathur AI agent orchestration system with:
//! - Task queue with priority and dependency management
//! - Concurrent agent swarm execution
//! - SQLite database with WAL mode
//! - MCP (Model Context Protocol) integration
//! - Claude API client
//! - Memory management (semantic, episodic, procedural)

pub mod infrastructure;

// Re-export key types for convenience
pub use infrastructure::database::DatabaseConnection;

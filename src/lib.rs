//! Abathur - AI Agent Orchestration System
//!
//! A Rust-based system for orchestrating AI agent swarms with:
//! - Task queue with priorities and dependencies
//! - Concurrent agent execution
//! - MCP (Model Context Protocol) integration
//! - Claude API client
//! - SQLite persistence
//! - Structured logging and audit trails

pub mod infrastructure;

// Re-export commonly used types
pub use infrastructure::logging::{AuditEvent, AuditEventType, AuditLogger, AuditOutcome, LogRotator};

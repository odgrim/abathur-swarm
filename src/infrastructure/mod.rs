//! Infrastructure layer module
//!
//! This module contains all infrastructure adapters and external integrations:
//! - Database implementations (SQLite with sqlx)
//! - Claude API client
//! - MCP integration
//! - Configuration management
//! - Logging infrastructure
//! - Process management
//! - Credentials management
//!
//! Infrastructure implementations satisfy the port traits defined in the domain layer.

pub mod claude;
pub mod config;
pub mod database;
pub mod mcp;
pub mod substrates;

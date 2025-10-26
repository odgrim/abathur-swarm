//! Abathur - AI Agent Orchestration System
//!
//! A Rust rewrite of the Python-based Abathur system for orchestrating
//! AI agents using Claude and the Model Context Protocol (MCP).
//!
//! # Architecture
//!
//! Following Clean Architecture principles:
//! - `domain`: Core business logic and entities
//! - `application`: Use cases and orchestration
//! - `infrastructure`: External dependencies (logging, DB, HTTP, MCP)
//! - `cli`: Command-line interface

pub mod infrastructure;

// Re-export commonly used items
pub use infrastructure::logging;

//! Port trait definitions (Hexagonal Architecture)
//!
//! This module defines async trait interfaces that infrastructure adapters must implement:
//! - TaskRepository: Database operations for tasks
//! - ClaudeClient: Claude API operations
//! - McpClient: MCP server operations
//! - Logger: Structured logging operations
//!
//! These traits define the contracts that allow the domain to be independent
//! of specific infrastructure implementations.

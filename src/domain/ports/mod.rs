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

pub mod agent_repository;
pub mod claude_client;
pub mod errors;
pub mod logger;
pub mod mcp_client;
pub mod memory_repository;
pub mod priority_calculator;
pub mod session_repository;
pub mod task_queue_service;
pub mod task_repository;

pub use agent_repository::AgentRepository;
pub use claude_client::{
    ClaudeClient, ContentBlock, Message, MessageChunk, MessageRequest, MessageResponse, Usage,
};
pub use errors::DatabaseError;
pub use logger::{Level, Logger};
pub use mcp_client::{McpClient, Resource, Tool};
pub use memory_repository::MemoryRepository;
pub use priority_calculator::PriorityCalculator;
pub use session_repository::SessionRepository;
pub use task_queue_service::TaskQueueService;
pub use task_repository::{TaskFilters, TaskRepository};

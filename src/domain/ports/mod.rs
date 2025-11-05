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
pub mod chain_repository;
pub mod claude_client;
pub mod embedding_repository;
pub mod llm_substrate;
pub mod logger;
pub mod mcp_client;
pub mod memory_repository;
pub mod priority_calculator;
pub mod session_repository;
pub mod task_queue_service;
pub mod task_repository;

pub use agent_repository::AgentRepository;
pub use chain_repository::{ChainRepository, ChainStats};
pub use claude_client::{
    ClaudeClient, ClaudeError, ClaudeRequest, ClaudeResponse, ContentBlock, Message, MessageChunk,
    MessageRequest, MessageResponse, TokenUsage, Usage,
};
pub use embedding_repository::{
    ChunkingService, EmbeddingRepository, EmbeddingService,
};
pub use llm_substrate::{
    ExecutionParameters, HealthStatus, LlmSubstrate, StopReason, SubstrateError, SubstrateRequest,
    SubstrateResponse, TokenUsage as SubstrateTokenUsage,
};
pub use logger::{Level, Logger};
pub use mcp_client::{
    McpClient, McpError, McpToolRequest, McpToolResponse, ResourceContent, ResourceInfo, ToolInfo,
};
pub use memory_repository::MemoryRepository;
pub use priority_calculator::PriorityCalculator;
pub use session_repository::SessionRepository;
pub use task_queue_service::TaskQueueService;
pub use task_repository::{TaskFilters, TaskRepository};

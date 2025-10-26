//! Abathur - Agentic Swarm Orchestrator
//!
//! Abathur is a task queue and swarm orchestration system for managing AI agents
//! with hierarchical memory, MCP integration, and priority-based task scheduling.
//!
//! # Architecture
//!
//! This crate follows Clean Architecture / Hexagonal Architecture principles:
//!
//! - **Domain Layer** (`domain`): Pure business logic and domain models
//! - **Application Layer** (`application`): Use case orchestration and workflows
//! - **Service Layer** (`services`): Business logic coordination
//! - **Infrastructure Layer** (`infrastructure`): External integrations and adapters
//! - **CLI Layer** (`cli`): Command-line interface
//!
//! # Example
//!
//! ```ignore
//! use abathur::application::Orchestrator;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Initialize and run orchestrator
//!     Ok(())
//! }
//! ```

pub mod application;
pub mod cli;
pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use application::{
    ConvergenceStrategy, LoopExecutor, LoopState, ResourceEvent, ResourceLimits, ResourceMonitor,
    ResourceStatus, TaskCoordinator, TaskStatusUpdate,
};
pub use cli::output::progress::{
    MultiProgressManager, ProgressBarExt, create_progress_bar, create_spinner,
};
pub use domain::models::{
    Agent, AgentStatus, Config, DatabaseConfig, LoggingConfig, McpServerConfig, Memory, MemoryType,
    RateLimitConfig, ResourceLimitsConfig, RetryConfig,
};
pub use domain::ports::{AgentRepository, MemoryRepository, PriorityCalculator, TaskQueueService};
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use infrastructure::database::{AgentRepositoryImpl, DatabaseConnection, DatabaseError};
pub use services::{DependencyResolver, MemoryService};

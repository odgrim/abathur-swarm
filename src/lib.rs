pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use application::{ConvergenceStrategy, LoopExecutor, LoopState, TaskCoordinator, TaskStatusUpdate};
pub use domain::models::{
    Agent, AgentStatus, Config, DatabaseConfig, LoggingConfig, McpServerConfig, Memory, MemoryType,
    RateLimitConfig, ResourceLimitsConfig, RetryConfig,
};
pub use domain::ports::{AgentRepository, ClaudeClient, MemoryRepository, PriorityCalculator, TaskQueueService};
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use infrastructure::database::errors::DatabaseError;
pub use services::{DependencyResolver, MemoryService};

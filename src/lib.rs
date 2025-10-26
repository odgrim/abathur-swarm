pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use application::{ConvergenceStrategy, LoopExecutor, LoopState, TaskCoordinator, TaskStatusUpdate};
pub use domain::models::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, Memory, MemoryType,
    RateLimitConfig, ResourceLimitsConfig, RetryConfig,
};
pub use domain::ports::{MemoryRepository, PriorityCalculator, TaskQueueService};
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use services::{DependencyResolver, MemoryService};

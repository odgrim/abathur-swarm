pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use domain::models::{
    Config, DatabaseConfig, DependencyType, LoggingConfig, McpServerConfig, RateLimitConfig,
    ResourceLimitsConfig, RetryConfig, Task, TaskSource, TaskStatus,
};
pub use domain::ports::{DatabaseError, TaskFilters, TaskRepository};
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use services::{DependencyResolver, PriorityCalculator, TaskQueueService};

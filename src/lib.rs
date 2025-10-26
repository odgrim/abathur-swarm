pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use application::{ConvergenceStrategy, LoopExecutor, LoopState};
pub use domain::models::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, Memory, MemoryType, RateLimitConfig,
    ResourceLimitsConfig, RetryConfig,
};
pub use domain::ports::MemoryRepository;
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use services::MemoryService;

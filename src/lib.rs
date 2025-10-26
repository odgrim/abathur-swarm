pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use domain::models::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, RateLimitConfig, ResourceLimitsConfig,
    RetryConfig, Memory, MemoryType,
};
pub use domain::ports::MemoryRepository;
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use services::MemoryService;

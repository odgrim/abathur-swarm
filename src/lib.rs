pub mod domain;
pub mod infrastructure;

// Re-export commonly used types for convenience
pub use domain::models::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, RateLimitConfig, ResourceLimitsConfig,
    RetryConfig,
};
pub use infrastructure::config::{ConfigError, ConfigLoader};

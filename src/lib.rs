pub mod application;
pub mod cli;
pub mod domain;
pub mod infrastructure;
pub mod services;

// Re-export commonly used types for convenience
pub use application::{
    ConvergenceStrategy, LoopExecutor, LoopState, ResourceEvent, ResourceLimits, ResourceMonitor,
    ResourceStatus,
};
pub use cli::output::progress::{
    MultiProgressManager, ProgressBarExt, create_progress_bar, create_spinner,
};
pub use domain::models::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, Memory, MemoryType, RateLimitConfig,
    ResourceLimitsConfig, RetryConfig,
};
pub use domain::ports::MemoryRepository;
pub use infrastructure::config::{ConfigError, ConfigLoader};
pub use services::MemoryService;

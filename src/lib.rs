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

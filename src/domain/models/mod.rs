<<<<<<< HEAD
<<<<<<< HEAD
//! Domain models
//!
//! Pure domain entities with business logic and validation rules.
//! These models are framework-agnostic and contain no infrastructure concerns.

pub mod agent;
pub mod config;
pub mod memory;
pub mod queue;
pub mod session;
pub mod task;

pub use agent::{Agent, AgentStatus};
pub use config::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, RateLimitConfig, ResourceLimitsConfig,
    RetryConfig,
};
pub use memory::{Memory, MemoryType};
pub use queue::{QueueItem, TaskQueue};
pub use session::{Session, SessionEvent};
=======
pub mod task;

>>>>>>> task_phase3-task-repository_2025-10-25-23-00-02
pub use task::{DependencyType, Task, TaskSource, TaskStatus};
=======
pub mod memory;

pub use memory::{Memory, MemoryType};
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04

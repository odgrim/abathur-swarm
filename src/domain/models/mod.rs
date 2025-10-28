//! Domain models
//!
//! Pure domain entities with business logic and validation rules.
//! These models are framework-agnostic and contain no infrastructure concerns.

pub mod agent;
pub mod agent_contract;
pub mod config;
pub mod memory;
pub mod queue;
pub mod session;
pub mod task;

pub use agent::{Agent, AgentStatus};
pub use agent_contract::AgentContractRegistry;
pub use config::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, RateLimitConfig,
    RetryConfig,
};
pub use memory::{Memory, MemoryType};
pub use queue::{Queue, QueueError};
pub use session::{Session, SessionEvent};
pub use task::{
    DependencyType, Task, TaskSource, TaskStatus, ValidationRequirement, WorkflowExpectations,
    WorkflowState,
};

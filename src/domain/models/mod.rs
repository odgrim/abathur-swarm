//! Domain models
//!
//! Pure domain entities with business logic and validation rules.
//! These models are framework-agnostic and contain no infrastructure concerns.

pub mod agent;
pub mod agent_contract;
pub mod agent_metadata;
pub mod config;
pub mod hook;
pub mod memory;
pub mod prune;
pub mod queue;
pub mod session;
pub mod task;

pub use agent::{Agent, AgentStatus};
pub use agent_contract::AgentContractRegistry;
pub use agent_metadata::{AgentMetadata, AgentMetadataRegistry};
pub use config::{
    Config, DatabaseConfig, LoggingConfig, McpServerConfig, RateLimitConfig,
    RetryConfig,
};
pub use hook::{
    BranchCompletionContext, BranchType, HookAction, HookCondition, HookContext, HookEvent,
    HookResult, HooksConfig, MergeStrategy, TaskHook,
};
pub use memory::{Memory, MemoryType};
pub use prune::{BlockedTask, PruneResult};
pub use queue::{Queue, QueueError};
pub use session::{Session, SessionEvent};
pub use task::{
    DependencyType, Task, TaskSource, TaskStatus, ValidationRequirement, WorkflowExpectations,
    WorkflowState,
};

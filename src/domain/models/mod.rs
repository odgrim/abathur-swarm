//! Domain models
//!
//! Pure domain entities with business logic and validation rules.
//! These models are framework-agnostic and contain no infrastructure concerns.

pub mod agent;
pub mod agent_config;
pub mod agent_contract;
pub mod agent_metadata;
pub mod chunking;
pub mod config;
pub mod embedding;
pub mod hook;
pub mod memory;
pub mod prompt_chain;
pub mod prune;
pub mod queue;
pub mod session;
pub mod task;

pub use agent::{Agent, AgentStatus};
pub use agent_config::{AgentConfiguration, AgentContract, ValidationType as AgentValidationType};
pub use agent_contract::AgentContractRegistry;
pub use agent_metadata::{AgentMetadata, AgentMetadataRegistry};
pub use chunking::{
    Chunk, ChunkMetadata, ChunkingConfig, ChunkingResult, OverlapStrategy,
};
pub use config::{
    ChunkingConfigSettings, Config, DatabaseConfig, EmbeddingConfig, LoggingConfig,
    McpServerConfig, OpenAIEmbeddingConfig, RagConfig, RateLimitConfig, RecoveryConfig,
    RetryConfig, VectorSearchConfig,
};
pub use embedding::{
    Citation, EmbeddingModel, SearchResult, VectorMemory,
};
pub use hook::{
    BranchCompletionContext, BranchType, HookAction, HookCondition, HookContext, HookEvent,
    HookResult, HooksConfig, MergeStrategy, TaskHook,
};
pub use memory::{Memory, MemoryType};
pub use prompt_chain::{
    ChainExecution, ChainStatus, OutputFormat, PromptChain, PromptStep, StepResult,
    ValidationRule, ValidationType,
};
pub use prune::{BlockedTask, PruneResult};
pub use queue::{Queue, QueueError};
pub use session::{Session, SessionEvent};
pub use task::{
    ChainHandoffState, DependencyType, Task, TaskSource, TaskStatus, ValidationRequirement,
    WorkflowExpectations, WorkflowState,
};

//! Domain ports (interfaces) for the Abathur swarm system.

pub mod adapter;
pub mod agent_repository;
pub mod embedding;
pub mod goal_repository;
pub mod memory_repository;
pub mod null_embedding;
pub mod null_memory;
pub mod substrate;
pub mod task_repository;
pub mod task_schedule_repository;
pub mod trajectory_repository;
pub mod trigger_rule_repository;
pub mod workflow_repository;
pub mod worktree_repository;

pub use adapter::{EgressAdapter, IngestionAdapter};
pub use agent_repository::{AgentFilter, AgentRepository};
pub use embedding::{EmbeddingInput, EmbeddingOutput, EmbeddingProvider};
pub use goal_repository::{GoalFilter, GoalRepository};
pub use memory_repository::MemoryRepository;
pub use null_embedding::NullEmbeddingProvider;
pub use null_memory::NullMemoryRepository;
pub use substrate::{Substrate, SubstrateFactory};
pub use task_repository::{TaskFilter, TaskRepository};
pub use trajectory_repository::*;
pub use task_schedule_repository::{TaskScheduleFilter, TaskScheduleRepository};
pub use trigger_rule_repository::TriggerRuleRepository;
pub use workflow_repository::WorkflowRepository;
pub use worktree_repository::WorktreeRepository;

//! Domain ports (interfaces) for the Abathur swarm system.

pub mod adapter;
pub mod agent_repository;
pub mod embedding;
pub mod federated_goal_repository;
pub mod goal_repository;
pub mod memory_repository;
pub mod merge_request_repository;
pub mod null_embedding;
pub mod null_memory;
pub mod outbox_repository;
pub mod quiet_window_repository;
pub mod substrate;
pub mod task_repository;
pub mod task_schedule_repository;
pub mod trajectory_repository;
pub mod trigger_rule_repository;
pub mod worktree_repository;

pub use adapter::{EgressAdapter, IngestionAdapter};
pub use agent_repository::{AgentFilter, AgentRepository};
pub use embedding::{EmbeddingInput, EmbeddingOutput, EmbeddingProvider};
pub use federated_goal_repository::FederatedGoalRepository;
pub use goal_repository::{GoalFilter, GoalRepository};
pub use memory_repository::MemoryRepository;
pub use merge_request_repository::MergeRequestRepository;
pub use null_embedding::NullEmbeddingProvider;
pub use null_memory::NullMemoryRepository;
pub use outbox_repository::OutboxRepository;
pub use quiet_window_repository::{QuietWindowFilter, QuietWindowRepository};
pub use substrate::{Substrate, SubstrateFactory};
pub use task_repository::{TaskFilter, TaskRepository};
pub use task_schedule_repository::{TaskScheduleFilter, TaskScheduleRepository};
pub use trajectory_repository::*;
pub use trigger_rule_repository::TriggerRuleRepository;
pub use worktree_repository::WorktreeRepository;

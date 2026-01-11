//! Domain ports (interfaces) for the Abathur swarm system.

pub mod agent_repository;
pub mod goal_repository;
pub mod memory_repository;
pub mod substrate;
pub mod task_repository;
pub mod worktree_repository;

pub use agent_repository::{AgentFilter, AgentRepository};
pub use goal_repository::{GoalFilter, GoalRepository};
pub use memory_repository::MemoryRepository;
pub use substrate::{Substrate, SubstrateFactory};
pub use task_repository::{TaskFilter, TaskRepository};
pub use worktree_repository::WorktreeRepository;

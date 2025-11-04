//! Service layer module
//!
//! This module contains business logic services that coordinate domain operations:
//! - Task queue service
//! - Dependency resolver
//! - Priority calculator
//! - Memory service
//! - Session service
//!
//! Services use domain models and port traits to implement business workflows.

pub mod dependency_resolver;
pub mod hook_executor;
pub mod hook_registry;
pub mod memory_service;
pub mod priority_calculator;
pub mod prompt_chain_service;
pub mod task_queue_service;

pub use dependency_resolver::DependencyResolver;
pub use hook_executor::HookExecutor;
pub use hook_registry::HookRegistry;
pub use memory_service::MemoryService;
pub use priority_calculator::PriorityCalculator;
pub use prompt_chain_service::PromptChainService;
pub use task_queue_service::TaskQueueService;

// Re-export pruning types from domain for convenience
pub use crate::domain::models::{BlockedTask, PruneResult};

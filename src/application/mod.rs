//! Application layer module
//!
//! This module contains the application orchestration logic including:
//! - Swarm orchestrator
//! - Agent pool management
//! - Main event loop executor
//! - Task coordinator
//! - Resource monitor
//!
//! The application layer coordinates between domain services and infrastructure,
//! implementing the use cases and business workflows.

pub mod agent_executor;
pub mod loop_executor;
pub mod resource_monitor;
pub mod task_coordinator;

pub use agent_executor::{AgentExecutor, ExecutionContext, ExecutionError};
pub use loop_executor::{ConvergenceStrategy, LoopExecutor, LoopState};
pub use resource_monitor::{ResourceEvent, ResourceLimits, ResourceMonitor, ResourceStatus};
pub use task_coordinator::{TaskCoordinator, TaskStatusUpdate};

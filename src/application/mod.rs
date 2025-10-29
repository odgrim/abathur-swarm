//! Application layer module
//!
//! This module contains the application orchestration logic including:
//! - Swarm orchestrator
//! - Agent pool management
//! - Main event loop executor
//! - Task coordinator
//! - Resource monitor
//! - Task validation
//!
//! The application layer coordinates between domain services and infrastructure,
//! implementing the use cases and business workflows.

pub mod agent_executor;
pub mod loop_executor;
pub mod resource_monitor;
pub mod swarm_orchestrator;
pub mod task_coordinator;
pub mod validation;
pub mod workflow_verifier;

pub use agent_executor::{AgentExecutor, ExecutionContext, ExecutionError};
pub use loop_executor::{ConvergenceStrategy, LoopExecutor, LoopState};
pub use resource_monitor::{ResourceEvent, ResourceLimits, ResourceMonitor, ResourceStatus};
pub use swarm_orchestrator::{SwarmOrchestrator, SwarmState, SwarmStats};
pub use task_coordinator::{TaskCoordinator, TaskStatusUpdate};
pub use validation::{validate_contract, validate_task_completion, ValidationResult};
pub use workflow_verifier::{remediate_orphaned_workflow, WorkflowHealthMonitor};

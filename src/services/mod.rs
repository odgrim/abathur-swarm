//! Application services for the Abathur swarm system.

pub mod agent_service;
pub mod config;
pub mod dag_executor;
pub mod goal_service;
pub mod guardrails;
pub mod memory_service;
pub mod meta_planner;
pub mod swarm_orchestrator;
pub mod task_service;
pub mod worktree_service;

pub use agent_service::AgentService;
pub use config::{Config, ConfigError};
pub use dag_executor::{DagExecutor, ExecutorConfig, ExecutionEvent, ExecutionResults, ExecutionStatus, TaskResult};
pub use goal_service::GoalService;
pub use guardrails::{GuardrailResult, Guardrails, GuardrailsConfig, RuntimeMetrics};
pub use memory_service::{DecayConfig, MaintenanceReport, MemoryService, MemoryStats};
pub use meta_planner::{AgentMetrics, AgentSpec, Complexity, DecompositionPlan, MetaPlanner, MetaPlannerConfig, TaskSpec};
pub use swarm_orchestrator::{OrchestratorStatus, SwarmConfig, SwarmEvent, SwarmOrchestrator, SwarmStats};
pub use task_service::TaskService;
pub use worktree_service::{WorktreeConfig, WorktreeService, WorktreeStats};

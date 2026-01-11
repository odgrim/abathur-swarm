//! Application services for the Abathur swarm system.

pub mod agent_service;
pub mod config;
pub mod dag_executor;
pub mod evolution_loop;
pub mod goal_service;
pub mod guardrails;
pub mod integration_verifier;
pub mod llm_planner;
pub mod memory_service;
pub mod merge_queue;
pub mod meta_planner;
pub mod swarm_orchestrator;
pub mod task_service;
pub mod worktree_service;

pub use agent_service::AgentService;
pub use config::{Config, ConfigError};
pub use dag_executor::{DagExecutor, ExecutorConfig, ExecutionEvent, ExecutionResults, ExecutionStatus, TaskResult};
pub use evolution_loop::{EvolutionConfig, EvolutionEvent, EvolutionLoop, EvolutionTrigger, RefinementRequest, RefinementSeverity, TaskExecution, TaskOutcome, TemplateStats};
pub use goal_service::GoalService;
pub use guardrails::{GuardrailResult, Guardrails, GuardrailsConfig, RuntimeMetrics};
pub use integration_verifier::{IntegrationVerifierService, VerificationCheck, VerificationResult, VerifierConfig, TestResult};
pub use llm_planner::{LlmPlanner, LlmPlannerConfig, LlmDecomposition, LlmTaskSpec, PlanningContext, AgentRefinementSuggestion};
pub use memory_service::{DecayConfig, MaintenanceReport, MemoryService, MemoryStats};
pub use merge_queue::{MergeQueue, MergeQueueConfig, MergeQueueStats, MergeRequest, MergeResult, MergeStage, MergeStatus};
pub use meta_planner::{AgentMetrics, AgentSpec, Complexity, DecompositionPlan, MetaPlanner, MetaPlannerConfig, TaskSpec};
pub use swarm_orchestrator::{OrchestratorStatus, SwarmConfig, SwarmEvent, SwarmOrchestrator, SwarmStats};
pub use task_service::TaskService;
pub use worktree_service::{WorktreeConfig, WorktreeService, WorktreeStats};

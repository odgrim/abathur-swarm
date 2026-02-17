//! Application services for the Abathur swarm system.

pub mod agent_service;
pub mod audit_log;
pub mod circuit_breaker;
pub mod cold_start;
pub mod command_bus;
pub mod config;
pub mod context_truncation;
pub mod context_window;
pub mod cost_tracker;
pub mod dag_executor;
pub mod dag_restructure;
pub mod embedding_service;
pub mod builtin_handlers;
pub mod event_bus;
pub mod event_factory;
pub mod event_reactor;
pub mod event_scheduler;
pub mod event_store;
pub mod evolution_loop;
pub mod goal_context_service;
pub mod goal_service;
pub mod guardrails;
pub mod integration_verifier;
pub mod intent_verifier;
pub mod llm_planner;
pub mod memory_decay_daemon;
pub mod model_router;
pub mod memory_service;
pub mod merge_queue;
pub mod meta_planner; // Rust service module for decomposition planning
pub mod overmind;
pub mod phase_orchestrator;
pub mod swarm_orchestrator;
pub mod task_service;
pub mod trigger_rules;
pub mod workflow_builder;
pub mod convergence_bridge;
pub mod convergence_engine;
pub mod overseers;
pub mod worktree_service;

pub use agent_service::AgentService;
pub use audit_log::{AuditAction, AuditActor, AuditCategory, AuditEntry, AuditFilter, AuditLevel, AuditLogConfig, AuditLogService, AuditStats, DecisionRationale};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerService, CircuitCheckResult, CircuitScope, CircuitState, CircuitStats as CircuitBreakerStats, CircuitTrippedEvent, RecoveryAction, RecoveryPolicy};
pub use command_bus::{CommandBus, CommandEnvelope, CommandError, CommandId, CommandOutcome, CommandResult, CommandSource, DomainCommand, GoalCommand, GoalCommandHandler, MemoryCommand, MemoryCommandHandler, TaskCommand, TaskCommandHandler};
pub use cold_start::{ColdStartConfig, ColdStartReport, ColdStartService, Convention, ConventionCategory, Dependency, ProjectType};
pub use convergence_engine::ConvergenceEngine;
pub use config::{Config, ConfigError, A2AFederationConfig, TrustedSwarmConfig, TrustLevel, FederationAuthMethod, SwarmIdentityConfig};
pub use context_truncation::{TruncationConfig, estimate_tokens, truncate_section, truncate_to_token_budget, truncate_context_sections};
pub use context_window::{ContextWindowGuard, ContextWindowGuardConfig, ContextWindowCheck, model_context_window};
pub use cost_tracker::{CostTracker, CostSummary, ModelPricing, get_model_pricing, estimate_cost, estimate_cost_cents};
pub use model_router::{ModelRouter, ModelRoutingConfig, ModelSelection, AgentTierHint};
pub use dag_executor::{DagExecutor, ExecutorConfig, ExecutionEvent, ExecutionResults, ExecutionStatus, TaskResult};
pub use dag_restructure::{DagRestructureService, FailedAttempt, NewTaskSpec, RestructureConfig, RestructureContext, RestructureDecision, RestructureTrigger, TaskPriorityModifier};
pub use embedding_service::{EmbeddingService, EmbeddingServiceConfig, BatchEmbeddingReport};
pub use evolution_loop::{EvolutionAction, EvolutionConfig, EvolutionEvent, EvolutionLoop, EvolutionTrigger, RefinementRequest, RefinementSeverity, TaskExecution, TaskOutcome, TemplateStats};
pub use goal_context_service::GoalContextService;
pub use goal_service::GoalService;
pub use guardrails::{GuardrailResult, Guardrails, GuardrailsConfig, RuntimeMetrics};
pub use integration_verifier::{IntegrationVerifierService, VerificationCheck, VerificationResult, VerifierConfig, TestResult};
pub use intent_verifier::{IntentVerifierConfig, IntentVerifierService};
pub use llm_planner::{LlmPlanner, LlmPlannerConfig, LlmDecomposition, LlmTaskSpec, PlanningContext, AgentRefinementSuggestion};
pub use memory_decay_daemon::{DaemonHandle, DaemonStatus, DecayDaemonConfig, DecayDaemonEvent, MemoryDecayDaemon, StopReason};
pub use memory_service::{DecayConfig, MaintenanceReport, MemoryService, MemoryStats};
pub use merge_queue::{MergeQueue, MergeQueueConfig, MergeQueueStats, MergeRequest, MergeResult, MergeStage, MergeStatus};
pub use meta_planner::{AgentMetrics, AgentSpec, Complexity, DecompositionPlan, MetaPlanner, MetaPlannerConfig, TaskSpec};
pub use overmind::{OvermindConfig, OvermindService};
pub use phase_orchestrator::{PhaseOrchestrator, PhaseOrchestratorConfig};
pub use workflow_builder::build_workflow_from_decomposition;
pub use swarm_orchestrator::{ConvergenceLoopConfig, McpServerConfig, OrchestratorStatus, SwarmConfig, SwarmEvent, SwarmOrchestrator, SwarmStats, VerificationLevel};
pub use task_service::{TaskService, SpawnLimitConfig, SpawnLimitResult, SpawnLimitType};
pub use worktree_service::{WorktreeConfig, WorktreeService, WorktreeStats};
pub use event_bus::{EventBus, EventBusConfig, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent};
pub use event_reactor::{EventReactor, ReactorConfig, EventHandler, EventFilter, HandlerId, HandlerPriority, Reaction, HandlerContext, HandlerMetadata, ErrorStrategy};
pub use event_scheduler::{EventScheduler, SchedulerConfig, ScheduledEvent, ScheduleType};
pub use event_store::{EventQuery, EventStore, EventStoreError, EventStoreStats, InMemoryEventStore};
pub use trigger_rules::{TriggerRule, TriggerRuleEngine, TriggerCondition, TriggerAction, TriggerEventPayload, SerializableEventFilter, SerializableDomainCommand};

/// Extract a JSON object from LLM text output.
///
/// Handles markdown code blocks (```json...```) and JSON embedded in prose text.
pub fn extract_json_from_response(response: &str) -> String {
    let trimmed = response.trim();

    // Handle ```json ... ``` blocks
    if trimmed.starts_with("```json") {
        if let Some(end) = trimmed.rfind("```") {
            if end > 7 {
                return trimmed[7..end].trim().to_string();
            }
        }
    }

    // Handle ``` ... ``` blocks
    if trimmed.starts_with("```") {
        if let Some(end) = trimmed.rfind("```") {
            let start = if trimmed.starts_with("```\n") { 4 } else { 3 };
            if end > start {
                return trimmed[start..end].trim().to_string();
            }
        }
    }

    // If it already looks like a JSON object, use it directly
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return trimmed.to_string();
    }

    // Try to find a JSON object embedded in text
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                return trimmed[start..=end].to_string();
            }
        }
    }

    trimmed.to_string()
}

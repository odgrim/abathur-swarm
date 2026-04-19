//! Application services for the Abathur swarm system.

pub mod adapter_loader;
pub mod adapter_registry;
pub mod agent_service;
pub mod audit_log;
pub mod budget_tracker;
pub mod builtin_handlers;
pub mod circuit_breaker;
pub mod clock;
pub mod cold_start;
pub mod command_bus;
pub mod config;
pub mod context_truncation;
pub mod context_window;
pub mod cost_tracker;
pub mod cost_window_service;
pub mod crypto;
pub mod dag_executor;
pub mod dag_restructure;
pub mod embedding_service;
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
pub mod memory_service;
pub mod merge_queue;
pub mod metrics_exporter;
pub mod meta_planner; // Rust service module for decomposition planning
pub mod model_router;
pub mod outbox_poller;
pub mod overmind;
pub mod prompt_adapter;
pub mod supervisor;
pub use supervisor::{supervise, supervise_result, supervise_with_handle};
pub mod convergence_bridge;
pub mod convergence_engine;
pub mod federation;
pub mod overseers;
pub mod swarm_orchestrator;
pub mod task_schedule_service;
pub mod task_service;
pub mod trigger_rules;
pub mod workflow_engine;
pub mod worktree_service;

pub use adapter_registry::AdapterRegistry;
pub use agent_service::AgentService;
pub use audit_log::{
    AuditAction, AuditActor, AuditCategory, AuditEntry, AuditFilter, AuditLevel, AuditLogConfig,
    AuditLogService, AuditStats, DecisionRationale,
};
pub use budget_tracker::{
    BudgetOpportunity, BudgetState, BudgetTracker, BudgetTrackerConfig, BudgetWindow,
    BudgetWindowType,
};
pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerService, CircuitCheckResult, CircuitScope,
    CircuitState, CircuitStats as CircuitBreakerStats, CircuitTrippedEvent, RecoveryAction,
    RecoveryPolicy,
};
pub use clock::{Clock, DynClock, SystemClock, system_clock};
pub use cold_start::{
    ColdStartConfig, ColdStartReport, ColdStartService, Convention, ConventionCategory, Dependency,
    ProjectType,
};
pub use command_bus::{
    CommandBus, CommandEnvelope, CommandError, CommandId, CommandOutcome, CommandResult,
    CommandSource, DomainCommand, GoalCommand, GoalCommandHandler, MemoryCommand,
    MemoryCommandHandler, TaskCommand, TaskCommandHandler,
};
pub use config::{
    A2AFederationConfig, BudgetConfig, Config, ConfigError, FederationAuthMethod,
    SwarmIdentityConfig, TrustLevel, TrustedSwarmConfig,
};
pub use context_truncation::{
    TruncationConfig, estimate_tokens, truncate_context_sections, truncate_section,
    truncate_to_token_budget,
};
pub use context_window::{
    ContextWindowCheck, ContextWindowGuard, ContextWindowGuardConfig, model_context_window,
};
pub use convergence_engine::ConvergenceEngine;
pub use cost_tracker::{
    CostSummary, CostTracker, ModelPricing, estimate_cost, estimate_cost_cents, get_model_pricing,
};
pub use cost_window_service::{CostWindowService, QuietWindowCheck};
pub use dag_executor::{
    DagExecutor, ExecutionEvent, ExecutionResults, ExecutionStatus, ExecutorConfig, TaskResult,
};
pub use dag_restructure::{
    DagRestructureService, FailedAttempt, NewTaskSpec, RestructureConfig, RestructureContext,
    RestructureDecision, RestructureTrigger, TaskPriorityModifier,
};
pub use embedding_service::{BatchEmbeddingReport, EmbeddingService, EmbeddingServiceConfig};
pub use event_bus::{
    BudgetPressureLevel, EventBus, EventBusConfig, EventCategory, EventId, EventPayload,
    EventSeverity, SequenceNumber, UnifiedEvent,
};
pub use event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, EventReactor, HandlerContext, HandlerId,
    HandlerMetadata, HandlerPriority, Reaction, ReactorConfig,
};
pub use event_scheduler::{EventScheduler, ScheduleType, ScheduledEvent, SchedulerConfig};
pub use event_store::{
    EventQuery, EventStore, EventStoreError, EventStoreStats, InMemoryEventStore,
};
pub use evolution_loop::{
    EvolutionAction, EvolutionConfig, EvolutionEvent, EvolutionLoop, EvolutionTrigger,
    RefinementRepository, RefinementRequest, RefinementSeverity, RefinementStatus, TaskExecution,
    TaskOutcome, TemplateStats,
};
pub use federation::{FederationConfig, FederationService};
pub use goal_context_service::GoalContextService;
pub use goal_service::GoalService;
pub use guardrails::{GuardrailResult, Guardrails, GuardrailsConfig, RuntimeMetrics};
pub use integration_verifier::{
    IntegrationVerifierService, TestResult, VerificationCheck, VerificationResult, VerifierConfig,
};
pub use intent_verifier::{IntentVerifierConfig, IntentVerifierService};
pub use llm_planner::{
    AgentRefinementSuggestion, LlmDecomposition, LlmPlanner, LlmPlannerConfig, LlmTaskSpec,
    PlanningContext,
};
pub use memory_decay_daemon::{
    DaemonHandle, DaemonStatus, DecayDaemonConfig, DecayDaemonEvent, MemoryDecayDaemon, StopReason,
};
pub use memory_service::{DecayConfig, MaintenanceReport, MemoryService, MemoryStats};
pub use merge_queue::{
    MergeQueue, MergeQueueConfig, MergeQueueStats, MergeRequest, MergeResult, MergeStage,
    MergeStatus, validate_branch_name, validate_workdir,
};
pub use meta_planner::{
    AgentMetrics, AgentSpec, Complexity, DecompositionPlan, MetaPlanner, MetaPlannerConfig,
    TaskSpec,
};
pub use model_router::{AgentTierHint, ModelRouter, ModelRoutingConfig, ModelSelection};
pub use outbox_poller::{OutboxPoller, OutboxPollerConfig, OutboxPollerHandle};
pub use overmind::{OvermindConfig, OvermindService};
pub use swarm_orchestrator::{
    ConvergenceLoopConfig, McpServerConfig, OrchestratorStatus, SwarmConfig, SwarmEvent,
    SwarmOrchestrator, SwarmStats, VerificationLevel,
};
pub use task_schedule_service::TaskScheduleService;
pub use task_service::{
    PruneResult, PruneSkipped, SpawnLimitConfig, SpawnLimitResult, SpawnLimitType, TaskService,
};
pub use trigger_rules::{
    SerializableDomainCommand, SerializableEventFilter, TriggerAction, TriggerCondition,
    TriggerEventPayload, TriggerRule, TriggerRuleEngine, normalize_cron_expression,
    validate_cron_expression,
};
pub use workflow_engine::WorkflowEngine;
pub use worktree_service::{WorktreeConfig, WorktreeService, WorktreeStats};

/// Extract a JSON object from LLM text output.
///
/// Handles markdown code blocks (```json...```) and JSON embedded in prose text.
///
/// Resolution order (first match wins):
/// 1. ```` ```json ... ``` ```` fenced block — return content between opening ```` ```json ```` and last ```` ``` ````.
/// 2. ```` ``` ... ``` ```` fenced block — return content between opening ```` ``` ```` and last ```` ``` ````.
/// 3. Trimmed input that starts with `{` and ends with `}` — returned as-is.
/// 4. Embedded JSON — slice from first `{` through last `}`.
/// 5. Otherwise return the trimmed input unchanged.
///
/// # Known limitation
///
/// This function uses `rfind('}')` to locate the end of an embedded JSON object.
/// If the JSON value contains a `}` inside a string (e.g. `{"msg": "hi}"}` embedded
/// in prose that also ends with `}`), the extractor may over-extract past the real
/// end of the object. Fixing this correctly requires an actual JSON-aware scanner
/// (brace counting that tracks string literals and escapes), which is out of scope
/// for this cheap text heuristic.
pub fn extract_json_from_response(response: &str) -> String {
    let trimmed = response.trim();

    // 1. ```json ... ``` fenced block.
    if let Some(rest) = trimmed.strip_prefix("```json")
        && let Some(end) = rest.rfind("```")
    {
        return rest[..end].trim().to_string();
    }

    // 2. ``` ... ``` fenced block (no language tag).
    if let Some(rest) = trimmed.strip_prefix("```")
        && let Some(end) = rest.rfind("```")
    {
        return rest[..end].trim().to_string();
    }

    // 3. Looks like a bare JSON object already.
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return trimmed.to_string();
    }

    // 4. JSON embedded in prose: slice from first `{` to last `}`.
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}'))
        && end > start
    {
        return trimmed[start..=end].to_string();
    }

    // 5. Fallback: no recognizable JSON structure.
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_bare_object() {
        assert_eq!(extract_json_from_response(r#"{"x":1}"#), r#"{"x":1}"#);
    }

    #[test]
    fn extract_json_with_whitespace() {
        assert_eq!(
            extract_json_from_response("  \n\t{\"x\":1}\n  "),
            r#"{"x":1}"#
        );
    }

    #[test]
    fn extract_json_code_fenced_with_json_tag() {
        let input = "```json\n{\"x\":1}\n```";
        assert_eq!(extract_json_from_response(input), r#"{"x":1}"#);
    }

    #[test]
    fn extract_json_code_fenced_without_tag() {
        let input = "```\n{\"x\":1}\n```";
        assert_eq!(extract_json_from_response(input), r#"{"x":1}"#);
    }

    #[test]
    fn extract_json_code_fenced_with_json_tag_and_trailing_whitespace() {
        let input = "  ```json\n{\"x\":1}\n```  ";
        assert_eq!(extract_json_from_response(input), r#"{"x":1}"#);
    }

    #[test]
    fn extract_json_embedded_in_prose() {
        let input = r#"Sure, here's the JSON: {"x":1}. Thanks!"#;
        assert_eq!(extract_json_from_response(input), r#"{"x":1}"#);
    }

    #[test]
    fn extract_json_multi_line_inside_prose() {
        let input =
            "Here is the result:\n{\n  \"x\": 1,\n  \"y\": 2\n}\nLet me know if you need more.";
        assert_eq!(
            extract_json_from_response(input),
            "{\n  \"x\": 1,\n  \"y\": 2\n}"
        );
    }

    #[test]
    fn extract_json_multi_line_bare() {
        let input = "{\n  \"x\": 1,\n  \"y\": 2\n}";
        assert_eq!(
            extract_json_from_response(input),
            "{\n  \"x\": 1,\n  \"y\": 2\n}"
        );
    }

    #[test]
    fn extract_json_empty_input() {
        assert_eq!(extract_json_from_response(""), "");
    }

    #[test]
    fn extract_json_whitespace_only_input() {
        assert_eq!(extract_json_from_response("   \n\t  "), "");
    }

    #[test]
    fn extract_json_no_json_at_all() {
        assert_eq!(extract_json_from_response("hello world"), "hello world");
    }

    #[test]
    fn extract_json_no_json_trims_input() {
        assert_eq!(extract_json_from_response("  hello world  "), "hello world");
    }

    #[test]
    fn extract_json_nested_object_in_fence() {
        let input = "```json\n{\"a\": {\"b\": 2}}\n```";
        assert_eq!(extract_json_from_response(input), r#"{"a": {"b": 2}}"#);
    }

    #[test]
    fn extract_json_known_limitation_brace_in_string() {
        // Documented known limitation: rfind('}') cannot see into strings, so
        // a `}` character inside a JSON string value followed by prose that
        // happens to end with `}` would over-extract. Here we just assert the
        // current (documented) behavior rather than a "correct" one.
        let input = r#"prefix {"msg": "x"} suffix}"#;
        // First `{` is at the start of the JSON object, last `}` is the stray
        // one in the suffix — so we capture everything through the suffix `}`.
        assert_eq!(extract_json_from_response(input), r#"{"msg": "x"} suffix}"#);
    }
}

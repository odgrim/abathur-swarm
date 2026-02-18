//! Public types for the swarm orchestrator.
//!
//! Configuration structs, event types, status enums, and statistics
//! used throughout the orchestrator subsystem.

use std::path::PathBuf;
use uuid::Uuid;

use crate::domain::models::workflow_template::WorkflowTemplate;

/// Configuration for the swarm orchestrator.
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Maximum concurrent agents.
    pub max_agents: usize,
    /// Default max turns per agent invocation.
    pub default_max_turns: u32,
    /// Whether to use worktrees for task isolation.
    pub use_worktrees: bool,
    /// Maximum execution time per goal (seconds).
    pub goal_timeout_secs: u64,
    /// Whether to auto-retry failed tasks.
    pub auto_retry: bool,
    /// Maximum retries per task.
    pub max_task_retries: u32,
    /// Maximum review loop-back iterations (plan → implement → review) before
    /// falling through to normal failure handling.
    pub max_review_iterations: u32,
    /// Base path for worktrees.
    pub worktree_base_path: PathBuf,
    /// Repository path.
    pub repo_path: PathBuf,
    /// Default base ref for worktrees.
    pub default_base_ref: String,
    /// Whether to use LLM for task decomposition.
    pub use_llm_decomposition: bool,
    /// Whether to run integration verification on task completion.
    pub verify_on_completion: bool,
    /// Whether to use merge queue for controlled merging.
    pub use_merge_queue: bool,
    /// Whether to prefer creating pull requests over direct merges.
    /// When true, tries `gh pr create` first; falls back to merge queue on failure.
    pub prefer_pull_requests: bool,
    /// Whether to track agent evolution metrics.
    pub track_evolution: bool,
    /// MCP server addresses for agent access to system services.
    /// These get passed to substrate requests so agents can access memory, tasks, etc.
    pub mcp_servers: McpServerConfig,
    /// Spawn limits for task creation (subtask depth, count, etc.).
    pub spawn_limits: crate::services::config::SpawnLimitsConfig,
    /// Whether to enable intent verification and convergence loops.
    pub enable_intent_verification: bool,
    /// Configuration for convergence behavior.
    pub convergence: ConvergenceLoopConfig,
    /// Interval in seconds for the reconciliation safety-net loop (default: 30).
    pub reconciliation_interval_secs: Option<u64>,
    /// Configurable polling intervals for all scheduled handlers.
    pub polling: PollingConfig,
    /// Retention period for events in days (default: 30). Events older than
    /// this are pruned by the EventPruningHandler.
    pub event_retention_days: u64,

    /// Whether convergent execution is enabled at all.
    /// When false, all tasks run Direct regardless of their execution_mode field.
    /// This is the global kill switch.
    pub convergence_enabled: bool,

    /// Default execution mode when not explicitly set and heuristic is disabled.
    /// When set to Some(mode), all tasks without an explicit mode use this.
    /// When None, the classification heuristic decides.
    pub default_execution_mode: Option<crate::domain::models::ExecutionMode>,

    /// Optional workflow template for configuring the Overmind's phase sequence.
    pub workflow_template: Option<WorkflowTemplate>,
}

/// Configurable polling intervals (seconds) for all scheduled handlers.
#[derive(Debug, Clone)]
pub struct PollingConfig {
    pub reconciliation_interval_secs: u64,
    pub stats_update_interval_secs: u64,
    pub memory_maintenance_interval_secs: u64,
    pub memory_reconciliation_interval_secs: u64,
    pub goal_reconciliation_interval_secs: u64,
    pub trigger_catchup_interval_secs: u64,
    pub watermark_audit_interval_secs: u64,
    pub retry_check_interval_secs: u64,
    pub specialist_check_interval_secs: u64,
    pub evolution_evaluation_interval_secs: u64,
    pub escalation_check_interval_secs: u64,
    pub goal_evaluation_interval_secs: u64,
    pub a2a_poll_interval_secs: u64,

    // --- Goal convergence check ---
    /// Interval for deep goal convergence check (default: 14400s = 4 hours).
    pub goal_convergence_check_interval_secs: u64,
    /// Whether periodic goal convergence checks are enabled (default: true).
    pub goal_convergence_check_enabled: bool,
    /// Interval for polling EventStore for cross-process events (default: 5s).
    pub event_store_poll_interval_secs: u64,
    /// Interval for retrying dead letter queue entries (default: 60s).
    pub dead_letter_retry_interval_secs: u64,
    /// Interval for pruning old events (default: 21600s = 6 hours).
    pub event_pruning_interval_secs: u64,

    // --- SLA enforcement ---
    /// Interval for SLA check (default: 60s).
    pub sla_check_interval_secs: u64,
    /// Warning threshold as fraction of total time remaining (default: 0.25).
    pub sla_warning_threshold_pct: f64,
    /// Critical threshold as fraction of total time remaining (default: 0.10).
    pub sla_critical_threshold_pct: f64,
    /// Whether to auto-escalate on SLA breach (default: true).
    pub sla_auto_escalate_on_breach: bool,

    // --- Priority aging ---
    /// Whether priority aging is enabled (default: false, opt-in).
    pub priority_aging_enabled: bool,
    /// Interval for priority aging check (default: 300s).
    pub priority_aging_interval_secs: u64,
    /// Seconds before Low -> Normal promotion (default: 3600).
    pub priority_aging_low_to_normal_secs: u64,
    /// Seconds before Normal -> High promotion (default: 7200).
    pub priority_aging_normal_to_high_secs: u64,
    /// Seconds before High -> Critical promotion (default: 14400).
    pub priority_aging_high_to_critical_secs: u64,

    // --- Memory-informed decomposition ---
    /// Whether memory-informed decomposition is enabled (default: true).
    pub memory_informed_decomposition_enabled: bool,
    /// Cooldown per goal for memory-informed re-evaluation (default: 120s).
    pub memory_informed_cooldown_per_goal_secs: u64,

    // --- Task completion learning ---
    /// Whether task completion learning is enabled (default: true).
    pub task_learning_enabled: bool,
    /// Minimum retries before storing a learning pattern (default: 1).
    pub task_learning_min_retries: u32,
    /// Whether to store efficiency patterns for fast completions (default: true).
    pub task_learning_store_efficiency: bool,

    // --- Diagnostic/remediation task creation ---
    /// Whether to auto-create diagnostic tasks from drift detection (default: true).
    pub auto_create_diagnostic_tasks: bool,
    /// Maximum diagnostic tasks per goal (default: 3).
    pub max_diagnostic_tasks_per_goal: u32,
    /// Whether to auto-create remediation tasks from constraint violations (default: true).
    pub auto_create_remediation_tasks: bool,

    // --- Startup catch-up ---
    /// Whether startup catch-up is enabled (default: true).
    pub startup_catchup_enabled: bool,
    /// Maximum events to replay during startup catch-up (default: 10000).
    pub startup_max_replay_events: u64,
    /// Stale task threshold in seconds for startup orphan detection (default: 300).
    pub startup_stale_task_threshold_secs: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            reconciliation_interval_secs: 30,
            stats_update_interval_secs: 10,
            memory_maintenance_interval_secs: 300,
            memory_reconciliation_interval_secs: 120,
            goal_reconciliation_interval_secs: 60,
            trigger_catchup_interval_secs: 300,
            watermark_audit_interval_secs: 600,
            retry_check_interval_secs: 15,
            specialist_check_interval_secs: 30,
            evolution_evaluation_interval_secs: 120,
            escalation_check_interval_secs: 30,
            goal_evaluation_interval_secs: 60,
            a2a_poll_interval_secs: 15,

            // Goal convergence check
            goal_convergence_check_interval_secs: 14400, // 4 hours
            goal_convergence_check_enabled: true,
            event_store_poll_interval_secs: 5,
            dead_letter_retry_interval_secs: 60,
            event_pruning_interval_secs: 21600, // 6 hours

            // SLA enforcement
            sla_check_interval_secs: 60,
            sla_warning_threshold_pct: 0.25,
            sla_critical_threshold_pct: 0.10,
            sla_auto_escalate_on_breach: true,

            // Priority aging (opt-in)
            priority_aging_enabled: false,
            priority_aging_interval_secs: 300,
            priority_aging_low_to_normal_secs: 3600,
            priority_aging_normal_to_high_secs: 7200,
            priority_aging_high_to_critical_secs: 14400,

            // Memory-informed decomposition
            memory_informed_decomposition_enabled: true,
            memory_informed_cooldown_per_goal_secs: 120,

            // Task completion learning
            task_learning_enabled: true,
            task_learning_min_retries: 1,
            task_learning_store_efficiency: true,

            // Diagnostic/remediation task creation
            auto_create_diagnostic_tasks: true,
            max_diagnostic_tasks_per_goal: 3,
            auto_create_remediation_tasks: true,

            // Startup catch-up
            startup_catchup_enabled: true,
            startup_max_replay_events: 10000,
            startup_stale_task_threshold_secs: 300,
        }
    }
}

/// Configuration for the convergence loop behavior.
#[derive(Debug, Clone)]
pub struct ConvergenceLoopConfig {
    /// Maximum iterations before giving up.
    pub max_iterations: u32,
    /// Minimum confidence to accept partial satisfaction.
    pub min_confidence_threshold: f64,
    /// Whether to require full satisfaction (vs. partial).
    pub require_full_satisfaction: bool,
    /// Whether to automatically retry on partial satisfaction.
    pub auto_retry_partial: bool,
    /// Timeout for the entire convergence loop (seconds).
    pub convergence_timeout_secs: u64,
    /// Verification level: "goal" for goal-level, "wave" for per-wave, "task" for per-task.
    pub verification_level: VerificationLevel,
}

/// Level at which intent verification is performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerificationLevel {
    /// Verify only at goal completion (default).
    #[default]
    Goal,
    /// Verify after each wave of parallel tasks completes.
    Wave,
    /// Verify each task individually after completion.
    Task,
}

impl VerificationLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Wave => "wave",
            Self::Task => "task",
        }
    }
}

impl Default for ConvergenceLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 3,
            min_confidence_threshold: 0.7,
            require_full_satisfaction: false,
            auto_retry_partial: true,
            convergence_timeout_secs: 7200, // 2 hours
            verification_level: VerificationLevel::default(),
        }
    }
}

/// MCP server configuration for agent access to system services.
#[derive(Debug, Clone, Default)]
pub struct McpServerConfig {
    /// Memory MCP server address (e.g., "http://localhost:9100")
    pub memory_server: Option<String>,
    /// Tasks MCP server address (e.g., "http://localhost:9101")
    pub tasks_server: Option<String>,
    /// A2A gateway address (e.g., "http://localhost:8080")
    pub a2a_gateway: Option<String>,
    /// Whether to auto-start embedded MCP servers.
    /// When true, the orchestrator will start MCP servers in-process before goal processing.
    pub auto_start_servers: bool,
    /// Host to bind embedded servers to.
    pub bind_host: String,
    /// Base port for embedded servers (memory=base, tasks=base+1, a2a=base+2).
    pub base_port: u16,
}

impl McpServerConfig {
    /// Create a new config with auto-start enabled on default ports.
    pub fn auto_start() -> Self {
        Self {
            memory_server: Some("http://127.0.0.1:9100".to_string()),
            tasks_server: Some("http://127.0.0.1:9101".to_string()),
            a2a_gateway: Some("http://127.0.0.1:8080".to_string()),
            auto_start_servers: true,
            bind_host: "127.0.0.1".to_string(),
            base_port: 9100,
        }
    }
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_agents: 4,
            default_max_turns: 25,
            use_worktrees: true,
            goal_timeout_secs: 3600,
            auto_retry: true,
            max_task_retries: 3,
            max_review_iterations: 3,
            worktree_base_path: PathBuf::from(".abathur/worktrees"),
            repo_path: PathBuf::from("."),
            default_base_ref: "main".to_string(),
            use_llm_decomposition: true,
            verify_on_completion: true,
            use_merge_queue: true,
            prefer_pull_requests: true,
            track_evolution: true,
            mcp_servers: McpServerConfig::default(),
            spawn_limits: crate::services::config::SpawnLimitsConfig::default(),
            enable_intent_verification: true,
            convergence: ConvergenceLoopConfig::default(),
            reconciliation_interval_secs: None,
            polling: PollingConfig::default(),
            event_retention_days: 30,
            convergence_enabled: true,
            default_execution_mode: Some(crate::domain::models::ExecutionMode::Convergent { parallel_samples: None }),
            workflow_template: None,
        }
    }
}

/// Orchestrator status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorStatus {
    /// Not running.
    Idle,
    /// Running and processing goals.
    Running,
    /// Paused but can resume.
    Paused,
    /// Shutting down gracefully.
    ShuttingDown,
    /// Stopped.
    Stopped,
}

/// Event emitted by the orchestrator.
#[derive(Debug, Clone)]
pub enum SwarmEvent {
    /// Orchestrator started.
    Started,
    /// Goal processing started.
    GoalStarted { goal_id: Uuid, goal_name: String },
    /// Goal decomposed into tasks.
    GoalDecomposed { goal_id: Uuid, task_count: usize },
    /// Task submitted (created and added to the system).
    TaskSubmitted { task_id: Uuid, task_title: String, goal_id: Uuid },
    /// Task readiness updated.
    TaskReady { task_id: Uuid, task_title: String },
    /// Task spawned.
    TaskSpawned { task_id: Uuid, task_title: String, agent_type: Option<String> },
    /// Worktree created for task.
    WorktreeCreated { task_id: Uuid, path: String },
    /// Task completed.
    TaskCompleted { task_id: Uuid, tokens_used: u64 },
    /// Task failed.
    TaskFailed { task_id: Uuid, error: String, retry_count: u32 },
    /// Task retrying.
    TaskRetrying { task_id: Uuid, attempt: u32, max_attempts: u32 },
    /// Task verified.
    TaskVerified { task_id: Uuid, passed: bool, checks_passed: usize, checks_total: usize, failures_summary: Option<String> },
    /// Task queued for merge.
    TaskQueuedForMerge { task_id: Uuid, stage: String },
    /// Pull request created for completed task.
    PullRequestCreated { task_id: Uuid, pr_url: String, branch: String },
    /// Task claimed by an agent.
    TaskClaimed { task_id: Uuid, agent_type: String },
    /// Task merged successfully.
    TaskMerged { task_id: Uuid, commit_sha: String },
    /// Agent instance completed.
    AgentInstanceCompleted { instance_id: Uuid, task_id: Uuid, tokens_used: u64 },
    /// Reconciliation completed.
    ReconciliationCompleted { corrections_made: u32 },
    /// Evolution event triggered.
    EvolutionTriggered { template_name: String, trigger: String },
    /// Specialist agent spawned for special handling.
    SpecialistSpawned { specialist_type: String, trigger: String, task_id: Option<Uuid> },
    /// Agent dynamically created through capability-driven genesis.
    AgentCreated { agent_type: String, tier: String },
    /// Goal alignment evaluated.
    GoalAlignmentEvaluated { task_id: Uuid, overall_score: f64, passes: bool },
    /// DAG restructure triggered for a permanently failed task.
    RestructureTriggered { task_id: Uuid, decision: String },
    /// Spawn limit exceeded, specialist evaluation requested.
    SpawnLimitExceeded {
        parent_task_id: Uuid,
        limit_type: String,
        current_value: u32,
        limit_value: u32,
    },
    /// Goal iteration completed (all current tasks done, goal remains active).
    ///
    /// Goals are never "completed" - they are convergent attractors.
    /// This event indicates a successful iteration of work toward the goal.
    GoalIterationCompleted { goal_id: Uuid, tasks_completed: usize },
    /// Goal paused (human-initiated).
    GoalPaused { goal_id: Uuid, reason: String },
    /// Intent verification started.
    IntentVerificationStarted { goal_id: Uuid, iteration: u32 },
    /// Intent verification completed.
    IntentVerificationCompleted {
        goal_id: Uuid,
        satisfaction: String,
        confidence: f64,
        gaps_count: usize,
        iteration: u32,
        will_retry: bool,
    },
    /// Convergence loop completed.
    ConvergenceCompleted {
        goal_id: Uuid,
        converged: bool,
        iterations: u32,
        final_satisfaction: String,
    },
    /// Human escalation required.
    HumanEscalationRequired {
        goal_id: Option<Uuid>,
        task_id: Option<Uuid>,
        reason: String,
        urgency: String,
        questions: Vec<String>,
        is_blocking: bool,
    },
    /// Human response received to escalation.
    HumanResponseReceived {
        escalation_id: Uuid,
        decision: String,
        allows_continuation: bool,
    },
    /// Branch verification started.
    BranchVerificationStarted {
        branch_task_ids: Vec<Uuid>,
        waiting_task_ids: Vec<Uuid>,
    },
    /// Branch verification completed.
    BranchVerificationCompleted {
        branch_satisfied: bool,
        dependents_can_proceed: bool,
        gaps_count: usize,
    },
    /// Subtask branch merged into feature branch.
    SubtaskMergedToFeature { task_id: Uuid, feature_branch: String },
    /// Semantic drift detected in convergence loop.
    SemanticDriftDetected {
        goal_id: Uuid,
        recurring_gaps: Vec<String>,
        iterations: u32,
    },
    /// Orchestrator paused.
    Paused,
    /// Orchestrator resumed.
    Resumed,
    /// Orchestrator stopped.
    Stopped,
    /// Status update.
    StatusUpdate(SwarmStats),
}

/// Statistics about the swarm.
#[derive(Debug, Clone, Default)]
pub struct SwarmStats {
    pub active_goals: usize,
    pub pending_tasks: usize,
    pub ready_tasks: usize,
    pub running_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub active_agents: usize,
    pub active_worktrees: usize,
    pub total_tokens_used: u64,
}

/// Try to convert an EventPayload into a SwarmEvent.
///
/// Not all EventPayload variants have SwarmEvent counterparts. Returns `None`
/// for payloads that don't map to a TUI-displayable SwarmEvent.
impl SwarmEvent {
    pub fn from_event_payload(payload: &crate::services::event_bus::EventPayload) -> Option<Self> {
        use crate::services::event_bus::EventPayload;
        match payload {
            EventPayload::OrchestratorStarted => Some(SwarmEvent::Started),
            EventPayload::OrchestratorPaused => Some(SwarmEvent::Paused),
            EventPayload::OrchestratorResumed => Some(SwarmEvent::Resumed),
            EventPayload::OrchestratorStopped => Some(SwarmEvent::Stopped),
            EventPayload::GoalStarted { goal_id, goal_name } => Some(SwarmEvent::GoalStarted {
                goal_id: *goal_id,
                goal_name: goal_name.clone(),
            }),
            EventPayload::GoalDecomposed { goal_id, task_count } => Some(SwarmEvent::GoalDecomposed {
                goal_id: *goal_id,
                task_count: *task_count,
            }),
            EventPayload::GoalIterationCompleted { goal_id, tasks_completed } => Some(SwarmEvent::GoalIterationCompleted {
                goal_id: *goal_id,
                tasks_completed: *tasks_completed,
            }),
            EventPayload::GoalPaused { goal_id, reason } => Some(SwarmEvent::GoalPaused {
                goal_id: *goal_id,
                reason: reason.clone(),
            }),
            EventPayload::ConvergenceCompleted { goal_id, converged, iterations, final_satisfaction } => Some(SwarmEvent::ConvergenceCompleted {
                goal_id: *goal_id,
                converged: *converged,
                iterations: *iterations,
                final_satisfaction: final_satisfaction.clone(),
            }),
            EventPayload::SemanticDriftDetected { goal_id, recurring_gaps, iterations } => Some(SwarmEvent::SemanticDriftDetected {
                goal_id: *goal_id,
                recurring_gaps: recurring_gaps.clone(),
                iterations: *iterations,
            }),
            EventPayload::TaskSubmitted { task_id, task_title, goal_id } => Some(SwarmEvent::TaskSubmitted {
                task_id: *task_id,
                task_title: task_title.clone(),
                goal_id: *goal_id,
            }),
            EventPayload::TaskReady { task_id, task_title } => Some(SwarmEvent::TaskReady {
                task_id: *task_id,
                task_title: task_title.clone(),
            }),
            EventPayload::TaskSpawned { task_id, task_title, agent_type } => Some(SwarmEvent::TaskSpawned {
                task_id: *task_id,
                task_title: task_title.clone(),
                agent_type: agent_type.clone(),
            }),
            EventPayload::TaskCompleted { task_id, tokens_used } => Some(SwarmEvent::TaskCompleted {
                task_id: *task_id,
                tokens_used: *tokens_used,
            }),
            EventPayload::TaskFailed { task_id, error, retry_count } => Some(SwarmEvent::TaskFailed {
                task_id: *task_id,
                error: error.clone(),
                retry_count: *retry_count,
            }),
            EventPayload::TaskRetrying { task_id, attempt, max_attempts } => Some(SwarmEvent::TaskRetrying {
                task_id: *task_id,
                attempt: *attempt,
                max_attempts: *max_attempts,
            }),
            EventPayload::TaskVerified { task_id, passed, checks_passed, checks_total } => Some(SwarmEvent::TaskVerified {
                task_id: *task_id,
                passed: *passed,
                checks_passed: *checks_passed,
                checks_total: *checks_total,
                failures_summary: None, // Not available from EventPayload
            }),
            EventPayload::TaskQueuedForMerge { task_id, stage } => Some(SwarmEvent::TaskQueuedForMerge {
                task_id: *task_id,
                stage: stage.clone(),
            }),
            EventPayload::PullRequestCreated { task_id, pr_url, branch } => Some(SwarmEvent::PullRequestCreated {
                task_id: *task_id,
                pr_url: pr_url.clone(),
                branch: branch.clone(),
            }),
            EventPayload::TaskMerged { task_id, commit_sha } => Some(SwarmEvent::TaskMerged {
                task_id: *task_id,
                commit_sha: commit_sha.clone(),
            }),
            EventPayload::WorktreeCreated { task_id, path } => Some(SwarmEvent::WorktreeCreated {
                task_id: *task_id,
                path: path.clone(),
            }),
            EventPayload::TaskClaimed { task_id, agent_type } => Some(SwarmEvent::TaskClaimed {
                task_id: *task_id,
                agent_type: agent_type.clone(),
            }),
            EventPayload::AgentInstanceCompleted { instance_id, task_id, tokens_used } => Some(SwarmEvent::AgentInstanceCompleted {
                instance_id: *instance_id,
                task_id: *task_id,
                tokens_used: *tokens_used,
            }),
            EventPayload::ReconciliationCompleted { corrections_made } => Some(SwarmEvent::ReconciliationCompleted {
                corrections_made: *corrections_made,
            }),
            EventPayload::EvolutionTriggered { template_name, trigger } => Some(SwarmEvent::EvolutionTriggered {
                template_name: template_name.clone(),
                trigger: trigger.clone(),
            }),
            EventPayload::SpecialistSpawned { specialist_type, trigger, task_id } => Some(SwarmEvent::SpecialistSpawned {
                specialist_type: specialist_type.clone(),
                trigger: trigger.clone(),
                task_id: *task_id,
            }),
            EventPayload::AgentCreated { agent_type, tier } => Some(SwarmEvent::AgentCreated {
                agent_type: agent_type.clone(),
                tier: tier.clone(),
            }),
            EventPayload::GoalAlignmentEvaluated { task_id, overall_score, passes } => Some(SwarmEvent::GoalAlignmentEvaluated {
                task_id: *task_id,
                overall_score: *overall_score,
                passes: *passes,
            }),
            EventPayload::RestructureTriggered { task_id, decision } => Some(SwarmEvent::RestructureTriggered {
                task_id: *task_id,
                decision: decision.clone(),
            }),
            EventPayload::SpawnLimitExceeded { parent_task_id, limit_type, current_value, limit_value } => Some(SwarmEvent::SpawnLimitExceeded {
                parent_task_id: *parent_task_id,
                limit_type: limit_type.clone(),
                current_value: *current_value,
                limit_value: *limit_value,
            }),
            EventPayload::SubtaskMergedToFeature { task_id, feature_branch } => Some(SwarmEvent::SubtaskMergedToFeature {
                task_id: *task_id,
                feature_branch: feature_branch.clone(),
            }),
            EventPayload::HumanEscalationRequired { goal_id, task_id, reason, urgency, questions, is_blocking } => Some(SwarmEvent::HumanEscalationRequired {
                goal_id: *goal_id,
                task_id: *task_id,
                reason: reason.clone(),
                urgency: urgency.clone(),
                questions: questions.clone(),
                is_blocking: *is_blocking,
            }),
            EventPayload::HumanResponseReceived { escalation_id, decision, allows_continuation } => Some(SwarmEvent::HumanResponseReceived {
                escalation_id: *escalation_id,
                decision: decision.clone(),
                allows_continuation: *allows_continuation,
            }),
            EventPayload::IntentVerificationStarted { goal_id, iteration } => Some(SwarmEvent::IntentVerificationStarted {
                goal_id: *goal_id,
                iteration: *iteration,
            }),
            EventPayload::IntentVerificationCompleted { goal_id, satisfaction, confidence, gaps_count, iteration, will_retry } => Some(SwarmEvent::IntentVerificationCompleted {
                goal_id: *goal_id,
                satisfaction: satisfaction.clone(),
                confidence: *confidence,
                gaps_count: *gaps_count,
                iteration: *iteration,
                will_retry: *will_retry,
            }),
            EventPayload::BranchVerificationStarted { branch_task_ids, waiting_task_ids } => Some(SwarmEvent::BranchVerificationStarted {
                branch_task_ids: branch_task_ids.clone(),
                waiting_task_ids: waiting_task_ids.clone(),
            }),
            EventPayload::BranchVerificationCompleted { branch_satisfied, dependents_can_proceed, gaps_count } => Some(SwarmEvent::BranchVerificationCompleted {
                branch_satisfied: *branch_satisfied,
                dependents_can_proceed: *dependents_can_proceed,
                gaps_count: *gaps_count,
            }),
            // EventPayload variants with no SwarmEvent counterpart
            _ => None,
        }
    }
}

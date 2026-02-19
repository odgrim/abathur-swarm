//! EventBus service for unified event streaming and distribution.
//!
//! Provides a broadcast-based event system with sequence numbering,
//! optional persistence, and correlation tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::event_store::EventStore;
use super::dag_executor::{ExecutionEvent, ExecutionResults, ExecutionStatus, TaskResult};
use super::swarm_orchestrator::{SwarmEvent, SwarmStats};

/// Unique identifier for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Monotonically increasing sequence number assigned by EventBus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SequenceNumber(pub u64);

impl SequenceNumber {
    pub fn zero() -> Self {
        Self(0)
    }
}

impl std::fmt::Display for SequenceNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Event severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventSeverity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl std::fmt::Display for EventSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debug => write!(f, "debug"),
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// Event category for filtering and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventCategory {
    Orchestrator,
    Goal,
    Task,
    Execution,
    Agent,
    Verification,
    Escalation,
    Memory,
    Scheduler,
    Convergence,
    Workflow,
    Adapter,
}

impl std::fmt::Display for EventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Orchestrator => write!(f, "orchestrator"),
            Self::Goal => write!(f, "goal"),
            Self::Task => write!(f, "task"),
            Self::Execution => write!(f, "execution"),
            Self::Agent => write!(f, "agent"),
            Self::Verification => write!(f, "verification"),
            Self::Escalation => write!(f, "escalation"),
            Self::Memory => write!(f, "memory"),
            Self::Scheduler => write!(f, "scheduler"),
            Self::Convergence => write!(f, "convergence"),
            Self::Workflow => write!(f, "workflow"),
            Self::Adapter => write!(f, "adapter"),
        }
    }
}

/// Unified event envelope containing all event metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedEvent {
    pub id: EventId,
    pub sequence: SequenceNumber,
    pub timestamp: DateTime<Utc>,
    pub severity: EventSeverity,
    pub category: EventCategory,
    pub goal_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    /// Identifies the EventBus process that originally published this event.
    /// Used by EventStorePoller to avoid re-broadcasting events from this process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_process_id: Option<Uuid>,
    pub payload: EventPayload,
}

/// Unified event payload combining all SwarmEvent and ExecutionEvent variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    // Orchestrator lifecycle events
    OrchestratorStarted,
    OrchestratorPaused,
    OrchestratorResumed,
    OrchestratorStopped,
    StatusUpdate(SwarmStatsPayload),

    // Goal events
    GoalStarted {
        goal_id: Uuid,
        goal_name: String,
    },
    GoalDecomposed {
        goal_id: Uuid,
        task_count: usize,
    },
    GoalIterationCompleted {
        goal_id: Uuid,
        tasks_completed: usize,
    },
    GoalPaused {
        goal_id: Uuid,
        reason: String,
    },
    ConvergenceCompleted {
        goal_id: Uuid,
        converged: bool,
        iterations: u32,
        final_satisfaction: String,
    },
    SemanticDriftDetected {
        goal_id: Uuid,
        recurring_gaps: Vec<String>,
        iterations: u32,
    },

    // Task lifecycle events
    TaskSubmitted {
        task_id: Uuid,
        task_title: String,
        goal_id: Uuid,
    },
    TaskReady {
        task_id: Uuid,
        task_title: String,
    },
    TaskSpawned {
        task_id: Uuid,
        task_title: String,
        agent_type: Option<String>,
    },
    TaskStarted {
        task_id: Uuid,
        task_title: String,
    },
    TaskCompleted {
        task_id: Uuid,
        tokens_used: u64,
    },
    TaskCompletedWithResult {
        task_id: Uuid,
        result: TaskResultPayload,
    },
    TaskFailed {
        task_id: Uuid,
        error: String,
        retry_count: u32,
    },
    TaskRetrying {
        task_id: Uuid,
        attempt: u32,
        max_attempts: u32,
    },
    TaskVerified {
        task_id: Uuid,
        passed: bool,
        checks_passed: usize,
        checks_total: usize,
    },
    TaskQueuedForMerge {
        task_id: Uuid,
        stage: String,
    },
    PullRequestCreated {
        task_id: Uuid,
        pr_url: String,
        branch: String,
    },
    TaskMerged {
        task_id: Uuid,
        commit_sha: String,
    },
    WorktreeCreated {
        task_id: Uuid,
        path: String,
    },

    // Execution events
    ExecutionStarted {
        total_tasks: usize,
        wave_count: usize,
    },
    ExecutionCompleted {
        status: ExecutionStatusPayload,
        results: ExecutionResultsPayload,
    },
    WaveStarted {
        wave_number: usize,
        task_count: usize,
    },
    WaveCompleted {
        wave_number: usize,
        succeeded: usize,
        failed: usize,
    },
    RestructureTriggered {
        task_id: Uuid,
        decision: String,
    },
    RestructureDecision {
        task_id: Uuid,
        decision: String,
    },

    /// Review failure loop-back triggered: spawns a new plan → implement → review cycle.
    ReviewLoopTriggered {
        failed_review_task_id: Uuid,
        iteration: u32,
        max_iterations: u32,
        new_plan_task_id: Uuid,
    },

    // Agent events
    AgentCreated {
        agent_type: String,
        tier: String,
    },
    SpecialistSpawned {
        specialist_type: String,
        trigger: String,
        task_id: Option<Uuid>,
    },
    EvolutionTriggered {
        template_name: String,
        trigger: String,
    },
    SpawnLimitExceeded {
        parent_task_id: Uuid,
        limit_type: String,
        current_value: u32,
        limit_value: u32,
    },
    GoalAlignmentEvaluated {
        task_id: Uuid,
        overall_score: f64,
        passes: bool,
    },

    // Verification events
    IntentVerificationStarted {
        goal_id: Uuid,
        iteration: u32,
    },
    IntentVerificationCompleted {
        goal_id: Uuid,
        satisfaction: String,
        confidence: f64,
        gaps_count: usize,
        iteration: u32,
        will_retry: bool,
    },
    IntentVerificationRequested {
        goal_id: Option<Uuid>,
        completed_task_ids: Vec<Uuid>,
    },
    IntentVerificationResult {
        satisfaction: String,
        confidence: f64,
        gaps_count: usize,
        iteration: u32,
        should_continue: bool,
    },
    WaveVerificationRequested {
        wave_number: usize,
        completed_task_ids: Vec<Uuid>,
        goal_id: Option<Uuid>,
    },
    WaveVerificationResult {
        wave_number: usize,
        satisfaction: String,
        confidence: f64,
        gaps_count: usize,
    },
    BranchVerificationStarted {
        branch_task_ids: Vec<Uuid>,
        waiting_task_ids: Vec<Uuid>,
    },
    BranchVerificationRequested {
        branch_task_ids: Vec<Uuid>,
        waiting_task_ids: Vec<Uuid>,
        branch_objective: String,
    },
    BranchVerificationCompleted {
        branch_satisfied: bool,
        dependents_can_proceed: bool,
        gaps_count: usize,
    },
    BranchVerificationResult {
        branch_satisfied: bool,
        confidence: f64,
        gaps_count: usize,
        dependents_can_proceed: bool,
    },

    // Memory events
    MemoryStored {
        memory_id: Uuid,
        key: String,
        namespace: String,
        tier: String,
        memory_type: String,
    },
    MemoryPromoted {
        memory_id: Uuid,
        key: String,
        from_tier: String,
        to_tier: String,
    },
    MemoryPruned {
        count: u64,
        reason: String,
    },
    MemoryAccessed {
        memory_id: Uuid,
        key: String,
        access_count: u32,
        /// String representation of the accessor that triggered this access.
        accessor: String,
        /// Number of distinct accessors that have accessed this memory.
        distinct_accessor_count: u32,
    },

    // Goal status events
    GoalStatusChanged {
        goal_id: Uuid,
        from_status: String,
        to_status: String,
    },
    GoalConstraintViolated {
        goal_id: Uuid,
        constraint_name: String,
        violation: String,
    },

    // Task claim event
    TaskClaimed {
        task_id: Uuid,
        agent_type: String,
    },

    // Task cancellation event
    TaskCanceled {
        task_id: Uuid,
        reason: String,
    },

    // Scheduler events
    ScheduledEventFired {
        schedule_id: Uuid,
        name: String,
    },
    ScheduledEventRegistered {
        schedule_id: Uuid,
        name: String,
        schedule_type: String,
    },
    ScheduledEventCanceled {
        schedule_id: Uuid,
        name: String,
    },

    // Agent events
    AgentInstanceCompleted {
        instance_id: Uuid,
        task_id: Uuid,
        tokens_used: u64,
    },

    // Goal domain/constraint events
    GoalDomainsUpdated {
        goal_id: Uuid,
        old_domains: Vec<String>,
        new_domains: Vec<String>,
    },
    GoalDeleted {
        goal_id: Uuid,
        goal_name: String,
    },
    GoalConstraintsUpdated {
        goal_id: Uuid,
    },

    // Agent template/instance lifecycle events
    AgentTemplateRegistered {
        template_name: String,
        tier: String,
        version: u32,
    },
    AgentTemplateStatusChanged {
        template_name: String,
        from_status: String,
        to_status: String,
    },
    AgentInstanceSpawned {
        instance_id: Uuid,
        template_name: String,
        tier: String,
    },
    AgentInstanceAssigned {
        instance_id: Uuid,
        task_id: Uuid,
        template_name: String,
    },
    AgentInstanceFailed {
        instance_id: Uuid,
        task_id: Option<Uuid>,
        template_name: String,
    },

    // Memory conflict events
    MemoryDeleted {
        memory_id: Uuid,
        key: String,
        namespace: String,
    },
    MemoryConflictDetected {
        memory_a: Uuid,
        memory_b: Uuid,
        key: String,
        similarity: f64,
    },
    MemoryConflictResolved {
        memory_a: Uuid,
        memory_b: Uuid,
        resolution_type: String,
    },

    // Task validation event
    TaskValidating {
        task_id: Uuid,
    },

    // Reconciliation events
    ReconciliationCompleted {
        corrections_made: u32,
    },

    // Escalation events
    HumanEscalationRequired {
        goal_id: Option<Uuid>,
        task_id: Option<Uuid>,
        reason: String,
        urgency: String,
        questions: Vec<String>,
        is_blocking: bool,
    },
    HumanEscalationNeeded {
        goal_id: Option<Uuid>,
        task_id: Option<Uuid>,
        reason: String,
        urgency: String,
        is_blocking: bool,
    },
    HumanResponseReceived {
        escalation_id: Uuid,
        decision: String,
        allows_continuation: bool,
    },

    // Trigger rule lifecycle events
    TriggerRuleCreated {
        rule_id: Uuid,
        rule_name: String,
    },
    TriggerRuleToggled {
        rule_id: Uuid,
        rule_name: String,
        enabled: bool,
    },
    TriggerRuleDeleted {
        rule_id: Uuid,
        rule_name: String,
    },

    // Memory maintenance summary
    MemoryMaintenanceCompleted {
        expired_pruned: u64,
        decayed_pruned: u64,
        promoted: u64,
        conflicts_resolved: u64,
    },

    /// A single memory maintenance cycle failed.
    MemoryMaintenanceFailed {
        run_number: u64,
        error: String,
        consecutive_failures: u32,
        max_consecutive_failures: u32,
    },

    /// Memory daemon is approaching failure threshold (pre-failure alert).
    MemoryDaemonDegraded {
        consecutive_failures: u32,
        max_consecutive_failures: u32,
        latest_error: String,
    },

    /// Memory daemon stopped (requested, too many failures, or channel closed).
    MemoryDaemonStopped {
        reason: String,
    },

    // Handler error events (emitted by reactor for monitoring)
    HandlerError {
        handler_name: String,
        event_sequence: u64,
        error: String,
        circuit_breaker_tripped: bool,
    },

    // Task lifecycle gap events
    TaskDependencyChanged {
        task_id: Uuid,
        added: Vec<Uuid>,
        removed: Vec<Uuid>,
    },
    TaskPriorityChanged {
        task_id: Uuid,
        from: String,
        to: String,
        reason: String,
    },

    // Escalation gap
    HumanEscalationExpired {
        task_id: Option<Uuid>,
        goal_id: Option<Uuid>,
        default_action: String,
    },

    // Worktree gap
    WorktreeDestroyed {
        worktree_id: Uuid,
        task_id: Uuid,
        reason: String,
    },

    // Startup/recovery
    StartupCatchUpCompleted {
        orphaned_tasks_fixed: u32,
        missed_events_replayed: u64,
        goals_reevaluated: u32,
        duration_ms: u64,
    },

    // SLA enforcement events
    TaskSLAWarning {
        task_id: Uuid,
        deadline: String,
        remaining_secs: i64,
    },
    TaskSLACritical {
        task_id: Uuid,
        deadline: String,
        remaining_secs: i64,
    },
    TaskSLABreached {
        task_id: Uuid,
        deadline: String,
        overdue_secs: i64,
    },

    // Stale task warning events
    TaskRunningLong {
        task_id: Uuid,
        runtime_secs: u64,
    },
    TaskRunningCritical {
        task_id: Uuid,
        runtime_secs: u64,
    },

    // Memory-informed goal evaluation
    MemoryInformedGoal {
        goal_id: Uuid,
        memory_id: Uuid,
        memory_key: String,
    },

    // Description update events (for escalation path)
    TaskDescriptionUpdated {
        task_id: Uuid,
        reason: String,
    },
    GoalDescriptionUpdated {
        goal_id: Uuid,
        reason: String,
    },

    // Convergence lifecycle events (task-level convergence integration)

    /// Emitted when convergence loop starts for a task.
    ConvergenceStarted {
        task_id: Uuid,
        trajectory_id: Uuid,
        estimated_iterations: u32,
        basin_width: String,
        convergence_mode: String,
    },

    /// Emitted after each convergence iteration.
    ConvergenceIteration {
        task_id: Uuid,
        trajectory_id: Uuid,
        iteration: u32,
        strategy: String,
        convergence_delta: f64,
        convergence_level: f64,
        attractor_type: String,
        budget_remaining_fraction: f64,
    },

    /// Emitted when attractor classification changes.
    ConvergenceAttractorTransition {
        task_id: Uuid,
        trajectory_id: Uuid,
        from: String,
        to: String,
        confidence: f64,
    },

    /// Emitted when convergence requests a budget extension.
    ConvergenceBudgetExtension {
        task_id: Uuid,
        trajectory_id: Uuid,
        granted: bool,
        additional_iterations: u32,
        additional_tokens: u64,
    },

    /// Emitted when a fresh start is triggered during convergence.
    ConvergenceFreshStart {
        task_id: Uuid,
        trajectory_id: Uuid,
        fresh_start_number: u32,
        reason: String,
    },

    /// Emitted when convergence completes (success or failure).
    ConvergenceTerminated {
        task_id: Uuid,
        trajectory_id: Uuid,
        outcome: String,
        total_iterations: u32,
        total_tokens: u64,
        final_convergence_level: f64,
    },

    /// Emitted on task completion for opportunistic convergence memory recording.
    /// Captures lightweight execution metrics that feed the classification heuristic
    /// dataset (Part 10.3 of convergence-task-integration spec). The actual memory
    /// storage is handled by an event handler, not by TaskService.
    TaskExecutionRecorded {
        task_id: Uuid,
        execution_mode: String,
        complexity: String,
        succeeded: bool,
        tokens_used: u64,
    },

    /// Subtask branch merged into feature branch.
    SubtaskMergedToFeature {
        task_id: Uuid,
        feature_branch: String,
    },

    // Workflow lifecycle events

    /// Workflow execution started.
    WorkflowStarted {
        workflow_instance_id: Uuid,
        workflow_name: String,
        goal_id: Uuid,
        phase_count: usize,
    },

    /// Workflow execution completed.
    WorkflowCompleted {
        workflow_instance_id: Uuid,
        goal_id: Uuid,
        status: String,
        tokens_consumed: u64,
    },

    /// A workflow phase started executing.
    PhaseStarted {
        workflow_instance_id: Uuid,
        phase_id: Uuid,
        phase_name: String,
        task_count: usize,
    },

    /// A workflow phase completed successfully.
    PhaseCompleted {
        workflow_instance_id: Uuid,
        phase_id: Uuid,
        phase_name: String,
    },

    /// A workflow phase failed.
    PhaseFailed {
        workflow_instance_id: Uuid,
        phase_id: Uuid,
        phase_name: String,
        error: String,
    },

    /// Phase verification gate completed.
    PhaseVerificationCompleted {
        workflow_instance_id: Uuid,
        phase_id: Uuid,
        passed: bool,
    },

    /// Phase recovery started (overmind invoked).
    PhaseRecoveryStarted {
        workflow_instance_id: Uuid,
        phase_id: Uuid,
        retry_count: u32,
    },

    // Adapter events

    /// Adapter ingestion poll completed successfully.
    AdapterIngestionCompleted {
        adapter_name: String,
        items_found: usize,
        tasks_created: usize,
    },

    /// Adapter ingestion poll failed.
    AdapterIngestionFailed {
        adapter_name: String,
        error: String,
    },

    /// Adapter egress action completed.
    AdapterEgressCompleted {
        adapter_name: String,
        task_id: Uuid,
        action: String,
        success: bool,
    },

    /// Adapter egress action failed.
    AdapterEgressFailed {
        adapter_name: String,
        task_id: Option<Uuid>,
        error: String,
    },

    /// A task was successfully created from an adapter ingestion item.
    AdapterTaskIngested {
        task_id: Uuid,
        adapter_name: String,
    },
}

impl EventPayload {
    /// Return the discriminant name of this payload variant as a static string.
    /// Used by EventFilter for matching payload types.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::OrchestratorStarted => "OrchestratorStarted",
            Self::OrchestratorPaused => "OrchestratorPaused",
            Self::OrchestratorResumed => "OrchestratorResumed",
            Self::OrchestratorStopped => "OrchestratorStopped",
            Self::StatusUpdate(_) => "StatusUpdate",
            Self::GoalStarted { .. } => "GoalStarted",
            Self::GoalDecomposed { .. } => "GoalDecomposed",
            Self::GoalIterationCompleted { .. } => "GoalIterationCompleted",
            Self::GoalPaused { .. } => "GoalPaused",
            Self::ConvergenceCompleted { .. } => "ConvergenceCompleted",
            Self::SemanticDriftDetected { .. } => "SemanticDriftDetected",
            Self::TaskSubmitted { .. } => "TaskSubmitted",
            Self::TaskReady { .. } => "TaskReady",
            Self::TaskSpawned { .. } => "TaskSpawned",
            Self::TaskStarted { .. } => "TaskStarted",
            Self::TaskCompleted { .. } => "TaskCompleted",
            Self::TaskCompletedWithResult { .. } => "TaskCompletedWithResult",
            Self::TaskFailed { .. } => "TaskFailed",
            Self::TaskRetrying { .. } => "TaskRetrying",
            Self::TaskVerified { .. } => "TaskVerified",
            Self::TaskQueuedForMerge { .. } => "TaskQueuedForMerge",
            Self::PullRequestCreated { .. } => "PullRequestCreated",
            Self::TaskMerged { .. } => "TaskMerged",
            Self::WorktreeCreated { .. } => "WorktreeCreated",
            Self::ExecutionStarted { .. } => "ExecutionStarted",
            Self::ExecutionCompleted { .. } => "ExecutionCompleted",
            Self::WaveStarted { .. } => "WaveStarted",
            Self::WaveCompleted { .. } => "WaveCompleted",
            Self::RestructureTriggered { .. } => "RestructureTriggered",
            Self::RestructureDecision { .. } => "RestructureDecision",
            Self::ReviewLoopTriggered { .. } => "ReviewLoopTriggered",
            Self::AgentCreated { .. } => "AgentCreated",
            Self::SpecialistSpawned { .. } => "SpecialistSpawned",
            Self::EvolutionTriggered { .. } => "EvolutionTriggered",
            Self::SpawnLimitExceeded { .. } => "SpawnLimitExceeded",
            Self::GoalAlignmentEvaluated { .. } => "GoalAlignmentEvaluated",
            Self::IntentVerificationStarted { .. } => "IntentVerificationStarted",
            Self::IntentVerificationCompleted { .. } => "IntentVerificationCompleted",
            Self::IntentVerificationRequested { .. } => "IntentVerificationRequested",
            Self::IntentVerificationResult { .. } => "IntentVerificationResult",
            Self::WaveVerificationRequested { .. } => "WaveVerificationRequested",
            Self::WaveVerificationResult { .. } => "WaveVerificationResult",
            Self::BranchVerificationStarted { .. } => "BranchVerificationStarted",
            Self::BranchVerificationRequested { .. } => "BranchVerificationRequested",
            Self::BranchVerificationCompleted { .. } => "BranchVerificationCompleted",
            Self::BranchVerificationResult { .. } => "BranchVerificationResult",
            Self::MemoryStored { .. } => "MemoryStored",
            Self::MemoryPromoted { .. } => "MemoryPromoted",
            Self::MemoryPruned { .. } => "MemoryPruned",
            Self::MemoryAccessed { .. } => "MemoryAccessed",
            Self::GoalStatusChanged { .. } => "GoalStatusChanged",
            Self::GoalConstraintViolated { .. } => "GoalConstraintViolated",
            Self::TaskClaimed { .. } => "TaskClaimed",
            Self::TaskCanceled { .. } => "TaskCanceled",
            Self::ScheduledEventFired { .. } => "ScheduledEventFired",
            Self::ScheduledEventRegistered { .. } => "ScheduledEventRegistered",
            Self::ScheduledEventCanceled { .. } => "ScheduledEventCanceled",
            Self::AgentInstanceCompleted { .. } => "AgentInstanceCompleted",
            Self::GoalDomainsUpdated { .. } => "GoalDomainsUpdated",
            Self::GoalDeleted { .. } => "GoalDeleted",
            Self::GoalConstraintsUpdated { .. } => "GoalConstraintsUpdated",
            Self::AgentTemplateRegistered { .. } => "AgentTemplateRegistered",
            Self::AgentTemplateStatusChanged { .. } => "AgentTemplateStatusChanged",
            Self::AgentInstanceSpawned { .. } => "AgentInstanceSpawned",
            Self::AgentInstanceAssigned { .. } => "AgentInstanceAssigned",
            Self::AgentInstanceFailed { .. } => "AgentInstanceFailed",
            Self::MemoryDeleted { .. } => "MemoryDeleted",
            Self::MemoryConflictDetected { .. } => "MemoryConflictDetected",
            Self::MemoryConflictResolved { .. } => "MemoryConflictResolved",
            Self::TaskValidating { .. } => "TaskValidating",
            Self::ReconciliationCompleted { .. } => "ReconciliationCompleted",
            Self::HumanEscalationRequired { .. } => "HumanEscalationRequired",
            Self::HumanEscalationNeeded { .. } => "HumanEscalationNeeded",
            Self::HumanResponseReceived { .. } => "HumanResponseReceived",
            Self::TriggerRuleCreated { .. } => "TriggerRuleCreated",
            Self::TriggerRuleToggled { .. } => "TriggerRuleToggled",
            Self::TriggerRuleDeleted { .. } => "TriggerRuleDeleted",
            Self::MemoryMaintenanceCompleted { .. } => "MemoryMaintenanceCompleted",
            Self::MemoryMaintenanceFailed { .. } => "MemoryMaintenanceFailed",
            Self::MemoryDaemonDegraded { .. } => "MemoryDaemonDegraded",
            Self::MemoryDaemonStopped { .. } => "MemoryDaemonStopped",
            Self::HandlerError { .. } => "HandlerError",
            Self::TaskDependencyChanged { .. } => "TaskDependencyChanged",
            Self::TaskPriorityChanged { .. } => "TaskPriorityChanged",
            Self::HumanEscalationExpired { .. } => "HumanEscalationExpired",
            Self::WorktreeDestroyed { .. } => "WorktreeDestroyed",
            Self::StartupCatchUpCompleted { .. } => "StartupCatchUpCompleted",
            Self::TaskSLAWarning { .. } => "TaskSLAWarning",
            Self::TaskSLACritical { .. } => "TaskSLACritical",
            Self::TaskSLABreached { .. } => "TaskSLABreached",
            Self::TaskRunningLong { .. } => "TaskRunningLong",
            Self::TaskRunningCritical { .. } => "TaskRunningCritical",
            Self::MemoryInformedGoal { .. } => "MemoryInformedGoal",
            Self::TaskDescriptionUpdated { .. } => "TaskDescriptionUpdated",
            Self::GoalDescriptionUpdated { .. } => "GoalDescriptionUpdated",
            Self::ConvergenceStarted { .. } => "ConvergenceStarted",
            Self::ConvergenceIteration { .. } => "ConvergenceIteration",
            Self::ConvergenceAttractorTransition { .. } => "ConvergenceAttractorTransition",
            Self::ConvergenceBudgetExtension { .. } => "ConvergenceBudgetExtension",
            Self::ConvergenceFreshStart { .. } => "ConvergenceFreshStart",
            Self::ConvergenceTerminated { .. } => "ConvergenceTerminated",
            Self::TaskExecutionRecorded { .. } => "TaskExecutionRecorded",
            Self::SubtaskMergedToFeature { .. } => "SubtaskMergedToFeature",
            Self::WorkflowStarted { .. } => "WorkflowStarted",
            Self::WorkflowCompleted { .. } => "WorkflowCompleted",
            Self::PhaseStarted { .. } => "PhaseStarted",
            Self::PhaseCompleted { .. } => "PhaseCompleted",
            Self::PhaseFailed { .. } => "PhaseFailed",
            Self::PhaseVerificationCompleted { .. } => "PhaseVerificationCompleted",
            Self::PhaseRecoveryStarted { .. } => "PhaseRecoveryStarted",
            Self::AdapterIngestionCompleted { .. } => "AdapterIngestionCompleted",
            Self::AdapterIngestionFailed { .. } => "AdapterIngestionFailed",
            Self::AdapterEgressCompleted { .. } => "AdapterEgressCompleted",
            Self::AdapterEgressFailed { .. } => "AdapterEgressFailed",
            Self::AdapterTaskIngested { .. } => "AdapterTaskIngested",
        }
    }
}

/// Serializable version of SwarmStats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmStatsPayload {
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

impl From<SwarmStats> for SwarmStatsPayload {
    fn from(stats: SwarmStats) -> Self {
        Self {
            active_goals: stats.active_goals,
            pending_tasks: stats.pending_tasks,
            ready_tasks: stats.ready_tasks,
            running_tasks: stats.running_tasks,
            completed_tasks: stats.completed_tasks,
            failed_tasks: stats.failed_tasks,
            active_agents: stats.active_agents,
            active_worktrees: stats.active_worktrees,
            total_tokens_used: stats.total_tokens_used,
        }
    }
}

/// Serializable version of TaskResult (without SubstrateSession which is not serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultPayload {
    pub task_id: Uuid,
    pub status: String,
    pub error: Option<String>,
    pub duration_secs: u64,
    pub retry_count: u32,
    pub tokens_used: u64,
}

impl From<TaskResult> for TaskResultPayload {
    fn from(result: TaskResult) -> Self {
        Self {
            task_id: result.task_id,
            status: format!("{:?}", result.status),
            error: result.error,
            duration_secs: result.duration_secs,
            retry_count: result.retry_count,
            tokens_used: result.session.as_ref().map(|s| s.total_tokens()).unwrap_or(0),
        }
    }
}

/// Serializable version of ExecutionStatus.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatusPayload {
    Pending,
    Running,
    Completed,
    PartialSuccess,
    Failed,
    Canceled,
}

impl From<ExecutionStatus> for ExecutionStatusPayload {
    fn from(status: ExecutionStatus) -> Self {
        match status {
            ExecutionStatus::Pending => Self::Pending,
            ExecutionStatus::Running => Self::Running,
            ExecutionStatus::Completed => Self::Completed,
            ExecutionStatus::PartialSuccess => Self::PartialSuccess,
            ExecutionStatus::Failed => Self::Failed,
            ExecutionStatus::Canceled => Self::Canceled,
        }
    }
}

/// Serializable version of ExecutionResults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResultsPayload {
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub skipped_tasks: usize,
    pub total_duration_secs: u64,
    pub total_tokens_used: u64,
}

impl From<ExecutionResults> for ExecutionResultsPayload {
    fn from(results: ExecutionResults) -> Self {
        Self {
            total_tasks: results.total_tasks,
            completed_tasks: results.completed_tasks,
            failed_tasks: results.failed_tasks,
            skipped_tasks: results.skipped_tasks,
            total_duration_secs: results.total_duration_secs,
            total_tokens_used: results.total_tokens_used,
        }
    }
}

/// Convert SwarmEvent to UnifiedEvent.
impl From<SwarmEvent> for UnifiedEvent {
    fn from(event: SwarmEvent) -> Self {
        let (severity, category, goal_id, task_id, payload) = match event {
            SwarmEvent::Started => (
                EventSeverity::Info,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::OrchestratorStarted,
            ),
            SwarmEvent::Paused => (
                EventSeverity::Info,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::OrchestratorPaused,
            ),
            SwarmEvent::Resumed => (
                EventSeverity::Info,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::OrchestratorResumed,
            ),
            SwarmEvent::Stopped => (
                EventSeverity::Info,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::OrchestratorStopped,
            ),
            SwarmEvent::StatusUpdate(stats) => (
                EventSeverity::Debug,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::StatusUpdate(stats.into()),
            ),
            SwarmEvent::GoalStarted { goal_id, goal_name } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalStarted { goal_id, goal_name },
            ),
            SwarmEvent::GoalDecomposed { goal_id, task_count } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalDecomposed { goal_id, task_count },
            ),
            SwarmEvent::GoalIterationCompleted { goal_id, tasks_completed } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalIterationCompleted { goal_id, tasks_completed },
            ),
            SwarmEvent::GoalPaused { goal_id, reason } => (
                EventSeverity::Warning,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalPaused { goal_id, reason },
            ),
            SwarmEvent::ConvergenceCompleted { goal_id, converged, iterations, final_satisfaction } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::ConvergenceCompleted { goal_id, converged, iterations, final_satisfaction },
            ),
            SwarmEvent::SemanticDriftDetected { goal_id, recurring_gaps, iterations } => (
                EventSeverity::Warning,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::SemanticDriftDetected { goal_id, recurring_gaps, iterations },
            ),
            SwarmEvent::TaskSubmitted { task_id, task_title, goal_id } => (
                EventSeverity::Info,
                EventCategory::Task,
                Some(goal_id),
                Some(task_id),
                EventPayload::TaskSubmitted { task_id, task_title, goal_id },
            ),
            SwarmEvent::TaskReady { task_id, task_title } => (
                EventSeverity::Debug,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskReady { task_id, task_title },
            ),
            SwarmEvent::TaskSpawned { task_id, task_title, agent_type } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskSpawned { task_id, task_title, agent_type },
            ),
            SwarmEvent::TaskCompleted { task_id, tokens_used } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskCompleted { task_id, tokens_used },
            ),
            SwarmEvent::TaskFailed { task_id, error, retry_count } => (
                EventSeverity::Error,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskFailed { task_id, error, retry_count },
            ),
            SwarmEvent::TaskRetrying { task_id, attempt, max_attempts } => (
                EventSeverity::Warning,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskRetrying { task_id, attempt, max_attempts },
            ),
            SwarmEvent::TaskVerified { task_id, passed, checks_passed, checks_total, .. } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                Some(task_id),
                EventPayload::TaskVerified { task_id, passed, checks_passed, checks_total },
            ),
            SwarmEvent::TaskQueuedForMerge { task_id, stage } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskQueuedForMerge { task_id, stage },
            ),
            SwarmEvent::PullRequestCreated { task_id, pr_url, branch } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::PullRequestCreated { task_id, pr_url, branch },
            ),
            SwarmEvent::TaskMerged { task_id, commit_sha } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskMerged { task_id, commit_sha },
            ),
            SwarmEvent::WorktreeCreated { task_id, path } => (
                EventSeverity::Debug,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::WorktreeCreated { task_id, path },
            ),
            SwarmEvent::TaskClaimed { task_id, agent_type } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskClaimed { task_id, agent_type },
            ),
            SwarmEvent::AgentInstanceCompleted { instance_id, task_id, tokens_used } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                Some(task_id),
                EventPayload::AgentInstanceCompleted { instance_id, task_id, tokens_used },
            ),
            SwarmEvent::ReconciliationCompleted { corrections_made } => (
                EventSeverity::Debug,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::ReconciliationCompleted { corrections_made },
            ),
            SwarmEvent::EvolutionTriggered { template_name, trigger } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                None,
                EventPayload::EvolutionTriggered { template_name, trigger },
            ),
            SwarmEvent::SpecialistSpawned { specialist_type, trigger, task_id } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                task_id,
                EventPayload::SpecialistSpawned { specialist_type, trigger, task_id },
            ),
            SwarmEvent::AgentCreated { agent_type, tier } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                None,
                EventPayload::AgentCreated { agent_type, tier },
            ),
            SwarmEvent::GoalAlignmentEvaluated { task_id, overall_score, passes } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                Some(task_id),
                EventPayload::GoalAlignmentEvaluated { task_id, overall_score, passes },
            ),
            SwarmEvent::RestructureTriggered { task_id, decision } => (
                EventSeverity::Warning,
                EventCategory::Execution,
                None,
                Some(task_id),
                EventPayload::RestructureTriggered { task_id, decision },
            ),
            SwarmEvent::SpawnLimitExceeded { parent_task_id, limit_type, current_value, limit_value } => (
                EventSeverity::Warning,
                EventCategory::Agent,
                None,
                Some(parent_task_id),
                EventPayload::SpawnLimitExceeded { parent_task_id, limit_type, current_value, limit_value },
            ),
            SwarmEvent::IntentVerificationStarted { goal_id, iteration } => (
                EventSeverity::Info,
                EventCategory::Verification,
                Some(goal_id),
                None,
                EventPayload::IntentVerificationStarted { goal_id, iteration },
            ),
            SwarmEvent::IntentVerificationCompleted { goal_id, satisfaction, confidence, gaps_count, iteration, will_retry } => (
                EventSeverity::Info,
                EventCategory::Verification,
                Some(goal_id),
                None,
                EventPayload::IntentVerificationCompleted { goal_id, satisfaction, confidence, gaps_count, iteration, will_retry },
            ),
            SwarmEvent::BranchVerificationStarted { branch_task_ids, waiting_task_ids } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationStarted { branch_task_ids, waiting_task_ids },
            ),
            SwarmEvent::BranchVerificationCompleted { branch_satisfied, dependents_can_proceed, gaps_count } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationCompleted { branch_satisfied, dependents_can_proceed, gaps_count },
            ),
            SwarmEvent::HumanEscalationRequired { goal_id, task_id, reason, urgency, questions, is_blocking } => (
                if is_blocking { EventSeverity::Critical } else { EventSeverity::Warning },
                EventCategory::Escalation,
                goal_id,
                task_id,
                EventPayload::HumanEscalationRequired { goal_id, task_id, reason, urgency, questions, is_blocking },
            ),
            SwarmEvent::HumanResponseReceived { escalation_id, decision, allows_continuation } => (
                EventSeverity::Info,
                EventCategory::Escalation,
                None,
                None,
                EventPayload::HumanResponseReceived { escalation_id, decision, allows_continuation },
            ),
            SwarmEvent::SubtaskMergedToFeature { task_id, feature_branch } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::SubtaskMergedToFeature { task_id, feature_branch },
            ),
        };

        Self {
            id: EventId::new(),
            sequence: SequenceNumber::zero(), // Will be assigned by EventBus
            timestamp: Utc::now(),
            severity,
            category,
            goal_id,
            task_id,
            correlation_id: None,
            source_process_id: None, // Will be stamped by EventBus on publish
            payload,
        }
    }
}

/// Convert ExecutionEvent to UnifiedEvent.
impl From<ExecutionEvent> for UnifiedEvent {
    fn from(event: ExecutionEvent) -> Self {
        let (severity, category, goal_id, task_id, payload) = match event {
            ExecutionEvent::Started { total_tasks, wave_count } => (
                EventSeverity::Info,
                EventCategory::Execution,
                None,
                None,
                EventPayload::ExecutionStarted { total_tasks, wave_count },
            ),
            ExecutionEvent::Completed { status, results } => (
                if matches!(status, ExecutionStatus::Failed) { EventSeverity::Error } else { EventSeverity::Info },
                EventCategory::Execution,
                None,
                None,
                EventPayload::ExecutionCompleted {
                    status: status.into(),
                    results: results.into(),
                },
            ),
            ExecutionEvent::WaveStarted { wave_number, task_count } => (
                EventSeverity::Info,
                EventCategory::Execution,
                None,
                None,
                EventPayload::WaveStarted { wave_number, task_count },
            ),
            ExecutionEvent::WaveCompleted { wave_number, succeeded, failed } => (
                if failed > 0 { EventSeverity::Warning } else { EventSeverity::Info },
                EventCategory::Execution,
                None,
                None,
                EventPayload::WaveCompleted { wave_number, succeeded, failed },
            ),
            ExecutionEvent::TaskStarted { task_id, task_title } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskStarted { task_id, task_title },
            ),
            ExecutionEvent::TaskCompleted { task_id, result } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskCompletedWithResult { task_id, result: result.into() },
            ),
            ExecutionEvent::TaskFailed { task_id, error, retry_count } => (
                EventSeverity::Error,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskFailed { task_id, error, retry_count },
            ),
            ExecutionEvent::TaskRetrying { task_id, attempt, max_attempts } => (
                EventSeverity::Warning,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskRetrying { task_id, attempt, max_attempts },
            ),
            ExecutionEvent::RestructureDecision { task_id, decision } => (
                EventSeverity::Warning,
                EventCategory::Execution,
                None,
                Some(task_id),
                EventPayload::RestructureDecision { task_id, decision },
            ),
            ExecutionEvent::IntentVerificationRequested { goal_id, completed_task_ids } => (
                EventSeverity::Info,
                EventCategory::Verification,
                goal_id,
                None,
                EventPayload::IntentVerificationRequested { goal_id, completed_task_ids },
            ),
            ExecutionEvent::IntentVerificationResult { satisfaction, confidence, gaps_count, iteration, should_continue } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::IntentVerificationResult { satisfaction, confidence, gaps_count, iteration, should_continue },
            ),
            ExecutionEvent::WaveVerificationRequested { wave_number, completed_task_ids, goal_id } => (
                EventSeverity::Info,
                EventCategory::Verification,
                goal_id,
                None,
                EventPayload::WaveVerificationRequested { wave_number, completed_task_ids, goal_id },
            ),
            ExecutionEvent::WaveVerificationResult { wave_number, satisfaction, confidence, gaps_count } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::WaveVerificationResult { wave_number, satisfaction, confidence, gaps_count },
            ),
            ExecutionEvent::BranchVerificationRequested { branch_task_ids, waiting_task_ids, branch_objective } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationRequested { branch_task_ids, waiting_task_ids, branch_objective },
            ),
            ExecutionEvent::BranchVerificationResult { branch_satisfied, confidence, gaps_count, dependents_can_proceed } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationResult { branch_satisfied, confidence, gaps_count, dependents_can_proceed },
            ),
            ExecutionEvent::HumanEscalationNeeded { goal_id, task_id, reason, urgency, is_blocking } => (
                if is_blocking { EventSeverity::Critical } else { EventSeverity::Warning },
                EventCategory::Escalation,
                goal_id,
                task_id,
                EventPayload::HumanEscalationNeeded { goal_id, task_id, reason, urgency, is_blocking },
            ),
        };

        Self {
            id: EventId::new(),
            sequence: SequenceNumber::zero(), // Will be assigned by EventBus
            timestamp: Utc::now(),
            severity,
            category,
            goal_id,
            task_id,
            correlation_id: None,
            source_process_id: None, // Will be stamped by EventBus on publish
            payload,
        }
    }
}

/// Configuration for the EventBus.
#[derive(Debug, Clone)]
pub struct EventBusConfig {
    /// Channel capacity for the broadcast channel.
    pub channel_capacity: usize,
    /// Whether to persist events to storage.
    pub persist_events: bool,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1024,
            persist_events: true,
        }
    }
}

/// Central event bus for broadcasting events to multiple consumers.
pub struct EventBus {
    sender: broadcast::Sender<UnifiedEvent>,
    sequence: AtomicU64,
    store: Option<Arc<dyn EventStore>>,
    correlation_context: Arc<RwLock<Option<Uuid>>>,
    config: EventBusConfig,
    /// Unique ID for this EventBus instance (process). Used to identify
    /// events originating from this process for cross-process dedup.
    process_id: Uuid,
}

impl EventBus {
    /// Create a new EventBus with the given configuration.
    pub fn new(config: EventBusConfig) -> Self {
        let (sender, _) = broadcast::channel(config.channel_capacity);
        Self {
            sender,
            sequence: AtomicU64::new(0),
            store: None,
            correlation_context: Arc::new(RwLock::new(None)),
            config,
            process_id: Uuid::new_v4(),
        }
    }

    /// Add an event store for persistence.
    pub fn with_store(mut self, store: Arc<dyn EventStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Publish a unified event.
    pub async fn publish(&self, mut event: UnifiedEvent) {
        // Assign sequence number
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        event.sequence = SequenceNumber(seq);

        // Stamp with this process's ID if not already set
        if event.source_process_id.is_none() {
            event.source_process_id = Some(self.process_id);
        }

        // Add correlation ID from context if not set
        if event.correlation_id.is_none() {
            let ctx = self.correlation_context.read().await;
            event.correlation_id = *ctx;
        }

        // Persist if store is configured
        if self.config.persist_events {
            if let Some(ref store) = self.store {
                if let Err(e) = store.append(&event).await {
                    let err_msg = e.to_string();
                    if err_msg.contains("UNIQUE constraint failed: events.sequence") {
                        // Sequence collision with another process — re-sync counter and retry
                        if let Ok(Some(latest)) = store.latest_sequence().await {
                            let new_seq = latest.0 + 1;
                            self.sequence.store(new_seq + 1, Ordering::SeqCst);
                            event.sequence = SequenceNumber(new_seq);
                            if let Err(e2) = store.append(&event).await {
                                tracing::warn!("Failed to persist event after sequence re-sync: {}", e2);
                            }
                        }
                    } else {
                        tracing::warn!("Failed to persist event: {}", e);
                    }
                }
            }
        }

        // Broadcast to subscribers (ignore send errors - may have no subscribers)
        let _ = self.sender.send(event);
    }

    /// Publish a SwarmEvent (converts to UnifiedEvent).
    ///
    /// **Deprecated**: Prefer constructing `UnifiedEvent` directly using
    /// `event_factory::make_event()` or dispatching through the `CommandBus`.
    #[deprecated(note = "Use event_factory::make_event() or dispatch through CommandBus")]
    pub async fn publish_swarm_event(&self, event: SwarmEvent) {
        self.publish(event.into()).await;
    }

    /// Publish an ExecutionEvent (converts to UnifiedEvent).
    ///
    /// **Deprecated**: Prefer constructing `UnifiedEvent` directly using
    /// `event_factory::make_event()` or dispatching through the `CommandBus`.
    #[deprecated(note = "Use event_factory::make_event() or dispatch through CommandBus")]
    pub async fn publish_execution_event(&self, event: ExecutionEvent) {
        self.publish(event.into()).await;
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<UnifiedEvent> {
        self.sender.subscribe()
    }

    /// Get the current sequence number.
    pub fn current_sequence(&self) -> SequenceNumber {
        SequenceNumber(self.sequence.load(Ordering::SeqCst))
    }

    /// Start a new correlation context for tracking related events.
    pub async fn start_correlation(&self) -> Uuid {
        let id = Uuid::new_v4();
        let mut ctx = self.correlation_context.write().await;
        *ctx = Some(id);
        id
    }

    /// End the current correlation context.
    pub async fn end_correlation(&self) {
        let mut ctx = self.correlation_context.write().await;
        *ctx = None;
    }

    /// Get the event store if configured.
    pub fn store(&self) -> Option<Arc<dyn EventStore>> {
        self.store.clone()
    }

    /// Get the unique process ID of this EventBus instance.
    pub fn process_id(&self) -> Uuid {
        self.process_id
    }

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Initialize the sequence counter from the event store.
    ///
    /// Reads the latest sequence number from the store and sets the atomic
    /// counter to `latest + 1` to prevent sequence overlap after restart.
    /// Must be called during startup before reactor/scheduler start.
    pub async fn initialize_sequence_from_store(&self) {
        if let Some(ref store) = self.store {
            match store.latest_sequence().await {
                Ok(Some(latest)) => {
                    self.sequence.store(latest.0 + 1, Ordering::SeqCst);
                    tracing::info!("EventBus: initialized sequence from store at {}", latest.0 + 1);
                }
                Ok(None) => {
                    // Empty store, start from 0
                }
                Err(e) => {
                    tracing::warn!("EventBus: failed to read latest sequence from store: {}", e);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_bus_sequence_assignment() {
        let bus = EventBus::new(EventBusConfig::default());

        assert_eq!(bus.current_sequence().0, 0);

        let mut rx = bus.subscribe();

        bus.publish_swarm_event(SwarmEvent::Started).await;
        let event1 = rx.recv().await.unwrap();
        assert_eq!(event1.sequence.0, 0);

        bus.publish_swarm_event(SwarmEvent::Stopped).await;
        let event2 = rx.recv().await.unwrap();
        assert_eq!(event2.sequence.0, 1);

        assert_eq!(bus.current_sequence().0, 2);
    }

    #[tokio::test]
    async fn test_event_bus_correlation() {
        let bus = EventBus::new(EventBusConfig::default());
        let mut rx = bus.subscribe();

        // Event without correlation
        bus.publish_swarm_event(SwarmEvent::Started).await;
        let event1 = rx.recv().await.unwrap();
        assert!(event1.correlation_id.is_none());

        // Start correlation
        let corr_id = bus.start_correlation().await;

        // Event with correlation
        bus.publish_swarm_event(SwarmEvent::Paused).await;
        let event2 = rx.recv().await.unwrap();
        assert_eq!(event2.correlation_id, Some(corr_id));

        // End correlation
        bus.end_correlation().await;

        // Event without correlation again
        bus.publish_swarm_event(SwarmEvent::Stopped).await;
        let event3 = rx.recv().await.unwrap();
        assert!(event3.correlation_id.is_none());
    }

    #[tokio::test]
    async fn test_swarm_event_conversion() {
        let event = SwarmEvent::TaskFailed {
            task_id: Uuid::new_v4(),
            error: "test error".to_string(),
            retry_count: 2,
        };

        let unified: UnifiedEvent = event.into();
        assert_eq!(unified.severity, EventSeverity::Error);
        assert_eq!(unified.category, EventCategory::Task);
        assert!(unified.task_id.is_some());
    }

    #[tokio::test]
    async fn test_execution_event_conversion() {
        let event = ExecutionEvent::WaveStarted {
            wave_number: 1,
            task_count: 5,
        };

        let unified: UnifiedEvent = event.into();
        assert_eq!(unified.severity, EventSeverity::Info);
        assert_eq!(unified.category, EventCategory::Execution);
    }
}

//! The `EventPayload` enum and its named per-variant payload structs.
//!
//! `EventPayload` is a large flat enum (~175 variants). Splitting it into
//! domain-grouped sub-enums is deliberately out of scope — it would touch
//! ~500 match arms across the codebase.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::conversions::{
    ExecutionResultsPayload, ExecutionStatusPayload, SwarmStatsPayload, TaskResultPayload,
};
use super::types::{BudgetPressureLevel, EventCategory};

// ============================================================================
// Named payload structs for variants with 6+ fields
//
// Variants wrapping these structs keep match arms readable while reducing the
// worst-case variant girth. See the enum definition below for usage.
// ============================================================================

/// Payload for `EventPayload::IntentVerificationCompleted`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentVerificationCompletedPayload {
    pub goal_id: Uuid,
    pub satisfaction: String,
    pub confidence: f64,
    pub gaps_count: usize,
    pub iteration: u32,
    pub will_retry: bool,
}

/// Payload for `HumanEscalationRequired` and `HumanEscalationNeeded`.
///
/// Both variants have identical shape — Required indicates a new escalation
/// produced by a monitor; Needed is the execution-side form emitted from the
/// DAG executor. The fields are the same in either case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanEscalationPayload {
    pub goal_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub reason: String,
    pub urgency: String,
    pub questions: Vec<String>,
    pub is_blocking: bool,
}

/// Payload for `EventPayload::ConvergenceIteration`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceIterationPayload {
    pub task_id: Uuid,
    pub trajectory_id: Uuid,
    pub iteration: u32,
    pub strategy: String,
    pub convergence_delta: f64,
    pub convergence_level: f64,
    pub attractor_type: String,
    pub budget_remaining_fraction: f64,
}

/// Payload for `EventPayload::ConvergenceTerminated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceTerminatedPayload {
    pub task_id: Uuid,
    pub trajectory_id: Uuid,
    pub outcome: String,
    pub total_iterations: u32,
    pub total_tokens: u64,
    pub final_convergence_level: f64,
}

/// Payload for `EventPayload::WorkflowVerificationCompleted`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVerificationCompletedPayload {
    pub task_id: Uuid,
    pub phase_index: usize,
    pub phase_name: String,
    pub satisfied: bool,
    pub retry_count: u32,
    pub summary: String,
}

/// Unified event payload combining all SwarmEvent and ExecutionEvent variants.
///
/// Variants are organized below into sections by [`EventCategory`] with
/// banner comments. Variants with 6+ named fields are wrapped in the `…Payload`
/// structs defined above.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    // ========================================================================
    // Orchestrator — lifecycle, reconciliation, handler infra, trigger rules,
    //                subsystem errors
    // ========================================================================
    OrchestratorStarted,
    OrchestratorPaused,
    OrchestratorResumed,
    OrchestratorStopped,
    StatusUpdate(SwarmStatsPayload),

    ReconciliationCompleted {
        corrections_made: u32,
    },

    StartupCatchUpCompleted {
        orphaned_tasks_fixed: u32,
        missed_events_replayed: u64,
        goals_reevaluated: u32,
        duration_ms: u64,
    },

    /// Handler error events (emitted by reactor for monitoring).
    HandlerError {
        handler_name: String,
        event_sequence: u64,
        error: String,
        circuit_breaker_tripped: bool,
    },

    /// Emitted when a critical handler's circuit breaker trips.
    /// Critical handlers are essential for system invariants (e.g. task lifecycle
    /// transitions) and use aggressive retry with exponential backoff.
    CriticalHandlerDegraded {
        handler_name: String,
        error: String,
        failure_count: u32,
        backoff_attempt: u32,
    },

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

    /// A subsystem encountered an error that was isolated (not propagated).
    /// Emitted when the main loop catches a subsystem failure to ensure
    /// graceful degradation — no single subsystem failure halts the swarm.
    SubsystemError {
        subsystem: String,
        error: String,
    },

    // ========================================================================
    // Goal — goal lifecycle, status changes, constraints, domains, descriptions
    // ========================================================================
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
    GoalDescriptionUpdated {
        goal_id: Uuid,
        reason: String,
    },

    /// Emitted when a memory record influences a goal evaluation.
    MemoryInformedGoal {
        goal_id: Uuid,
        memory_id: Uuid,
        memory_key: String,
    },

    // ========================================================================
    // Task — lifecycle, SLAs, stale-task warnings, worktrees, PRs, merges,
    //        TaskExecutionRecorded
    // ========================================================================
    TaskSubmitted {
        task_id: Uuid,
        task_title: String,
        goal_id: Option<Uuid>,
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
    TaskQueuedForMerge {
        task_id: Uuid,
        stage: String,
    },
    TaskMerged {
        task_id: Uuid,
        commit_sha: String,
    },
    TaskClaimed {
        task_id: Uuid,
        agent_type: String,
    },
    TaskCanceled {
        task_id: Uuid,
        reason: String,
    },
    TaskValidating {
        task_id: Uuid,
    },
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
    TaskRunningLong {
        task_id: Uuid,
        runtime_secs: u64,
    },
    TaskRunningCritical {
        task_id: Uuid,
        runtime_secs: u64,
    },
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
    TaskDescriptionUpdated {
        task_id: Uuid,
        reason: String,
    },
    WorktreeCreated {
        task_id: Uuid,
        path: String,
    },
    WorktreeDestroyed {
        worktree_id: Uuid,
        task_id: Uuid,
        reason: String,
    },
    PullRequestCreated {
        task_id: Uuid,
        pr_url: String,
        branch: String,
    },
    SubtaskMergedToFeature {
        task_id: Uuid,
        feature_branch: String,
    },

    /// Review failure loop-back triggered: spawns a new plan → implement → review cycle.
    ReviewLoopTriggered {
        failed_review_task_id: Uuid,
        iteration: u32,
        max_iterations: u32,
        new_plan_task_id: Uuid,
        /// The new review task ID, stored as successor on the failed review task
        /// so the parent orchestrating agent can follow the chain.
        new_review_task_id: Uuid,
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

    // ========================================================================
    // Execution — DAG execution lifecycle, waves, restructure decisions
    // ========================================================================
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

    // ========================================================================
    // Agent — agent creation, templates, instance lifecycle, specialist spawns,
    //         evolution
    // ========================================================================
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
    AgentInstanceCompleted {
        instance_id: Uuid,
        task_id: Uuid,
        tokens_used: u64,
    },
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

    // ========================================================================
    // Verification — intent/wave/branch verification, task alignment, verified
    // ========================================================================
    IntentVerificationStarted {
        goal_id: Uuid,
        iteration: u32,
    },
    IntentVerificationCompleted(IntentVerificationCompletedPayload),
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
    GoalAlignmentEvaluated {
        task_id: Uuid,
        overall_score: f64,
        passes: bool,
    },
    TaskVerified {
        task_id: Uuid,
        passed: bool,
        checks_passed: usize,
        checks_total: usize,
        failures_summary: Option<String>,
    },

    // ========================================================================
    // Escalation — human escalation and responses (blocking and non-blocking)
    // ========================================================================
    HumanEscalationRequired(HumanEscalationPayload),
    HumanEscalationNeeded(HumanEscalationPayload),
    HumanResponseReceived {
        escalation_id: Uuid,
        decision: String,
        allows_continuation: bool,
    },
    HumanEscalationExpired {
        task_id: Option<Uuid>,
        goal_id: Option<Uuid>,
        default_action: String,
    },

    // ========================================================================
    // Memory — memory CRUD, conflicts, pruning, daemon maintenance
    // ========================================================================
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

    // ========================================================================
    // Scheduler — scheduled events, quiet-window enter/exit
    // ========================================================================
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

    /// Emitted when the swarm enters a quiet window — dispatch is suppressed.
    QuietWindowEntered {
        window_id: Uuid,
        window_name: String,
    },

    /// Emitted when the swarm exits a quiet window — dispatch resumes.
    QuietWindowExited {
        window_id: Uuid,
        window_name: String,
    },

    // ========================================================================
    // Convergence — iteration, attractor transitions, fresh starts, termination
    // ========================================================================
    /// Emitted when convergence loop starts for a task.
    ConvergenceStarted {
        task_id: Uuid,
        trajectory_id: Uuid,
        estimated_iterations: u32,
        basin_width: String,
        convergence_mode: String,
    },

    /// Emitted after each convergence iteration.
    ConvergenceIteration(ConvergenceIterationPayload),

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
    ConvergenceTerminated(ConvergenceTerminatedPayload),

    // ========================================================================
    // Workflow — state machine phases, gates, retries
    // ========================================================================
    /// Task enrolled in a workflow.
    WorkflowEnrolled {
        task_id: Uuid,
        workflow_name: String,
    },

    /// Workflow phase started with subtask(s).
    WorkflowPhaseStarted {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
        subtask_ids: Vec<Uuid>,
    },

    /// Gate phase reached — awaiting overmind verdict.
    WorkflowGateReached {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
    },

    /// Gate verdict provided by overmind.
    WorkflowGateVerdict {
        task_id: Uuid,
        phase_index: usize,
        verdict: String,
    },

    /// A gate phase rejected the task (triage, validation, or review).
    WorkflowGateRejected {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
        reason: String,
    },

    /// Workflow advanced from one phase to another.
    WorkflowAdvanced {
        task_id: Uuid,
        from_phase: usize,
        to_phase: usize,
    },

    /// Non-gate phase completed; overmind must call workflow_advance or workflow_fan_out.
    WorkflowPhaseReady {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
    },

    /// All workflow phases completed.
    WorkflowCompleted {
        task_id: Uuid,
    },

    /// Workflow phase verification requested — engine is in Verifying state.
    WorkflowVerificationRequested {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
        retry_count: u32,
    },

    /// Workflow phase verification completed — satisfied or failed.
    WorkflowVerificationCompleted(WorkflowVerificationCompletedPayload),

    /// A workflow phase subtask was retried after failure.
    WorkflowPhaseRetried {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
        retry_count: u64,
    },

    /// A workflow phase failed after exhausting all retries.
    WorkflowPhaseFailed {
        task_id: Uuid,
        phase_index: usize,
        phase_name: String,
        reason: String,
    },

    // ========================================================================
    // Adapter — adapter ingestion and egress
    // ========================================================================
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

    // ========================================================================
    // Budget — budget pressure and opportunity
    // ========================================================================
    /// Emitted when the aggregate budget pressure level changes.
    BudgetPressureChanged {
        previous_level: BudgetPressureLevel,
        new_level: BudgetPressureLevel,
        /// Consumed percentage (0.0–1.0) that triggered the transition.
        consumed_pct: f64,
        /// ID of the window that caused the change (or "aggregate" for recompute).
        window_id: String,
    },

    /// Emitted when a budget window has enough remaining tokens to schedule
    /// additional work aggressively.
    BudgetOpportunityDetected {
        window_id: String,
        remaining_tokens: u64,
        time_to_reset_secs: u64,
        opportunity_score: f64,
    },

    // ========================================================================
    // Federation — cerebrate connectivity, task delegation, federated goals,
    //              swarm DAGs
    // ========================================================================
    /// A cerebrate connected to the federation.
    FederationCerebrateConnected {
        cerebrate_id: String,
        capabilities: Vec<String>,
    },

    /// A cerebrate disconnected from the federation.
    FederationCerebrateDisconnected {
        cerebrate_id: String,
        reason: String,
    },

    /// A task was delegated to a cerebrate.
    FederationTaskDelegated {
        task_id: Uuid,
        cerebrate_id: String,
    },

    /// A cerebrate accepted a delegated task.
    FederationTaskAccepted {
        task_id: Uuid,
        cerebrate_id: String,
    },

    /// A cerebrate rejected a delegated task.
    FederationTaskRejected {
        task_id: Uuid,
        cerebrate_id: String,
        reason: String,
    },

    /// Progress received from a cerebrate on a delegated task.
    FederationProgressReceived {
        task_id: Uuid,
        cerebrate_id: String,
        phase: String,
        progress_pct: f64,
        summary: String,
    },

    /// Final result received from a cerebrate.
    FederationResultReceived {
        task_id: Uuid,
        cerebrate_id: String,
        status: String,
        summary: String,
        artifacts: Vec<crate::domain::models::a2a::Artifact>,
    },

    /// A cerebrate missed heartbeats.
    FederationHeartbeatMissed {
        cerebrate_id: String,
        missed_count: u32,
    },

    /// A cerebrate became unreachable after too many missed heartbeats.
    FederationCerebrateUnreachable {
        cerebrate_id: String,
        in_flight_tasks: Vec<Uuid>,
    },

    /// Stall detected: no progress from cerebrate within threshold.
    FederationStallDetected {
        task_id: Uuid,
        cerebrate_id: String,
        stall_duration_secs: u64,
    },

    /// A federation reaction was emitted by the result processor.
    /// Distinct from protocol-level events — these represent the system's
    /// reactive decisions in response to federation results.
    FederationReactionEmitted {
        reaction_type: String,
        description: String,
        goal_id: Option<Uuid>,
        task_id: Option<Uuid>,
    },

    /// A federated goal was created and delegated to a child cerebrate.
    FederatedGoalCreated {
        local_goal_id: Uuid,
        cerebrate_id: String,
        remote_task_id: String,
    },

    /// Progress update received for a federated goal.
    FederatedGoalProgress {
        /// NOTE: Despite the name, this is the `FederatedGoal.id` (federation
        /// record UUID), not `FederatedGoal.local_goal_id` (parent goal UUID).
        /// Used by `SwarmDagEventHandler` to correlate with DAG node ownership.
        local_goal_id: Uuid,
        convergence_level: f64,
        signals: std::collections::HashMap<String, f64>,
    },

    /// A federated goal converged successfully.
    FederatedGoalConverged {
        /// NOTE: Despite the name, this is the `FederatedGoal.id` (federation
        /// record UUID), not `FederatedGoal.local_goal_id` (parent goal UUID).
        /// Used by `SwarmDagEventHandler` to correlate with DAG node ownership.
        local_goal_id: Uuid,
        cerebrate_id: String,
    },

    /// A federated goal failed.
    FederatedGoalFailed {
        /// NOTE: Despite the name, this is the `FederatedGoal.id` (federation
        /// record UUID), not `FederatedGoal.local_goal_id` (parent goal UUID).
        /// Used by `SwarmDagEventHandler` to correlate with DAG node ownership.
        local_goal_id: Uuid,
        cerebrate_id: String,
        reason: String,
    },

    /// A swarm DAG was created and execution started.
    SwarmDagCreated {
        dag_id: Uuid,
        dag_name: String,
        node_count: usize,
    },

    /// A swarm DAG node was delegated to a child swarm.
    SwarmDagNodeDelegated {
        dag_id: Uuid,
        node_id: Uuid,
        node_label: String,
        cerebrate_id: String,
    },

    /// A swarm DAG node was unblocked (dependencies met, now delegated).
    SwarmDagNodeUnblocked {
        dag_id: Uuid,
        node_id: Uuid,
        node_label: String,
    },

    /// A swarm DAG node failed.
    SwarmDagNodeFailed {
        dag_id: Uuid,
        node_id: Uuid,
        node_label: String,
        reason: String,
    },

    /// A swarm DAG completed (all nodes terminal).
    SwarmDagCompleted {
        dag_id: Uuid,
        dag_name: String,
        converged_count: usize,
        failed_count: usize,
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
            Self::IntentVerificationCompleted(_) => "IntentVerificationCompleted",
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
            Self::HumanEscalationRequired(_) => "HumanEscalationRequired",
            Self::HumanEscalationNeeded(_) => "HumanEscalationNeeded",
            Self::HumanResponseReceived { .. } => "HumanResponseReceived",
            Self::TriggerRuleCreated { .. } => "TriggerRuleCreated",
            Self::TriggerRuleToggled { .. } => "TriggerRuleToggled",
            Self::TriggerRuleDeleted { .. } => "TriggerRuleDeleted",
            Self::MemoryMaintenanceCompleted { .. } => "MemoryMaintenanceCompleted",
            Self::MemoryMaintenanceFailed { .. } => "MemoryMaintenanceFailed",
            Self::MemoryDaemonDegraded { .. } => "MemoryDaemonDegraded",
            Self::MemoryDaemonStopped { .. } => "MemoryDaemonStopped",
            Self::HandlerError { .. } => "HandlerError",
            Self::CriticalHandlerDegraded { .. } => "CriticalHandlerDegraded",
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
            Self::ConvergenceIteration(_) => "ConvergenceIteration",
            Self::ConvergenceAttractorTransition { .. } => "ConvergenceAttractorTransition",
            Self::ConvergenceBudgetExtension { .. } => "ConvergenceBudgetExtension",
            Self::ConvergenceFreshStart { .. } => "ConvergenceFreshStart",
            Self::ConvergenceTerminated(_) => "ConvergenceTerminated",
            Self::TaskExecutionRecorded { .. } => "TaskExecutionRecorded",
            Self::SubtaskMergedToFeature { .. } => "SubtaskMergedToFeature",
            Self::AdapterIngestionCompleted { .. } => "AdapterIngestionCompleted",
            Self::AdapterIngestionFailed { .. } => "AdapterIngestionFailed",
            Self::AdapterEgressCompleted { .. } => "AdapterEgressCompleted",
            Self::AdapterEgressFailed { .. } => "AdapterEgressFailed",
            Self::AdapterTaskIngested { .. } => "AdapterTaskIngested",
            Self::BudgetPressureChanged { .. } => "BudgetPressureChanged",
            Self::BudgetOpportunityDetected { .. } => "BudgetOpportunityDetected",
            Self::WorkflowEnrolled { .. } => "WorkflowEnrolled",
            Self::WorkflowPhaseStarted { .. } => "WorkflowPhaseStarted",
            Self::WorkflowGateReached { .. } => "WorkflowGateReached",
            Self::WorkflowGateVerdict { .. } => "WorkflowGateVerdict",
            Self::WorkflowGateRejected { .. } => "WorkflowGateRejected",
            Self::WorkflowAdvanced { .. } => "WorkflowAdvanced",
            Self::WorkflowPhaseReady { .. } => "WorkflowPhaseReady",
            Self::WorkflowCompleted { .. } => "WorkflowCompleted",
            Self::WorkflowVerificationRequested { .. } => "WorkflowVerificationRequested",
            Self::WorkflowVerificationCompleted(_) => "WorkflowVerificationCompleted",
            Self::WorkflowPhaseRetried { .. } => "WorkflowPhaseRetried",
            Self::WorkflowPhaseFailed { .. } => "WorkflowPhaseFailed",
            Self::FederationCerebrateConnected { .. } => "FederationCerebrateConnected",
            Self::FederationCerebrateDisconnected { .. } => "FederationCerebrateDisconnected",
            Self::FederationTaskDelegated { .. } => "FederationTaskDelegated",
            Self::FederationTaskAccepted { .. } => "FederationTaskAccepted",
            Self::FederationTaskRejected { .. } => "FederationTaskRejected",
            Self::FederationProgressReceived { .. } => "FederationProgressReceived",
            Self::FederationResultReceived { .. } => "FederationResultReceived",
            Self::FederationHeartbeatMissed { .. } => "FederationHeartbeatMissed",
            Self::FederationCerebrateUnreachable { .. } => "FederationCerebrateUnreachable",
            Self::FederationStallDetected { .. } => "FederationStallDetected",
            Self::FederationReactionEmitted { .. } => "FederationReactionEmitted",
            Self::FederatedGoalCreated { .. } => "FederatedGoalCreated",
            Self::FederatedGoalProgress { .. } => "FederatedGoalProgress",
            Self::FederatedGoalConverged { .. } => "FederatedGoalConverged",
            Self::FederatedGoalFailed { .. } => "FederatedGoalFailed",
            Self::SwarmDagCreated { .. } => "SwarmDagCreated",
            Self::SwarmDagNodeDelegated { .. } => "SwarmDagNodeDelegated",
            Self::SwarmDagNodeUnblocked { .. } => "SwarmDagNodeUnblocked",
            Self::SwarmDagNodeFailed { .. } => "SwarmDagNodeFailed",
            Self::SwarmDagCompleted { .. } => "SwarmDagCompleted",
            Self::SubsystemError { .. } => "SubsystemError",
            Self::QuietWindowEntered { .. } => "QuietWindowEntered",
            Self::QuietWindowExited { .. } => "QuietWindowExited",
        }
    }

    pub fn expected_category(&self) -> Option<EventCategory> {
        match self {
            Self::OrchestratorStarted
            | Self::OrchestratorPaused
            | Self::OrchestratorResumed
            | Self::OrchestratorStopped
            | Self::StatusUpdate(_)
            | Self::ReconciliationCompleted { .. }
            | Self::StartupCatchUpCompleted { .. }
            | Self::HandlerError { .. }
            | Self::CriticalHandlerDegraded { .. }
            | Self::TriggerRuleCreated { .. }
            | Self::TriggerRuleToggled { .. }
            | Self::TriggerRuleDeleted { .. } => Some(EventCategory::Orchestrator),

            Self::GoalStarted { .. }
            | Self::GoalDecomposed { .. }
            | Self::GoalIterationCompleted { .. }
            | Self::GoalPaused { .. }
            | Self::ConvergenceCompleted { .. }
            | Self::SemanticDriftDetected { .. }
            | Self::GoalStatusChanged { .. }
            | Self::GoalConstraintViolated { .. }
            | Self::GoalDomainsUpdated { .. }
            | Self::GoalDeleted { .. }
            | Self::GoalConstraintsUpdated { .. }
            | Self::GoalDescriptionUpdated { .. }
            | Self::MemoryInformedGoal { .. } => Some(EventCategory::Goal),

            Self::TaskSubmitted { .. }
            | Self::TaskReady { .. }
            | Self::TaskSpawned { .. }
            | Self::TaskStarted { .. }
            | Self::TaskCompleted { .. }
            | Self::TaskCompletedWithResult { .. }
            | Self::TaskFailed { .. }
            | Self::TaskRetrying { .. }
            | Self::TaskQueuedForMerge { .. }
            | Self::TaskMerged { .. }
            | Self::TaskClaimed { .. }
            | Self::TaskCanceled { .. }
            | Self::TaskValidating { .. }
            | Self::TaskSLAWarning { .. }
            | Self::TaskSLACritical { .. }
            | Self::TaskSLABreached { .. }
            | Self::TaskRunningLong { .. }
            | Self::TaskRunningCritical { .. }
            | Self::TaskDependencyChanged { .. }
            | Self::TaskPriorityChanged { .. }
            | Self::TaskDescriptionUpdated { .. }
            | Self::WorktreeCreated { .. }
            | Self::WorktreeDestroyed { .. }
            | Self::PullRequestCreated { .. }
            | Self::SubtaskMergedToFeature { .. }
            | Self::ReviewLoopTriggered { .. } => Some(EventCategory::Task),

            Self::ExecutionStarted { .. }
            | Self::ExecutionCompleted { .. }
            | Self::WaveStarted { .. }
            | Self::WaveCompleted { .. }
            | Self::RestructureTriggered { .. }
            | Self::RestructureDecision { .. } => Some(EventCategory::Execution),

            Self::AgentCreated { .. }
            | Self::SpecialistSpawned { .. }
            | Self::EvolutionTriggered { .. }
            | Self::SpawnLimitExceeded { .. }
            | Self::AgentInstanceCompleted { .. }
            | Self::AgentTemplateRegistered { .. }
            | Self::AgentTemplateStatusChanged { .. }
            | Self::AgentInstanceSpawned { .. }
            | Self::AgentInstanceAssigned { .. }
            | Self::AgentInstanceFailed { .. } => Some(EventCategory::Agent),

            Self::IntentVerificationStarted { .. }
            | Self::IntentVerificationCompleted(_)
            | Self::IntentVerificationRequested { .. }
            | Self::WaveVerificationRequested { .. }
            | Self::WaveVerificationResult { .. }
            | Self::BranchVerificationStarted { .. }
            | Self::BranchVerificationRequested { .. }
            | Self::BranchVerificationCompleted { .. }
            | Self::BranchVerificationResult { .. }
            | Self::GoalAlignmentEvaluated { .. }
            | Self::TaskVerified { .. }
            | Self::IntentVerificationResult { .. } => Some(EventCategory::Verification),

            Self::HumanEscalationRequired(_)
            | Self::HumanEscalationNeeded(_)
            | Self::HumanResponseReceived { .. }
            | Self::HumanEscalationExpired { .. } => Some(EventCategory::Escalation),

            Self::MemoryStored { .. }
            | Self::MemoryPromoted { .. }
            | Self::MemoryPruned { .. }
            | Self::MemoryAccessed { .. }
            | Self::MemoryDeleted { .. }
            | Self::MemoryConflictDetected { .. }
            | Self::MemoryConflictResolved { .. }
            | Self::MemoryMaintenanceCompleted { .. }
            | Self::MemoryMaintenanceFailed { .. }
            | Self::MemoryDaemonDegraded { .. }
            | Self::MemoryDaemonStopped { .. } => Some(EventCategory::Memory),

            Self::ScheduledEventFired { .. }
            | Self::ScheduledEventRegistered { .. }
            | Self::ScheduledEventCanceled { .. } => Some(EventCategory::Scheduler),

            Self::ConvergenceStarted { .. }
            | Self::ConvergenceIteration(_)
            | Self::ConvergenceAttractorTransition { .. }
            | Self::ConvergenceBudgetExtension { .. }
            | Self::ConvergenceFreshStart { .. }
            | Self::ConvergenceTerminated(_) => Some(EventCategory::Convergence),

            Self::TaskExecutionRecorded { .. } => Some(EventCategory::Task),

            Self::WorkflowEnrolled { .. }
            | Self::WorkflowPhaseStarted { .. }
            | Self::WorkflowGateReached { .. }
            | Self::WorkflowGateVerdict { .. }
            | Self::WorkflowGateRejected { .. }
            | Self::WorkflowAdvanced { .. }
            | Self::WorkflowPhaseReady { .. }
            | Self::WorkflowCompleted { .. }
            | Self::WorkflowVerificationRequested { .. }
            | Self::WorkflowVerificationCompleted(_)
            | Self::WorkflowPhaseRetried { .. }
            | Self::WorkflowPhaseFailed { .. } => Some(EventCategory::Workflow),

            Self::AdapterIngestionCompleted { .. }
            | Self::AdapterIngestionFailed { .. }
            | Self::AdapterEgressCompleted { .. }
            | Self::AdapterEgressFailed { .. }
            | Self::AdapterTaskIngested { .. } => Some(EventCategory::Adapter),

            Self::BudgetPressureChanged { .. } | Self::BudgetOpportunityDetected { .. } => {
                Some(EventCategory::Budget)
            }

            Self::FederationCerebrateConnected { .. }
            | Self::FederationCerebrateDisconnected { .. }
            | Self::FederationTaskDelegated { .. }
            | Self::FederationTaskAccepted { .. }
            | Self::FederationTaskRejected { .. }
            | Self::FederationProgressReceived { .. }
            | Self::FederationResultReceived { .. }
            | Self::FederationHeartbeatMissed { .. }
            | Self::FederationCerebrateUnreachable { .. }
            | Self::FederationStallDetected { .. }
            | Self::FederationReactionEmitted { .. }
            | Self::FederatedGoalCreated { .. }
            | Self::FederatedGoalProgress { .. }
            | Self::FederatedGoalConverged { .. }
            | Self::FederatedGoalFailed { .. }
            | Self::SwarmDagCreated { .. }
            | Self::SwarmDagNodeDelegated { .. }
            | Self::SwarmDagNodeUnblocked { .. }
            | Self::SwarmDagNodeFailed { .. }
            | Self::SwarmDagCompleted { .. } => Some(EventCategory::Federation),

            Self::SubsystemError { .. } => Some(EventCategory::Orchestrator),
            Self::QuietWindowEntered { .. } | Self::QuietWindowExited { .. } => {
                Some(EventCategory::Scheduler)
            }
        }
    }
}

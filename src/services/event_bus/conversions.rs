//! Conversions from legacy event types into `UnifiedEvent`, plus the
//! serializable payload structs used by those conversions.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::dag_executor::{ExecutionEvent, ExecutionResults, ExecutionStatus, TaskResult};
use super::super::swarm_orchestrator::{SwarmEvent, SwarmStats};
use super::payload::{EventPayload, HumanEscalationPayload, IntentVerificationCompletedPayload};
use super::types::{EventCategory, EventId, EventSeverity, SequenceNumber, UnifiedEvent};

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
///
/// # Egress routing
///
/// The optional [`egress`](Self::egress) field carries a structured
/// [`EgressDirective`](crate::domain::models::adapter::EgressDirective) when a
/// task completion should be routed to an external system. Historically, this
/// was overloaded onto the [`status`](Self::status) string as a JSON blob with
/// an `"egress"` key; that legacy path is still honored by
/// `EgressRoutingHandler` for backwards compatibility with older persisted
/// events, but new producers should populate this dedicated field instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultPayload {
    pub task_id: Uuid,
    pub status: String,
    pub error: Option<String>,
    pub duration_secs: u64,
    pub retry_count: u32,
    pub tokens_used: u64,
    /// Optional structured egress routing directive. When present,
    /// `EgressRoutingHandler` consumes this instead of trying to parse the
    /// status field for a JSON-embedded directive. `None` on events produced
    /// before this field existed — deserialization tolerates its absence via
    /// `serde(default)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<crate::domain::models::adapter::EgressDirective>,
}

impl From<TaskResult> for TaskResultPayload {
    fn from(result: TaskResult) -> Self {
        Self {
            task_id: result.task_id,
            status: format!("{:?}", result.status),
            error: result.error,
            duration_secs: result.duration_secs,
            retry_count: result.retry_count,
            tokens_used: result
                .session
                .as_ref()
                .map(|s| s.total_tokens())
                .unwrap_or(0),
            egress: None,
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
            SwarmEvent::GoalDecomposed {
                goal_id,
                task_count,
            } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalDecomposed {
                    goal_id,
                    task_count,
                },
            ),
            SwarmEvent::GoalIterationCompleted {
                goal_id,
                tasks_completed,
            } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalIterationCompleted {
                    goal_id,
                    tasks_completed,
                },
            ),
            SwarmEvent::GoalPaused { goal_id, reason } => (
                EventSeverity::Warning,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::GoalPaused { goal_id, reason },
            ),
            SwarmEvent::ConvergenceCompleted {
                goal_id,
                converged,
                iterations,
                final_satisfaction,
            } => (
                EventSeverity::Info,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::ConvergenceCompleted {
                    goal_id,
                    converged,
                    iterations,
                    final_satisfaction,
                },
            ),
            SwarmEvent::SemanticDriftDetected {
                goal_id,
                recurring_gaps,
                iterations,
            } => (
                EventSeverity::Warning,
                EventCategory::Goal,
                Some(goal_id),
                None,
                EventPayload::SemanticDriftDetected {
                    goal_id,
                    recurring_gaps,
                    iterations,
                },
            ),
            SwarmEvent::TaskSubmitted {
                task_id,
                task_title,
                goal_id,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                goal_id,
                Some(task_id),
                EventPayload::TaskSubmitted {
                    task_id,
                    task_title,
                    goal_id,
                },
            ),
            SwarmEvent::TaskReady {
                task_id,
                task_title,
            } => (
                EventSeverity::Debug,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskReady {
                    task_id,
                    task_title,
                },
            ),
            SwarmEvent::TaskSpawned {
                task_id,
                task_title,
                agent_type,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskSpawned {
                    task_id,
                    task_title,
                    agent_type,
                },
            ),
            SwarmEvent::TaskCompleted {
                task_id,
                tokens_used,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskCompleted {
                    task_id,
                    tokens_used,
                },
            ),
            SwarmEvent::TaskFailed {
                task_id,
                error,
                retry_count,
            } => (
                EventSeverity::Error,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskFailed {
                    task_id,
                    error,
                    retry_count,
                },
            ),
            SwarmEvent::TaskRetrying {
                task_id,
                attempt,
                max_attempts,
            } => (
                EventSeverity::Warning,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskRetrying {
                    task_id,
                    attempt,
                    max_attempts,
                },
            ),
            SwarmEvent::TaskVerified {
                task_id,
                passed,
                checks_passed,
                checks_total,
                failures_summary,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                Some(task_id),
                EventPayload::TaskVerified {
                    task_id,
                    passed,
                    checks_passed,
                    checks_total,
                    failures_summary,
                },
            ),
            SwarmEvent::TaskQueuedForMerge { task_id, stage } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskQueuedForMerge { task_id, stage },
            ),
            SwarmEvent::PullRequestCreated {
                task_id,
                pr_url,
                branch,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::PullRequestCreated {
                    task_id,
                    pr_url,
                    branch,
                },
            ),
            SwarmEvent::TaskMerged {
                task_id,
                commit_sha,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskMerged {
                    task_id,
                    commit_sha,
                },
            ),
            SwarmEvent::WorktreeCreated { task_id, path } => (
                EventSeverity::Debug,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::WorktreeCreated { task_id, path },
            ),
            SwarmEvent::TaskClaimed {
                task_id,
                agent_type,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskClaimed {
                    task_id,
                    agent_type,
                },
            ),
            SwarmEvent::AgentInstanceCompleted {
                instance_id,
                task_id,
                tokens_used,
            } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                Some(task_id),
                EventPayload::AgentInstanceCompleted {
                    instance_id,
                    task_id,
                    tokens_used,
                },
            ),
            SwarmEvent::ReconciliationCompleted { corrections_made } => (
                EventSeverity::Debug,
                EventCategory::Orchestrator,
                None,
                None,
                EventPayload::ReconciliationCompleted { corrections_made },
            ),
            SwarmEvent::EvolutionTriggered {
                template_name,
                trigger,
            } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                None,
                EventPayload::EvolutionTriggered {
                    template_name,
                    trigger,
                },
            ),
            SwarmEvent::SpecialistSpawned {
                specialist_type,
                trigger,
                task_id,
            } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                task_id,
                EventPayload::SpecialistSpawned {
                    specialist_type,
                    trigger,
                    task_id,
                },
            ),
            SwarmEvent::AgentCreated { agent_type, tier } => (
                EventSeverity::Info,
                EventCategory::Agent,
                None,
                None,
                EventPayload::AgentCreated { agent_type, tier },
            ),
            SwarmEvent::GoalAlignmentEvaluated {
                task_id,
                overall_score,
                passes,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                Some(task_id),
                EventPayload::GoalAlignmentEvaluated {
                    task_id,
                    overall_score,
                    passes,
                },
            ),
            SwarmEvent::RestructureTriggered { task_id, decision } => (
                EventSeverity::Warning,
                EventCategory::Execution,
                None,
                Some(task_id),
                EventPayload::RestructureTriggered { task_id, decision },
            ),
            SwarmEvent::SpawnLimitExceeded {
                parent_task_id,
                limit_type,
                current_value,
                limit_value,
            } => (
                EventSeverity::Warning,
                EventCategory::Agent,
                None,
                Some(parent_task_id),
                EventPayload::SpawnLimitExceeded {
                    parent_task_id,
                    limit_type,
                    current_value,
                    limit_value,
                },
            ),
            SwarmEvent::IntentVerificationStarted { goal_id, iteration } => (
                EventSeverity::Info,
                EventCategory::Verification,
                Some(goal_id),
                None,
                EventPayload::IntentVerificationStarted { goal_id, iteration },
            ),
            SwarmEvent::IntentVerificationCompleted {
                goal_id,
                satisfaction,
                confidence,
                gaps_count,
                iteration,
                will_retry,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                Some(goal_id),
                None,
                EventPayload::IntentVerificationCompleted(IntentVerificationCompletedPayload {
                    goal_id,
                    satisfaction,
                    confidence,
                    gaps_count,
                    iteration,
                    will_retry,
                }),
            ),
            SwarmEvent::BranchVerificationStarted {
                branch_task_ids,
                waiting_task_ids,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationStarted {
                    branch_task_ids,
                    waiting_task_ids,
                },
            ),
            SwarmEvent::BranchVerificationCompleted {
                branch_satisfied,
                dependents_can_proceed,
                gaps_count,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationCompleted {
                    branch_satisfied,
                    dependents_can_proceed,
                    gaps_count,
                },
            ),
            SwarmEvent::HumanEscalationRequired {
                goal_id,
                task_id,
                reason,
                urgency,
                questions,
                is_blocking,
            } => (
                if is_blocking {
                    EventSeverity::Critical
                } else {
                    EventSeverity::Warning
                },
                EventCategory::Escalation,
                goal_id,
                task_id,
                EventPayload::HumanEscalationRequired(HumanEscalationPayload {
                    goal_id,
                    task_id,
                    reason,
                    urgency,
                    questions,
                    is_blocking,
                }),
            ),
            SwarmEvent::HumanResponseReceived {
                escalation_id,
                decision,
                allows_continuation,
            } => (
                EventSeverity::Info,
                EventCategory::Escalation,
                None,
                None,
                EventPayload::HumanResponseReceived {
                    escalation_id,
                    decision,
                    allows_continuation,
                },
            ),
            SwarmEvent::SubtaskMergedToFeature {
                task_id,
                feature_branch,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::SubtaskMergedToFeature {
                    task_id,
                    feature_branch,
                },
            ),
            // Federation events
            SwarmEvent::FederationCerebrateConnected {
                cerebrate_id,
                capabilities,
            } => (
                EventSeverity::Info,
                EventCategory::Federation,
                None,
                None,
                EventPayload::FederationCerebrateConnected {
                    cerebrate_id,
                    capabilities,
                },
            ),
            SwarmEvent::FederationCerebrateDisconnected {
                cerebrate_id,
                reason,
            } => (
                EventSeverity::Warning,
                EventCategory::Federation,
                None,
                None,
                EventPayload::FederationCerebrateDisconnected {
                    cerebrate_id,
                    reason,
                },
            ),
            SwarmEvent::FederationTaskDelegated {
                task_id,
                cerebrate_id,
            } => (
                EventSeverity::Info,
                EventCategory::Federation,
                None,
                Some(task_id),
                EventPayload::FederationTaskDelegated {
                    task_id,
                    cerebrate_id,
                },
            ),
            SwarmEvent::FederationTaskAccepted {
                task_id,
                cerebrate_id,
            } => (
                EventSeverity::Info,
                EventCategory::Federation,
                None,
                Some(task_id),
                EventPayload::FederationTaskAccepted {
                    task_id,
                    cerebrate_id,
                },
            ),
            SwarmEvent::FederationTaskRejected {
                task_id,
                cerebrate_id,
                reason,
            } => (
                EventSeverity::Warning,
                EventCategory::Federation,
                None,
                Some(task_id),
                EventPayload::FederationTaskRejected {
                    task_id,
                    cerebrate_id,
                    reason,
                },
            ),
            SwarmEvent::FederationProgressReceived {
                task_id,
                cerebrate_id,
                phase,
                progress_pct,
                summary,
            } => (
                EventSeverity::Debug,
                EventCategory::Federation,
                None,
                Some(task_id),
                EventPayload::FederationProgressReceived {
                    task_id,
                    cerebrate_id,
                    phase,
                    progress_pct,
                    summary,
                },
            ),
            SwarmEvent::FederationResultReceived {
                task_id,
                cerebrate_id,
                status,
                summary,
                artifacts,
            } => (
                EventSeverity::Info,
                EventCategory::Federation,
                None,
                Some(task_id),
                EventPayload::FederationResultReceived {
                    task_id,
                    cerebrate_id,
                    status,
                    summary,
                    artifacts,
                },
            ),
            SwarmEvent::FederationHeartbeatMissed {
                cerebrate_id,
                missed_count,
            } => (
                EventSeverity::Warning,
                EventCategory::Federation,
                None,
                None,
                EventPayload::FederationHeartbeatMissed {
                    cerebrate_id,
                    missed_count,
                },
            ),
            SwarmEvent::FederationCerebrateUnreachable {
                cerebrate_id,
                in_flight_tasks,
            } => (
                EventSeverity::Error,
                EventCategory::Federation,
                None,
                None,
                EventPayload::FederationCerebrateUnreachable {
                    cerebrate_id,
                    in_flight_tasks,
                },
            ),
            SwarmEvent::FederationStallDetected {
                task_id,
                cerebrate_id,
                stall_duration_secs,
            } => (
                EventSeverity::Warning,
                EventCategory::Federation,
                None,
                Some(task_id),
                EventPayload::FederationStallDetected {
                    task_id,
                    cerebrate_id,
                    stall_duration_secs,
                },
            ),
            SwarmEvent::FederationReactionEmitted {
                reaction_type,
                description,
                goal_id,
                task_id,
            } => (
                EventSeverity::Info,
                EventCategory::Federation,
                goal_id,
                task_id,
                EventPayload::FederationReactionEmitted {
                    reaction_type,
                    description,
                    goal_id,
                    task_id,
                },
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
            ExecutionEvent::Started {
                total_tasks,
                wave_count,
            } => (
                EventSeverity::Info,
                EventCategory::Execution,
                None,
                None,
                EventPayload::ExecutionStarted {
                    total_tasks,
                    wave_count,
                },
            ),
            ExecutionEvent::Completed { status, results } => (
                if matches!(status, ExecutionStatus::Failed) {
                    EventSeverity::Error
                } else {
                    EventSeverity::Info
                },
                EventCategory::Execution,
                None,
                None,
                EventPayload::ExecutionCompleted {
                    status: status.into(),
                    results: results.into(),
                },
            ),
            ExecutionEvent::WaveStarted {
                wave_number,
                task_count,
            } => (
                EventSeverity::Info,
                EventCategory::Execution,
                None,
                None,
                EventPayload::WaveStarted {
                    wave_number,
                    task_count,
                },
            ),
            ExecutionEvent::WaveCompleted {
                wave_number,
                succeeded,
                failed,
            } => (
                if failed > 0 {
                    EventSeverity::Warning
                } else {
                    EventSeverity::Info
                },
                EventCategory::Execution,
                None,
                None,
                EventPayload::WaveCompleted {
                    wave_number,
                    succeeded,
                    failed,
                },
            ),
            ExecutionEvent::TaskStarted {
                task_id,
                task_title,
            } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskStarted {
                    task_id,
                    task_title,
                },
            ),
            ExecutionEvent::TaskCompleted { task_id, result } => (
                EventSeverity::Info,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskCompletedWithResult {
                    task_id,
                    result: result.into(),
                },
            ),
            ExecutionEvent::TaskFailed {
                task_id,
                error,
                retry_count,
            } => (
                EventSeverity::Error,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskFailed {
                    task_id,
                    error,
                    retry_count,
                },
            ),
            ExecutionEvent::TaskRetrying {
                task_id,
                attempt,
                max_attempts,
            } => (
                EventSeverity::Warning,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskRetrying {
                    task_id,
                    attempt,
                    max_attempts,
                },
            ),
            ExecutionEvent::RestructureDecision { task_id, decision } => (
                EventSeverity::Warning,
                EventCategory::Execution,
                None,
                Some(task_id),
                EventPayload::RestructureDecision { task_id, decision },
            ),
            ExecutionEvent::IntentVerificationRequested {
                goal_id,
                completed_task_ids,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                goal_id,
                None,
                EventPayload::IntentVerificationRequested {
                    goal_id,
                    completed_task_ids,
                },
            ),
            ExecutionEvent::IntentVerificationResult {
                satisfaction,
                confidence,
                gaps_count,
                iteration,
                should_continue,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::IntentVerificationResult {
                    satisfaction,
                    confidence,
                    gaps_count,
                    iteration,
                    should_continue,
                },
            ),
            ExecutionEvent::WaveVerificationRequested {
                wave_number,
                completed_task_ids,
                goal_id,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                goal_id,
                None,
                EventPayload::WaveVerificationRequested {
                    wave_number,
                    completed_task_ids,
                    goal_id,
                },
            ),
            ExecutionEvent::WaveVerificationResult {
                wave_number,
                satisfaction,
                confidence,
                gaps_count,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::WaveVerificationResult {
                    wave_number,
                    satisfaction,
                    confidence,
                    gaps_count,
                },
            ),
            ExecutionEvent::BranchVerificationRequested {
                branch_task_ids,
                waiting_task_ids,
                branch_objective,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationRequested {
                    branch_task_ids,
                    waiting_task_ids,
                    branch_objective,
                },
            ),
            ExecutionEvent::BranchVerificationResult {
                branch_satisfied,
                confidence,
                gaps_count,
                dependents_can_proceed,
            } => (
                EventSeverity::Info,
                EventCategory::Verification,
                None,
                None,
                EventPayload::BranchVerificationResult {
                    branch_satisfied,
                    confidence,
                    gaps_count,
                    dependents_can_proceed,
                },
            ),
            ExecutionEvent::HumanEscalationNeeded {
                goal_id,
                task_id,
                reason,
                urgency,
                questions,
                is_blocking,
            } => (
                if is_blocking {
                    EventSeverity::Critical
                } else {
                    EventSeverity::Warning
                },
                EventCategory::Escalation,
                goal_id,
                task_id,
                EventPayload::HumanEscalationNeeded(HumanEscalationPayload {
                    goal_id,
                    task_id,
                    reason,
                    urgency,
                    questions,
                    is_blocking,
                }),
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

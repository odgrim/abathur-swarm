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
            SwarmEvent::TaskVerified { task_id, passed, checks_passed, checks_total } => (
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

        // Add correlation ID from context if not set
        if event.correlation_id.is_none() {
            let ctx = self.correlation_context.read().await;
            event.correlation_id = *ctx;
        }

        // Persist if store is configured
        if self.config.persist_events {
            if let Some(ref store) = self.store {
                if let Err(e) = store.append(&event).await {
                    tracing::warn!("Failed to persist event: {}", e);
                }
            }
        }

        // Broadcast to subscribers (ignore send errors - may have no subscribers)
        let _ = self.sender.send(event);
    }

    /// Publish a SwarmEvent (converts to UnifiedEvent).
    pub async fn publish_swarm_event(&self, event: SwarmEvent) {
        self.publish(event.into()).await;
    }

    /// Publish an ExecutionEvent (converts to UnifiedEvent).
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

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

#[cfg(test)]
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

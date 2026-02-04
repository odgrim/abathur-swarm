//! Overmind decision types.
//!
//! The Overmind is the "probabilistic brain" of the Abathur swarm - an Architect-tier
//! agent invoked by deterministic services at key decision points to provide intelligent,
//! context-aware strategic decisions.
//!
//! ## Design Philosophy
//!
//! The Overmind operates on the principle of "structured cognition" - it receives
//! strongly-typed requests and returns strongly-typed decisions. This allows the
//! deterministic orchestration layer to invoke intelligence precisely where needed
//! while maintaining predictable system behavior.
//!
//! ## Decision Types
//!
//! - **Goal Decomposition**: Strategic task DAG generation with dependencies
//! - **Cross-Goal Prioritization**: Balance multiple active goals
//! - **Capability Gap Analysis**: Detect and decide on new agent creation
//! - **Conflict Resolution**: Mediate between agents/tasks
//! - **Stuck State Recovery**: Analyze failures and determine recovery
//! - **Escalation Decisions**: Determine when human input is truly needed

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::models::TaskPriority;

// ============================================================================
// Request Types
// ============================================================================

/// All possible requests to the Overmind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "request_type", rename_all = "snake_case")]
pub enum OvermindRequest {
    /// Request to decompose a goal into tasks.
    GoalDecomposition(GoalDecompositionRequest),
    /// Request to prioritize multiple goals.
    Prioritization(PrioritizationRequest),
    /// Request to analyze a capability gap.
    CapabilityGap(CapabilityGapRequest),
    /// Request to resolve a conflict.
    ConflictResolution(ConflictResolutionRequest),
    /// Request to recover from a stuck state.
    StuckStateRecovery(StuckStateRecoveryRequest),
    /// Request to evaluate whether to escalate.
    Escalation(EscalationRequest),
}

impl OvermindRequest {
    /// Get a human-readable description of this request type.
    pub fn request_type_name(&self) -> &'static str {
        match self {
            Self::GoalDecomposition(_) => "goal_decomposition",
            Self::Prioritization(_) => "prioritization",
            Self::CapabilityGap(_) => "capability_gap",
            Self::ConflictResolution(_) => "conflict_resolution",
            Self::StuckStateRecovery(_) => "stuck_state_recovery",
            Self::Escalation(_) => "escalation",
        }
    }
}

/// Request to decompose a goal into a task DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalDecompositionRequest {
    /// The goal to decompose.
    pub goal_id: Uuid,
    /// Goal name.
    pub goal_name: String,
    /// Goal description.
    pub goal_description: String,
    /// Goal constraints.
    pub constraints: Vec<String>,
    /// Available agent types.
    pub available_agents: Vec<String>,
    /// Existing tasks for this goal (if any).
    pub existing_tasks: Vec<ExistingTaskSummary>,
    /// Memory patterns from similar decompositions.
    pub memory_patterns: Vec<String>,
    /// Maximum number of tasks to create.
    pub max_tasks: usize,
}

/// Summary of an existing task for context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingTaskSummary {
    pub id: Uuid,
    pub title: String,
    pub status: String,
    pub agent_type: Option<String>,
}

/// Request to prioritize multiple active goals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrioritizationRequest {
    /// Active goals to prioritize.
    pub goals: Vec<GoalSummary>,
    /// Current resource constraints.
    pub resource_constraints: ResourceConstraints,
    /// Conflicts between goals (if any).
    pub known_conflicts: Vec<GoalConflict>,
}

/// Summary of a goal for prioritization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSummary {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub priority: String,
    pub pending_tasks: usize,
    pub blocked_tasks: usize,
    pub completed_tasks: usize,
    pub age_hours: f64,
}

/// Resource constraints for prioritization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConstraints {
    /// Maximum concurrent agents.
    pub max_concurrent_agents: usize,
    /// Current agent count.
    pub current_agents: usize,
    /// Token budget remaining (if any).
    pub token_budget_remaining: Option<u64>,
    /// Time constraints (deadline hours from now).
    pub time_constraints: Vec<(Uuid, f64)>,
}

/// A known conflict between goals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalConflict {
    pub goal_a: Uuid,
    pub goal_b: Uuid,
    pub conflict_type: String,
    pub description: String,
}

/// Request to analyze a capability gap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGapRequest {
    /// The task that revealed the gap.
    pub task_id: Uuid,
    /// Task description.
    pub task_description: String,
    /// Required capabilities that are missing.
    pub missing_capabilities: Vec<String>,
    /// Available agent types.
    pub available_agents: Vec<AgentCapabilitySummary>,
    /// Recent similar gaps (for pattern detection).
    pub similar_gaps: Vec<String>,
}

/// Summary of an agent's capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilitySummary {
    pub name: String,
    pub tier: String,
    pub capabilities: Vec<String>,
    pub tools: Vec<String>,
}

/// Request to resolve a conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionRequest {
    /// The type of conflict.
    pub conflict_type: ConflictType,
    /// Parties involved in the conflict.
    pub parties: Vec<ConflictParty>,
    /// Context about the conflict.
    pub context: String,
    /// Previous resolution attempts.
    pub previous_attempts: Vec<String>,
}

/// Type of conflict to resolve.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Two tasks modifying the same resource.
    ResourceContention,
    /// Circular or conflicting dependencies.
    DependencyConflict,
    /// Two agents claiming the same work.
    AgentContention,
    /// Conflicting goal requirements.
    GoalConflict,
    /// Merge conflict in code.
    MergeConflict,
}

/// A party involved in a conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictParty {
    /// Type of party (task, agent, goal).
    pub party_type: String,
    /// ID of the party.
    pub id: Uuid,
    /// Name/title of the party.
    pub name: String,
    /// What this party wants/needs.
    pub interest: String,
}

/// Request to recover from a stuck state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StuckStateRecoveryRequest {
    /// The stuck task.
    pub task_id: Uuid,
    /// Task title.
    pub task_title: String,
    /// Task description.
    pub task_description: String,
    /// Goal context.
    pub goal_context: GoalContext,
    /// Failure history.
    pub failure_history: Vec<FailureRecord>,
    /// Previous recovery attempts.
    pub previous_recovery_attempts: Vec<RecoveryAttempt>,
    /// Available alternative approaches.
    pub available_approaches: Vec<String>,
}

/// Context about the goal for a stuck task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalContext {
    pub goal_id: Uuid,
    pub goal_name: String,
    pub goal_description: String,
    pub other_tasks_status: String,
}

/// Record of a task failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRecord {
    pub attempt: u32,
    pub timestamp: DateTime<Utc>,
    pub error: String,
    pub agent_type: String,
    pub turns_used: u32,
}

/// Record of a recovery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryAttempt {
    pub attempt: u32,
    pub strategy: String,
    pub outcome: String,
}

/// Request to evaluate whether to escalate to human.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRequest {
    /// Context of the situation.
    pub context: EscalationContext,
    /// What triggered the escalation consideration.
    pub trigger: EscalationTrigger,
    /// Previous escalations in this session.
    pub previous_escalations: Vec<PreviousEscalation>,
    /// User preferences about escalation.
    pub escalation_preferences: EscalationPreferences,
}

/// Context for an escalation decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationContext {
    /// Related goal.
    pub goal_id: Option<Uuid>,
    /// Related task.
    pub task_id: Option<Uuid>,
    /// Description of what's happening.
    pub situation: String,
    /// What has been tried.
    pub attempts_made: Vec<String>,
    /// Time spent on this issue.
    pub time_spent_minutes: u32,
}

/// What triggered the escalation consideration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationTrigger {
    /// All retries exhausted.
    RetriesExhausted,
    /// Ambiguous requirements.
    AmbiguousRequirements,
    /// Security concern.
    SecurityConcern,
    /// High-risk operation.
    HighRiskOperation,
    /// Indeterminate verification result.
    IndeterminateVerification,
    /// Conflicting constraints.
    ConflictingConstraints,
    /// External dependency failure.
    ExternalDependencyFailure,
}

/// Record of a previous escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviousEscalation {
    pub timestamp: DateTime<Utc>,
    pub reason: String,
    pub human_response: Option<String>,
    pub outcome: String,
}

/// User preferences about escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationPreferences {
    /// Whether user prefers autonomous operation.
    pub prefer_autonomous: bool,
    /// Maximum cost before escalating (cents).
    pub max_cost_before_escalation: Option<f64>,
    /// Whether to escalate on security concerns.
    pub escalate_on_security: bool,
    /// Escalation cooldown in minutes.
    pub cooldown_minutes: u32,
}

impl Default for EscalationPreferences {
    fn default() -> Self {
        Self {
            prefer_autonomous: true,
            max_cost_before_escalation: None,
            escalate_on_security: true,
            cooldown_minutes: 30,
        }
    }
}

// ============================================================================
// Decision Types
// ============================================================================

/// All possible decisions from the Overmind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision_type", rename_all = "snake_case")]
pub enum OvermindDecision {
    /// Decision about goal decomposition.
    GoalDecomposition(GoalDecompositionDecision),
    /// Decision about goal prioritization.
    Prioritization(PrioritizationDecision),
    /// Decision about capability gaps.
    CapabilityGap(CapabilityGapDecision),
    /// Decision about conflict resolution.
    ConflictResolution(ConflictResolutionDecision),
    /// Decision about stuck state recovery.
    StuckStateRecovery(StuckStateRecoveryDecision),
    /// Decision about escalation.
    Escalation(OvermindEscalationDecision),
}

impl OvermindDecision {
    /// Get metadata about this decision.
    pub fn metadata(&self) -> &DecisionMetadata {
        match self {
            Self::GoalDecomposition(d) => &d.metadata,
            Self::Prioritization(d) => &d.metadata,
            Self::CapabilityGap(d) => &d.metadata,
            Self::ConflictResolution(d) => &d.metadata,
            Self::StuckStateRecovery(d) => &d.metadata,
            Self::Escalation(d) => &d.metadata,
        }
    }
}

/// Metadata attached to every decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionMetadata {
    /// Unique decision ID.
    pub decision_id: Uuid,
    /// Confidence in this decision (0.0 - 1.0).
    pub confidence: f64,
    /// Rationale for the decision.
    pub rationale: String,
    /// Alternative options considered.
    pub alternatives_considered: Vec<String>,
    /// Risks of this decision.
    pub risks: Vec<String>,
    /// When this decision was made.
    pub decided_at: DateTime<Utc>,
}

impl Default for DecisionMetadata {
    fn default() -> Self {
        Self {
            decision_id: Uuid::new_v4(),
            confidence: 0.0,
            rationale: String::new(),
            alternatives_considered: Vec::new(),
            risks: Vec::new(),
            decided_at: Utc::now(),
        }
    }
}

impl DecisionMetadata {
    pub fn new(confidence: f64, rationale: impl Into<String>) -> Self {
        Self {
            decision_id: Uuid::new_v4(),
            confidence,
            rationale: rationale.into(),
            alternatives_considered: Vec::new(),
            risks: Vec::new(),
            decided_at: Utc::now(),
        }
    }

    pub fn with_alternatives(mut self, alternatives: Vec<String>) -> Self {
        self.alternatives_considered = alternatives;
        self
    }

    pub fn with_risks(mut self, risks: Vec<String>) -> Self {
        self.risks = risks;
        self
    }
}

/// Decision about how to decompose a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalDecompositionDecision {
    /// Decision metadata.
    pub metadata: DecisionMetadata,
    /// Recommended decomposition strategy.
    pub strategy: DecompositionStrategy,
    /// Tasks to create.
    pub tasks: Vec<TaskDefinition>,
    /// Key verification points during execution.
    pub verification_points: Vec<VerificationPoint>,
    /// Suggested execution order hints.
    pub execution_hints: Vec<String>,
}

/// Strategy for decomposing a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecompositionStrategy {
    /// Sequential tasks with clear dependencies.
    Sequential,
    /// Parallel independent tasks.
    Parallel,
    /// Mix of sequential and parallel.
    Hybrid,
    /// Research first, then implement.
    ResearchFirst,
    /// Incremental delivery.
    Incremental,
}

/// Definition of a task to create.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDefinition {
    /// Task title.
    pub title: String,
    /// Task description.
    pub description: String,
    /// Suggested agent type.
    pub agent_type: Option<String>,
    /// Task priority.
    pub priority: TaskPriority,
    /// Dependencies (titles of other tasks).
    pub depends_on: Vec<String>,
    /// Whether this task needs a worktree.
    pub needs_worktree: bool,
    /// Estimated complexity (1-5).
    pub estimated_complexity: u8,
    /// Acceptance criteria.
    pub acceptance_criteria: Vec<String>,
}

/// A point where verification should occur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPoint {
    /// After which task(s).
    pub after_tasks: Vec<String>,
    /// What to verify.
    pub verify: String,
    /// Whether blocking (must pass to continue).
    pub is_blocking: bool,
}

/// Decision about goal prioritization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrioritizationDecision {
    /// Decision metadata.
    pub metadata: DecisionMetadata,
    /// Ordered list of goal IDs by priority.
    pub priority_order: Vec<Uuid>,
    /// Resource allocation per goal.
    pub resource_allocation: Vec<ResourceAllocation>,
    /// Identified conflicts and resolutions.
    pub conflict_resolutions: Vec<ConflictResolution>,
    /// Goals that should be paused.
    pub goals_to_pause: Vec<Uuid>,
}

/// Resource allocation for a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAllocation {
    pub goal_id: Uuid,
    /// Percentage of agent capacity (0-100).
    pub agent_capacity_percent: u8,
    /// Priority tier (1 = highest).
    pub priority_tier: u8,
}

/// Resolution for a goal conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolution {
    pub goal_a: Uuid,
    pub goal_b: Uuid,
    pub resolution: String,
    pub winner: Option<Uuid>,
}

/// Decision about a capability gap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGapDecision {
    /// Decision metadata.
    pub metadata: DecisionMetadata,
    /// Recommended action.
    pub action: CapabilityGapAction,
    /// If creating agent, the spec.
    pub new_agent_spec: Option<NewAgentSpec>,
    /// If extending agent, which one and how.
    pub agent_extension: Option<AgentExtension>,
}

/// Action to take for a capability gap.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGapAction {
    /// Create a new agent type.
    CreateAgent,
    /// Extend an existing agent.
    ExtendAgent,
    /// Decompose task to use existing agents.
    DecomposeTask,
    /// Escalate to human.
    Escalate,
    /// Use generic worker with instructions.
    UseGenericWorker,
}

/// Specification for a new agent to create.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAgentSpec {
    pub name: String,
    pub description: String,
    pub tier: String,
    pub capabilities: Vec<String>,
    pub tools: Vec<String>,
    pub system_prompt_guidance: String,
}

/// How to extend an existing agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtension {
    pub agent_name: String,
    pub add_capabilities: Vec<String>,
    pub add_tools: Vec<String>,
    pub prompt_additions: String,
}

/// Decision about conflict resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionDecision {
    /// Decision metadata.
    pub metadata: DecisionMetadata,
    /// Resolution approach.
    pub approach: ConflictResolutionApproach,
    /// Task modifications required.
    pub task_modifications: Vec<TaskModification>,
    /// Whether any parties need notification.
    pub notifications: Vec<String>,
}

/// Approach to resolving a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionApproach {
    /// One party wins, other yields.
    PriorityBased { winner: Uuid },
    /// Merge the competing interests.
    Merge,
    /// Serialize the conflicting operations.
    Serialize,
    /// Create separate paths for each.
    Isolate,
    /// Neither can proceed, escalate.
    Escalate,
}

/// Modification to make to a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskModification {
    pub task_id: Uuid,
    pub modification_type: TaskModificationType,
    pub description: String,
}

/// Type of task modification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskModificationType {
    /// Add a dependency.
    AddDependency { depends_on: Uuid },
    /// Remove a dependency.
    RemoveDependency { dependency: Uuid },
    /// Change priority.
    ChangePriority { new_priority: TaskPriority },
    /// Update description.
    UpdateDescription { new_description: String },
    /// Cancel the task.
    Cancel,
    /// Pause the task.
    Pause,
}

/// Decision about stuck state recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StuckStateRecoveryDecision {
    /// Decision metadata.
    pub metadata: DecisionMetadata,
    /// Root cause analysis.
    pub root_cause: RootCause,
    /// Recovery action to take.
    pub recovery_action: RecoveryAction,
    /// New tasks to create (if any).
    pub new_tasks: Vec<TaskDefinition>,
    /// Whether the original task should be cancelled.
    pub cancel_original: bool,
}

/// Root cause of a stuck state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCause {
    /// Category of the root cause.
    pub category: RootCauseCategory,
    /// Detailed explanation.
    pub explanation: String,
    /// Evidence supporting this diagnosis.
    pub evidence: Vec<String>,
}

/// Category of root cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootCauseCategory {
    /// Missing information or unclear requirements.
    InformationGap,
    /// Technical limitation or bug.
    TechnicalIssue,
    /// Wrong approach or strategy.
    WrongApproach,
    /// External dependency problem.
    ExternalDependency,
    /// Resource constraints.
    ResourceConstraint,
    /// Task is fundamentally impossible.
    Impossible,
}

/// Action to recover from stuck state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Retry with a different approach.
    RetryDifferentApproach { approach: String, agent_type: Option<String> },
    /// Decompose the task differently.
    Redecompose,
    /// Research the problem first.
    ResearchFirst { research_questions: Vec<String> },
    /// Wait for external condition.
    WaitFor { condition: String, check_interval_mins: u32 },
    /// Escalate to human.
    Escalate { reason: String },
    /// Accept failure and move on.
    AcceptFailure { reason: String },
}

/// Decision about whether to escalate (from Overmind).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvermindEscalationDecision {
    /// Decision metadata.
    pub metadata: DecisionMetadata,
    /// Whether to escalate.
    pub should_escalate: bool,
    /// If escalating, the urgency.
    pub urgency: Option<OvermindEscalationUrgency>,
    /// Questions to ask the human.
    pub questions: Vec<String>,
    /// Context to provide.
    pub context_for_human: String,
    /// Alternatives if human is unavailable.
    pub alternatives_if_unavailable: Vec<String>,
    /// Whether this blocks progress.
    pub is_blocking: bool,
}

/// Urgency level for Overmind escalation decisions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OvermindEscalationUrgency {
    /// Can wait, proceed with best guess if needed.
    Low,
    /// Should address soon but not blocking.
    Medium,
    /// Blocking progress, needs attention.
    High,
    /// Critical issue, immediate attention needed.
    Critical,
}

impl OvermindEscalationUrgency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_metadata_builder() {
        let meta = DecisionMetadata::new(0.85, "This is the best approach")
            .with_alternatives(vec!["Option B".to_string(), "Option C".to_string()])
            .with_risks(vec!["Risk 1".to_string()]);

        assert_eq!(meta.confidence, 0.85);
        assert_eq!(meta.alternatives_considered.len(), 2);
        assert_eq!(meta.risks.len(), 1);
    }

    #[test]
    fn test_request_serialization() {
        let request = OvermindRequest::GoalDecomposition(GoalDecompositionRequest {
            goal_id: Uuid::new_v4(),
            goal_name: "Test Goal".to_string(),
            goal_description: "A test goal".to_string(),
            constraints: vec!["Must be fast".to_string()],
            available_agents: vec!["worker".to_string()],
            existing_tasks: vec![],
            memory_patterns: vec![],
            max_tasks: 10,
        });

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("goal_decomposition"));

        let parsed: OvermindRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.request_type_name(), "goal_decomposition");
    }

    #[test]
    fn test_decision_serialization() {
        let decision = OvermindDecision::Escalation(OvermindEscalationDecision {
            metadata: DecisionMetadata::new(0.9, "Human input needed"),
            should_escalate: true,
            urgency: Some(OvermindEscalationUrgency::High),
            questions: vec!["What API key to use?".to_string()],
            context_for_human: "Trying to connect to external service".to_string(),
            alternatives_if_unavailable: vec!["Skip this step".to_string()],
            is_blocking: true,
        });

        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains("escalation"));

        let parsed: OvermindDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.metadata().confidence, 0.9);
    }

    #[test]
    fn test_task_definition() {
        let task = TaskDefinition {
            title: "Implement feature X".to_string(),
            description: "Full description".to_string(),
            agent_type: Some("code-implementer".to_string()),
            priority: TaskPriority::High,
            depends_on: vec!["Design feature X".to_string()],
            needs_worktree: true,
            estimated_complexity: 3,
            acceptance_criteria: vec!["Tests pass".to_string()],
        };

        let json = serde_json::to_string(&task).unwrap();
        let parsed: TaskDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "Implement feature X");
        assert_eq!(parsed.estimated_complexity, 3);
    }
}

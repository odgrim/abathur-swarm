//! Intent Verification domain model.
//!
//! Captures the concept of verifying that completed work satisfies the
//! original intent, not just the derived checklist. This enables convergence
//! loops where work can be re-evaluated and refined.
//!
//! ## Key Principles
//!
//! 1. **Goals are convergent attractors** - they are never "completed."
//!    Verification happens at the task/wave level, not goal level.
//!
//! 2. **The re-prompt test**: "If someone submitted the exact same prompt again,
//!    would there be additional work done?" If yes, intent is not satisfied.
//!
//! 3. **Semantic drift detection**: If the same gaps keep appearing across
//!    iterations, we're not making progress and should escalate or restructure.
//!
//! ## Verification Hierarchy
//!
//! - Task verification: Single task against its description
//! - Wave verification: Batch of concurrent tasks
//! - Branch verification: Dependency chain sub-objective
//! - Intent alignment: Tasks against the guiding goal's intent (but never "goal completion")

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Result of an intent verification evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSatisfaction {
    /// Work fully satisfies the original intent
    Satisfied,
    /// Work partially satisfies intent but has gaps
    Partial,
    /// Work does not satisfy the intent
    Unsatisfied,
    /// Unable to determine (needs human input)
    Indeterminate,
}

impl Default for IntentSatisfaction {
    fn default() -> Self {
        Self::Indeterminate
    }
}

impl IntentSatisfaction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Partial => "partial",
            Self::Unsatisfied => "unsatisfied",
            Self::Indeterminate => "indeterminate",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "satisfied" => Some(Self::Satisfied),
            "partial" => Some(Self::Partial),
            "unsatisfied" => Some(Self::Unsatisfied),
            "indeterminate" => Some(Self::Indeterminate),
            _ => None,
        }
    }

    /// Whether this result indicates convergence (no more work needed).
    pub fn is_converged(&self) -> bool {
        matches!(self, Self::Satisfied)
    }

    /// Whether this result indicates re-prompting may help.
    pub fn should_retry(&self) -> bool {
        matches!(self, Self::Partial | Self::Unsatisfied)
    }
}

/// A gap identified between the work done and the original intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentGap {
    /// Description of what's missing or incomplete
    pub description: String,
    /// Severity of the gap
    pub severity: GapSeverity,
    /// Category of the gap (functional, error_handling, security, etc.)
    pub category: GapCategory,
    /// Suggested action to address the gap
    pub suggested_action: Option<String>,
    /// Which task(s) this gap relates to
    pub related_tasks: Vec<Uuid>,
    /// Whether this gap was from implicit (unstated) requirements
    pub is_implicit: bool,
    /// Why this was expected (for implicit gaps)
    pub implicit_rationale: Option<String>,
    /// Embedding vector for semantic similarity (populated by embedding service)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl IntentGap {
    pub fn new(description: impl Into<String>, severity: GapSeverity) -> Self {
        Self {
            description: description.into(),
            severity,
            category: GapCategory::Functional,
            suggested_action: None,
            related_tasks: Vec::new(),
            is_implicit: false,
            implicit_rationale: None,
            embedding: None,
        }
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_action = Some(action.into());
        self
    }

    pub fn with_task(mut self, task_id: Uuid) -> Self {
        self.related_tasks.push(task_id);
        self
    }

    pub fn with_category(mut self, category: GapCategory) -> Self {
        self.category = category;
        self
    }

    pub fn as_implicit(mut self, rationale: impl Into<String>) -> Self {
        self.is_implicit = true;
        self.implicit_rationale = Some(rationale.into());
        self
    }

    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }
}

/// Category of an intent gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GapCategory {
    /// Missing features or behaviors
    #[default]
    Functional,
    /// Missing or inadequate error handling
    ErrorHandling,
    /// Doesn't work with other components
    Integration,
    /// Insufficient test coverage
    Testing,
    /// Security vulnerabilities or concerns
    Security,
    /// Performance issues or concerns
    Performance,
    /// Missing logging, metrics, or monitoring
    Observability,
    /// Missing or inadequate documentation
    Documentation,
    /// Code quality or design issues
    Maintainability,
}

impl GapCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Functional => "functional",
            Self::ErrorHandling => "error_handling",
            Self::Integration => "integration",
            Self::Testing => "testing",
            Self::Security => "security",
            Self::Performance => "performance",
            Self::Observability => "observability",
            Self::Documentation => "documentation",
            Self::Maintainability => "maintainability",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "functional" => Some(Self::Functional),
            "error_handling" | "errorhandling" => Some(Self::ErrorHandling),
            "integration" => Some(Self::Integration),
            "testing" => Some(Self::Testing),
            "security" => Some(Self::Security),
            "performance" => Some(Self::Performance),
            "observability" => Some(Self::Observability),
            "documentation" => Some(Self::Documentation),
            "maintainability" => Some(Self::Maintainability),
            _ => None,
        }
    }

    /// Whether this category typically requires human judgment
    pub fn typically_needs_human(&self) -> bool {
        matches!(self, Self::Security | Self::Integration)
    }
}

/// Severity of an intent gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapSeverity {
    /// Minor gap, work is still acceptable
    Minor,
    /// Moderate gap, should be addressed
    Moderate,
    /// Major gap, must be addressed
    Major,
    /// Critical gap, work is fundamentally wrong
    Critical,
}

impl Default for GapSeverity {
    fn default() -> Self {
        Self::Moderate
    }
}

impl GapSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Moderate => "moderate",
            Self::Major => "major",
            Self::Critical => "critical",
        }
    }
}

/// The original intent captured from a goal or prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OriginalIntent {
    /// Unique identifier
    pub id: Uuid,
    /// The source of this intent (goal_id, user prompt, etc.)
    pub source_id: Uuid,
    /// Type of source
    pub source_type: IntentSource,
    /// The original text/description of the intent
    pub original_text: String,
    /// Key requirements extracted from the intent
    pub key_requirements: Vec<String>,
    /// Success criteria (what would make this "done")
    pub success_criteria: Vec<String>,
    /// When this intent was captured
    pub captured_at: DateTime<Utc>,
}

impl OriginalIntent {
    pub fn from_goal(goal_id: Uuid, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_id: goal_id,
            source_type: IntentSource::Goal,
            original_text: description.into(),
            key_requirements: Vec::new(),
            success_criteria: Vec::new(),
            captured_at: Utc::now(),
        }
    }

    pub fn from_task(task_id: Uuid, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_id: task_id,
            source_type: IntentSource::Task,
            original_text: description.into(),
            key_requirements: Vec::new(),
            success_criteria: Vec::new(),
            captured_at: Utc::now(),
        }
    }

    pub fn with_requirement(mut self, req: impl Into<String>) -> Self {
        self.key_requirements.push(req.into());
        self
    }

    pub fn with_success_criterion(mut self, criterion: impl Into<String>) -> Self {
        self.success_criteria.push(criterion.into());
        self
    }
}

/// Source of an intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSource {
    /// Intent from a goal
    Goal,
    /// Intent from a task
    Task,
    /// Intent from a user prompt
    UserPrompt,
    /// Intent from a DAG branch
    DagBranch,
}

impl IntentSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Task => "task",
            Self::UserPrompt => "user_prompt",
            Self::DagBranch => "dag_branch",
        }
    }
}

/// Result of an intent verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentVerificationResult {
    /// Unique identifier
    pub id: Uuid,
    /// The intent being verified
    pub intent_id: Uuid,
    /// Overall satisfaction level
    pub satisfaction: IntentSatisfaction,
    /// Confidence in this evaluation (0.0-1.0)
    pub confidence: f64,
    /// Gaps identified (explicit requirements)
    pub gaps: Vec<IntentGap>,
    /// Implicit gaps (unstated but expected requirements)
    pub implicit_gaps: Vec<IntentGap>,
    /// Tasks that were evaluated
    pub evaluated_tasks: Vec<Uuid>,
    /// Summary of what was accomplished
    pub accomplishment_summary: String,
    /// Re-prompting guidance if needed
    pub reprompt_guidance: Option<RepromptGuidance>,
    /// Iteration number (how many times this intent has been verified)
    pub iteration: u32,
    /// When this verification was performed
    pub verified_at: DateTime<Utc>,
    /// Human escalation information
    pub escalation: Option<HumanEscalation>,
}

/// Information about why human judgment is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanEscalation {
    /// Whether human judgment is needed
    pub needs_human: bool,
    /// Reason human judgment is needed
    pub reason: String,
    /// Urgency of the escalation
    pub urgency: EscalationUrgency,
    /// Specific questions for the human
    pub questions: Vec<String>,
    /// What will happen if human doesn't respond (default action)
    pub default_action: Option<String>,
    /// Deadline for human response (if any)
    pub deadline: Option<DateTime<Utc>>,
    /// Context to help human make decision
    pub decision_context: String,
}

impl HumanEscalation {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            needs_human: true,
            reason: reason.into(),
            urgency: EscalationUrgency::Normal,
            questions: Vec::new(),
            default_action: None,
            deadline: None,
            decision_context: String::new(),
        }
    }

    pub fn with_urgency(mut self, urgency: EscalationUrgency) -> Self {
        self.urgency = urgency;
        self
    }

    pub fn with_question(mut self, question: impl Into<String>) -> Self {
        self.questions.push(question.into());
        self
    }

    pub fn with_default_action(mut self, action: impl Into<String>) -> Self {
        self.default_action = Some(action.into());
        self
    }

    pub fn with_deadline(mut self, deadline: DateTime<Utc>) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.decision_context = context.into();
        self
    }

    /// Create escalation for ambiguous requirements
    pub fn ambiguous_requirements(description: impl Into<String>) -> Self {
        Self::new("Ambiguous requirements require human clarification")
            .with_urgency(EscalationUrgency::Normal)
            .with_context(description)
    }

    /// Create escalation for security-sensitive decisions
    pub fn security_decision(description: impl Into<String>) -> Self {
        Self::new("Security-sensitive decision requires human authorization")
            .with_urgency(EscalationUrgency::High)
            .with_context(description)
    }

    /// Create escalation for policy decisions
    pub fn policy_decision(description: impl Into<String>) -> Self {
        Self::new("Policy or business logic decision not specified")
            .with_urgency(EscalationUrgency::Normal)
            .with_context(description)
    }

    /// Create escalation for recurring drift
    pub fn recurring_drift(gaps: &[String]) -> Self {
        let gap_list = gaps.join(", ");
        Self::new("Semantic drift detected - same gaps recurring across iterations")
            .with_urgency(EscalationUrgency::High)
            .with_context(format!("Recurring gaps: {}", gap_list))
            .with_question("Are these gaps actually important, or should they be accepted?")
            .with_question("Is the original intent correctly understood?")
            .with_default_action("Continue with current approach after 3 more iterations")
    }

    /// Create escalation for access/permission issues
    pub fn access_required(description: impl Into<String>) -> Self {
        Self::new("Access or permissions required that the system lacks")
            .with_urgency(EscalationUrgency::Blocking)
            .with_context(description)
    }
}

/// Urgency level for human escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EscalationUrgency {
    /// Can wait, not blocking progress
    Low,
    /// Should be addressed soon
    #[default]
    Normal,
    /// Important, affects quality
    High,
    /// Blocking progress, cannot continue without human input
    Blocking,
}

impl EscalationUrgency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Blocking => "blocking",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            "blocking" => Some(Self::Blocking),
            _ => None,
        }
    }
}

impl IntentVerificationResult {
    pub fn new(intent_id: Uuid, satisfaction: IntentSatisfaction) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            satisfaction,
            confidence: 0.0,
            gaps: Vec::new(),
            implicit_gaps: Vec::new(),
            evaluated_tasks: Vec::new(),
            accomplishment_summary: String::new(),
            reprompt_guidance: None,
            iteration: 1,
            verified_at: Utc::now(),
            escalation: None,
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_gap(mut self, gap: IntentGap) -> Self {
        self.gaps.push(gap);
        self
    }

    pub fn with_task(mut self, task_id: Uuid) -> Self {
        self.evaluated_tasks.push(task_id);
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.accomplishment_summary = summary.into();
        self
    }

    pub fn with_reprompt_guidance(mut self, guidance: RepromptGuidance) -> Self {
        self.reprompt_guidance = Some(guidance);
        self
    }

    pub fn with_iteration(mut self, iteration: u32) -> Self {
        self.iteration = iteration;
        self
    }

    pub fn with_implicit_gap(mut self, gap: IntentGap) -> Self {
        self.implicit_gaps.push(gap);
        self
    }

    pub fn with_escalation(mut self, escalation: HumanEscalation) -> Self {
        self.escalation = Some(escalation);
        self
    }

    /// Check if we should attempt another iteration.
    pub fn should_iterate(&self) -> bool {
        // Don't iterate if human escalation is blocking
        if let Some(ref esc) = self.escalation {
            if esc.needs_human && esc.urgency == EscalationUrgency::Blocking {
                return false;
            }
        }
        self.satisfaction.should_retry() && self.reprompt_guidance.is_some()
    }

    /// Check if human judgment is required.
    pub fn needs_human(&self) -> bool {
        self.escalation.as_ref().map_or(false, |e| e.needs_human)
    }

    /// Check if progress is blocked pending human input.
    pub fn is_blocked_on_human(&self) -> bool {
        self.escalation.as_ref().map_or(false, |e| {
            e.needs_human && e.urgency == EscalationUrgency::Blocking
        })
    }

    /// Get the most severe gap, if any.
    pub fn most_severe_gap(&self) -> Option<&IntentGap> {
        self.gaps.iter().max_by_key(|g| g.severity)
    }

    /// Get the most severe gap from either explicit or implicit gaps.
    pub fn most_severe_any_gap(&self) -> Option<&IntentGap> {
        self.gaps.iter()
            .chain(self.implicit_gaps.iter())
            .max_by_key(|g| g.severity)
    }

    /// Get all gaps (explicit and implicit) combined.
    pub fn all_gaps(&self) -> impl Iterator<Item = &IntentGap> {
        self.gaps.iter().chain(self.implicit_gaps.iter())
    }

    /// Count of all gaps.
    pub fn total_gap_count(&self) -> usize {
        self.gaps.len() + self.implicit_gaps.len()
    }

    /// Check if there are any critical gaps.
    pub fn has_critical_gaps(&self) -> bool {
        self.all_gaps().any(|g| g.severity == GapSeverity::Critical)
    }

    /// Check if there are any security-related gaps.
    pub fn has_security_gaps(&self) -> bool {
        self.all_gaps().any(|g| g.category == GapCategory::Security)
    }

    /// Get gaps by category.
    pub fn gaps_by_category(&self, category: GapCategory) -> Vec<&IntentGap> {
        self.all_gaps().filter(|g| g.category == category).collect()
    }

    /// Determine if this result should trigger human escalation based on gap patterns.
    pub fn should_escalate(&self) -> Option<HumanEscalation> {
        // Critical security gaps always escalate
        if self.has_security_gaps() && self.gaps_by_category(GapCategory::Security)
            .iter().any(|g| g.severity >= GapSeverity::Major)
        {
            return Some(HumanEscalation::security_decision(
                "Security-related gaps require human review"
            ));
        }

        // Many implicit gaps suggest requirements ambiguity
        if self.implicit_gaps.len() >= 3 {
            return Some(HumanEscalation::ambiguous_requirements(
                format!("Multiple implicit requirements ({}) were missed, suggesting the original intent may be unclear",
                    self.implicit_gaps.len())
            ));
        }

        // Already has explicit escalation
        self.escalation.clone()
    }
}

/// Guidance for re-prompting to address gaps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepromptGuidance {
    /// What specifically needs to be done
    pub focus_areas: Vec<String>,
    /// Tasks that should be re-executed
    pub tasks_to_retry: Vec<Uuid>,
    /// Tasks that should be added
    pub tasks_to_add: Vec<NewTaskGuidance>,
    /// Additional context to provide
    pub additional_context: String,
    /// Recommended approach for the re-prompt
    pub approach: RepromptApproach,
}

impl RepromptGuidance {
    pub fn new(approach: RepromptApproach) -> Self {
        Self {
            focus_areas: Vec::new(),
            tasks_to_retry: Vec::new(),
            tasks_to_add: Vec::new(),
            additional_context: String::new(),
            approach,
        }
    }

    pub fn with_focus(mut self, area: impl Into<String>) -> Self {
        self.focus_areas.push(area.into());
        self
    }

    pub fn with_retry(mut self, task_id: Uuid) -> Self {
        self.tasks_to_retry.push(task_id);
        self
    }

    pub fn with_new_task(mut self, task: NewTaskGuidance) -> Self {
        self.tasks_to_add.push(task);
        self
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.additional_context = context.into();
        self
    }
}

/// Guidance for a new task to add.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewTaskGuidance {
    /// Suggested title
    pub title: String,
    /// Suggested description
    pub description: String,
    /// Priority relative to existing tasks
    pub priority: TaskGuidancePriority,
    /// Dependencies on existing tasks
    pub depends_on: Vec<Uuid>,
    /// Whether this task blocks other work or can run in parallel
    pub execution_mode: TaskExecutionMode,
    /// Gap category this task addresses
    pub addresses_category: Option<GapCategory>,
}

impl NewTaskGuidance {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            priority: TaskGuidancePriority::Normal,
            depends_on: Vec::new(),
            execution_mode: TaskExecutionMode::Parallel,
            addresses_category: None,
        }
    }

    pub fn high_priority(mut self) -> Self {
        self.priority = TaskGuidancePriority::High;
        self
    }

    pub fn with_dependency(mut self, task_id: Uuid) -> Self {
        self.depends_on.push(task_id);
        self
    }

    pub fn blocking(mut self) -> Self {
        self.execution_mode = TaskExecutionMode::Blocking;
        self
    }

    pub fn for_category(mut self, category: GapCategory) -> Self {
        self.addresses_category = Some(category);
        self
    }
}

/// How a new task should be executed relative to others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecutionMode {
    /// Can run in parallel with other tasks
    #[default]
    Parallel,
    /// Must complete before other tasks can proceed
    Blocking,
}

/// Priority for task guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskGuidancePriority {
    Low,
    Normal,
    High,
}

/// Approach for re-prompting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepromptApproach {
    /// Retry the exact same prompt - agent may have misunderstood
    RetrySame,
    /// Retry the same tasks with additional context and emphasis
    RetryWithContext,
    /// Retry with augmented prompts that include gap information
    RetryAugmented {
        /// Specific instructions to add to each task
        augmentation_instructions: String,
    },
    /// Add new tasks to address gaps (don't retry existing)
    AddTasks,
    /// Both retry existing and add new tasks
    RetryAndAddTasks,
    /// Restructure the entire approach - decomposition was wrong
    Restructure {
        /// What was wrong with the original approach
        original_problem: String,
        /// Suggested new approach
        suggested_approach: String,
    },
    /// Escalate to human - cannot proceed automatically
    Escalate {
        /// Reason for escalation
        reason: String,
    },
}

impl Default for RepromptApproach {
    fn default() -> Self {
        Self::RetryWithContext
    }
}

impl RepromptApproach {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RetrySame => "retry_same",
            Self::RetryWithContext => "retry_with_context",
            Self::RetryAugmented { .. } => "retry_augmented",
            Self::AddTasks => "add_tasks",
            Self::RetryAndAddTasks => "retry_and_add_tasks",
            Self::Restructure { .. } => "restructure",
            Self::Escalate { .. } => "escalate",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "retry_same" => Some(Self::RetrySame),
            "retry_with_context" | "retry_context" => Some(Self::RetryWithContext),
            "retry_augmented" => Some(Self::RetryAugmented {
                augmentation_instructions: String::new(),
            }),
            "add_tasks" => Some(Self::AddTasks),
            "retry_and_add_tasks" | "retry_and_add" => Some(Self::RetryAndAddTasks),
            "restructure" => Some(Self::Restructure {
                original_problem: String::new(),
                suggested_approach: String::new(),
            }),
            "escalate" => Some(Self::Escalate {
                reason: String::new(),
            }),
            _ => None,
        }
    }

    /// Whether this approach requires human involvement
    pub fn needs_human(&self) -> bool {
        matches!(self, Self::Escalate { .. })
    }

    /// Whether this approach involves restructuring work
    pub fn is_restructure(&self) -> bool {
        matches!(self, Self::Restructure { .. })
    }
}

/// Strategy selection based on gap analysis.
#[derive(Debug, Clone)]
pub struct RepromptStrategySelector;

impl RepromptStrategySelector {
    /// Select the best re-prompt strategy based on verification results.
    pub fn select_strategy(result: &IntentVerificationResult) -> RepromptApproach {
        // Check for escalation triggers first
        if let Some(ref escalation) = result.escalation {
            if escalation.needs_human && escalation.urgency == EscalationUrgency::Blocking {
                return RepromptApproach::Escalate {
                    reason: escalation.reason.clone(),
                };
            }
        }

        // Check for critical gaps - may need restructure
        if result.has_critical_gaps() {
            let critical_gaps: Vec<_> = result.all_gaps()
                .filter(|g| g.severity == GapSeverity::Critical)
                .collect();

            // If critical gaps are functional, likely need restructure
            if critical_gaps.iter().any(|g| g.category == GapCategory::Functional) {
                return RepromptApproach::Restructure {
                    original_problem: "Critical functional gaps indicate fundamental approach issues".to_string(),
                    suggested_approach: critical_gaps.iter()
                        .filter_map(|g| g.suggested_action.clone())
                        .collect::<Vec<_>>()
                        .join("; "),
                };
            }
        }

        // Check for many implicit gaps - may indicate misunderstanding
        if result.implicit_gaps.len() >= 3 {
            return RepromptApproach::RetryAugmented {
                augmentation_instructions: format!(
                    "Previous attempt missed {} implicit requirements. Focus on: {}",
                    result.implicit_gaps.len(),
                    result.implicit_gaps.iter()
                        .take(3)
                        .map(|g| g.description.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
        }

        // Security gaps get augmented retry with emphasis
        if result.has_security_gaps() {
            return RepromptApproach::RetryAugmented {
                augmentation_instructions: "SECURITY CRITICAL: Previous attempt had security gaps. Review and address security concerns carefully.".to_string(),
            };
        }

        // Count gap severities
        let major_count = result.all_gaps().filter(|g| g.severity == GapSeverity::Major).count();
        let moderate_count = result.all_gaps().filter(|g| g.severity == GapSeverity::Moderate).count();
        let minor_count = result.all_gaps().filter(|g| g.severity == GapSeverity::Minor).count();

        // Many major gaps - add tasks
        if major_count >= 2 {
            return RepromptApproach::RetryAndAddTasks;
        }

        // Mix of major and moderate - add tasks
        if major_count >= 1 && moderate_count >= 2 {
            return RepromptApproach::AddTasks;
        }

        // Just moderate gaps - retry with context
        if moderate_count >= 1 {
            return RepromptApproach::RetryWithContext;
        }

        // Only minor gaps - simple retry
        if minor_count >= 1 {
            return RepromptApproach::RetrySame;
        }

        // Default
        RepromptApproach::RetryWithContext
    }
}

/// Augmentation to apply to a pending task based on verification feedback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskAugmentation {
    /// The task ID to augment
    pub task_id: Uuid,
    /// Additional context to prepend to the task description
    pub context_prefix: String,
    /// Specific gaps this task should address
    pub gaps_to_address: Vec<String>,
    /// Focus areas from verification
    pub focus_areas: Vec<String>,
    /// Whether this is a retry of a previously failed task
    pub is_retry: bool,
    /// Previous attempt summary if this is a retry
    pub previous_attempt_summary: Option<String>,
}

impl TaskAugmentation {
    pub fn new(task_id: Uuid) -> Self {
        Self {
            task_id,
            context_prefix: String::new(),
            gaps_to_address: Vec::new(),
            focus_areas: Vec::new(),
            is_retry: false,
            previous_attempt_summary: None,
        }
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context_prefix = context.into();
        self
    }

    pub fn with_gap(mut self, gap: impl Into<String>) -> Self {
        self.gaps_to_address.push(gap.into());
        self
    }

    pub fn with_focus(mut self, area: impl Into<String>) -> Self {
        self.focus_areas.push(area.into());
        self
    }

    pub fn as_retry(mut self, previous_summary: impl Into<String>) -> Self {
        self.is_retry = true;
        self.previous_attempt_summary = Some(previous_summary.into());
        self
    }

    /// Format this augmentation as a prefix for the task description.
    pub fn format_as_description_prefix(&self) -> String {
        let mut prefix = String::new();

        if self.is_retry {
            prefix.push_str("**RETRY ATTEMPT**\n\n");
            if let Some(ref summary) = self.previous_attempt_summary {
                prefix.push_str(&format!("Previous attempt result: {}\n\n", summary));
            }
        }

        if !self.context_prefix.is_empty() {
            prefix.push_str(&format!("{}\n\n", self.context_prefix));
        }

        if !self.gaps_to_address.is_empty() {
            prefix.push_str("**Gaps to Address:**\n");
            for gap in &self.gaps_to_address {
                prefix.push_str(&format!("- {}\n", gap));
            }
            prefix.push('\n');
        }

        if !self.focus_areas.is_empty() {
            prefix.push_str("**Focus Areas:**\n");
            for area in &self.focus_areas {
                prefix.push_str(&format!("- {}\n", area));
            }
            prefix.push('\n');
        }

        if !prefix.is_empty() {
            prefix.push_str("---\n\n**Original Task:**\n");
        }

        prefix
    }
}

/// Build task augmentations from a verification result.
pub fn build_task_augmentations(
    verification: &IntentVerificationResult,
    pending_tasks: &[Uuid],
) -> Vec<TaskAugmentation> {
    let mut augmentations = Vec::new();

    // Get guidance if available
    let guidance = match &verification.reprompt_guidance {
        Some(g) => g,
        None => return augmentations,
    };

    // Augment retry tasks
    for task_id in &guidance.tasks_to_retry {
        if pending_tasks.contains(task_id) {
            let mut aug = TaskAugmentation::new(*task_id)
                .as_retry(format!(
                    "{} (confidence: {:.0}%)",
                    verification.satisfaction.as_str(),
                    verification.confidence * 100.0
                ));

            // Add relevant gaps
            for gap in &verification.gaps {
                if gap.related_tasks.contains(task_id) || gap.related_tasks.is_empty() {
                    aug = aug.with_gap(&gap.description);
                }
            }

            // Add focus areas
            for area in &guidance.focus_areas {
                aug = aug.with_focus(area);
            }

            augmentations.push(aug);
        }
    }

    // For other pending tasks, add general gap context
    for task_id in pending_tasks {
        if guidance.tasks_to_retry.contains(task_id) {
            continue; // Already handled above
        }

        // Only augment if there are relevant gaps or focus areas
        if verification.gaps.is_empty() && guidance.focus_areas.is_empty() {
            continue;
        }

        let mut aug = TaskAugmentation::new(*task_id);

        // Add gaps that aren't task-specific
        for gap in &verification.gaps {
            if gap.related_tasks.is_empty() {
                aug = aug.with_gap(&gap.description);
            }
        }

        // Add focus areas
        for area in &guidance.focus_areas {
            aug = aug.with_focus(area);
        }

        // Only include if we have something to add
        if !aug.gaps_to_address.is_empty() || !aug.focus_areas.is_empty() {
            augmentations.push(aug);
        }
    }

    augmentations
}

/// Configuration for the convergence loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceConfig {
    /// Maximum iterations before giving up
    pub max_iterations: u32,
    /// Minimum confidence to accept partial satisfaction
    pub min_confidence_threshold: f64,
    /// Whether to require explicit satisfaction (vs. partial)
    pub require_full_satisfaction: bool,
    /// Whether to automatically retry on partial satisfaction
    pub auto_retry_partial: bool,
    /// Timeout for the entire convergence loop (seconds)
    pub convergence_timeout_secs: u64,
}

impl Default for ConvergenceConfig {
    fn default() -> Self {
        Self {
            max_iterations: 3,
            min_confidence_threshold: 0.7,
            require_full_satisfaction: false,
            auto_retry_partial: true,
            convergence_timeout_secs: 7200, // 2 hours
        }
    }
}

impl ConvergenceConfig {
    /// Check if we should continue iterating.
    pub fn should_continue(&self, result: &IntentVerificationResult) -> bool {
        // Don't continue if we've hit max iterations
        if result.iteration >= self.max_iterations {
            return false;
        }

        // Don't continue if fully satisfied
        if result.satisfaction == IntentSatisfaction::Satisfied {
            return false;
        }

        // Don't continue if indeterminate (needs human)
        if result.satisfaction == IntentSatisfaction::Indeterminate {
            return false;
        }

        // For partial satisfaction, check config
        if result.satisfaction == IntentSatisfaction::Partial {
            if self.require_full_satisfaction {
                return true;
            }
            // Accept partial if confidence is high enough
            if result.confidence >= self.min_confidence_threshold {
                return false;
            }
            return self.auto_retry_partial;
        }

        // Unsatisfied - continue if we have guidance
        result.should_iterate()
    }
}

/// State of a convergence loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceState {
    /// The intent being converged on
    pub intent: OriginalIntent,
    /// History of verification results
    pub verification_history: Vec<IntentVerificationResult>,
    /// Current iteration number
    pub current_iteration: u32,
    /// Whether convergence has been achieved
    pub converged: bool,
    /// When the loop started
    pub started_at: DateTime<Utc>,
    /// When the loop ended (if done)
    pub ended_at: Option<DateTime<Utc>>,
    /// Detected semantic drift (same gaps recurring)
    pub drift_detected: bool,
    /// Gap fingerprints seen across iterations for drift detection
    pub gap_fingerprints: Vec<GapFingerprint>,
}

/// Fingerprint of a gap for semantic drift detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GapFingerprint {
    /// Normalized description (lowercase, trimmed)
    pub normalized_description: String,
    /// Severity of the gap
    pub severity: GapSeverity,
    /// Which iteration this gap was first seen
    pub first_seen_iteration: u32,
    /// How many times this gap has appeared
    pub occurrence_count: u32,
}

impl ConvergenceState {
    pub fn new(intent: OriginalIntent) -> Self {
        Self {
            intent,
            verification_history: Vec::new(),
            current_iteration: 0,
            converged: false,
            started_at: Utc::now(),
            ended_at: None,
            drift_detected: false,
            gap_fingerprints: Vec::new(),
        }
    }

    /// Record a verification result and update drift detection.
    pub fn record_verification(&mut self, result: IntentVerificationResult) {
        self.current_iteration = result.iteration;
        if result.satisfaction == IntentSatisfaction::Satisfied {
            self.converged = true;
            self.ended_at = Some(Utc::now());
        }

        // Update gap fingerprints for drift detection
        self.update_gap_fingerprints(&result);

        self.verification_history.push(result);
    }

    /// Update gap fingerprints and detect semantic drift.
    fn update_gap_fingerprints(&mut self, result: &IntentVerificationResult) {
        for gap in &result.gaps {
            let normalized = Self::normalize_gap_description(&gap.description);

            // Check if we've seen a similar gap before
            let existing = self.gap_fingerprints.iter_mut().find(|fp| {
                Self::gaps_are_similar(&fp.normalized_description, &normalized)
            });

            if let Some(fingerprint) = existing {
                fingerprint.occurrence_count += 1;
                // If same gap appears 3+ times, we have drift
                if fingerprint.occurrence_count >= 3 {
                    self.drift_detected = true;
                }
            } else {
                self.gap_fingerprints.push(GapFingerprint {
                    normalized_description: normalized,
                    severity: gap.severity,
                    first_seen_iteration: result.iteration,
                    occurrence_count: 1,
                });
            }
        }
    }

    /// Normalize a gap description for comparison.
    fn normalize_gap_description(description: &str) -> String {
        description
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Check if two gap descriptions are semantically similar.
    /// Uses simple word overlap heuristic.
    fn gaps_are_similar(a: &str, b: &str) -> bool {
        let words_a: std::collections::HashSet<_> = a.split_whitespace().collect();
        let words_b: std::collections::HashSet<_> = b.split_whitespace().collect();

        if words_a.is_empty() || words_b.is_empty() {
            return false;
        }

        let intersection = words_a.intersection(&words_b).count();
        let union = words_a.union(&words_b).count();

        // Jaccard similarity > 0.5 means similar
        (intersection as f64 / union as f64) > 0.5
    }

    /// Get the latest verification result.
    pub fn latest_result(&self) -> Option<&IntentVerificationResult> {
        self.verification_history.last()
    }

    /// Mark the loop as ended (even if not converged).
    pub fn end(&mut self) {
        if self.ended_at.is_none() {
            self.ended_at = Some(Utc::now());
        }
    }

    /// Check if we've made progress across iterations.
    pub fn is_making_progress(&self) -> bool {
        // If drift detected, we're not making progress
        if self.drift_detected {
            return false;
        }

        if self.verification_history.len() < 2 {
            return true; // Not enough data
        }

        let recent: Vec<_> = self.verification_history.iter().rev().take(2).collect();
        if recent.len() < 2 {
            return true;
        }

        // Check if gaps are decreasing
        let current_gaps = recent[0].gaps.len();
        let previous_gaps = recent[1].gaps.len();

        // Check if confidence is increasing
        let current_conf = recent[0].confidence;
        let previous_conf = recent[1].confidence;

        // Check if we're seeing different gaps (progress even if count same)
        let current_gap_set: std::collections::HashSet<_> = recent[0]
            .gaps
            .iter()
            .map(|g| Self::normalize_gap_description(&g.description))
            .collect();
        let previous_gap_set: std::collections::HashSet<_> = recent[1]
            .gaps
            .iter()
            .map(|g| Self::normalize_gap_description(&g.description))
            .collect();
        let gaps_changed = current_gap_set != previous_gap_set;

        current_gaps < previous_gaps || current_conf > previous_conf || gaps_changed
    }

    /// Get recurring gaps (those that have appeared multiple times).
    pub fn recurring_gaps(&self) -> Vec<&GapFingerprint> {
        self.gap_fingerprints
            .iter()
            .filter(|fp| fp.occurrence_count > 1)
            .collect()
    }

    /// Build context about the convergence state for agent prompts.
    pub fn build_iteration_context(&self) -> IterationContext {
        let recurring = self.recurring_gaps();
        let recurring_descriptions: Vec<String> = recurring
            .iter()
            .map(|fp| format!(
                "- {} (seen {} times, severity: {})",
                fp.normalized_description,
                fp.occurrence_count,
                fp.severity.as_str()
            ))
            .collect();

        let previous_attempts: Vec<String> = self.verification_history
            .iter()
            .map(|r| format!(
                "Iteration {}: {} (confidence: {:.0}%, {} gaps)",
                r.iteration,
                r.satisfaction.as_str(),
                r.confidence * 100.0,
                r.gaps.len()
            ))
            .collect();

        IterationContext {
            current_iteration: self.current_iteration + 1, // Next iteration
            total_iterations_so_far: self.current_iteration,
            drift_detected: self.drift_detected,
            recurring_gap_descriptions: recurring_descriptions,
            previous_attempt_summaries: previous_attempts,
            focus_areas: self.latest_result()
                .and_then(|r| r.reprompt_guidance.as_ref())
                .map(|g| g.focus_areas.clone())
                .unwrap_or_default(),
        }
    }
}

/// Context about the current convergence iteration for agent prompts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationContext {
    /// Which iteration this is (1-indexed)
    pub current_iteration: u32,
    /// How many iterations have been attempted
    pub total_iterations_so_far: u32,
    /// Whether semantic drift has been detected
    pub drift_detected: bool,
    /// Descriptions of gaps that keep recurring
    pub recurring_gap_descriptions: Vec<String>,
    /// Summaries of previous attempts
    pub previous_attempt_summaries: Vec<String>,
    /// Focus areas from latest verification
    pub focus_areas: Vec<String>,
}

impl IterationContext {
    /// Format as a section for agent system prompts.
    pub fn format_for_prompt(&self) -> String {
        if self.total_iterations_so_far == 0 {
            return String::new();
        }

        let mut context = String::from("\n\n## Convergence Loop Context\n\n");
        context.push_str(&format!(
            "**This is iteration {} of a convergence loop.**\n\n",
            self.current_iteration
        ));

        if !self.previous_attempt_summaries.is_empty() {
            context.push_str("### Previous Attempts\n");
            for summary in &self.previous_attempt_summaries {
                context.push_str(&format!("- {}\n", summary));
            }
            context.push('\n');
        }

        if self.drift_detected {
            context.push_str("**WARNING: Semantic drift detected.** The same gaps keep appearing across iterations.\n");
            context.push_str("Please carefully review whether you are truly addressing the root cause.\n\n");
        }

        if !self.recurring_gap_descriptions.is_empty() {
            context.push_str("### Recurring Gaps (NOT YET RESOLVED)\n");
            context.push_str("These issues have appeared multiple times and MUST be addressed:\n");
            for gap in &self.recurring_gap_descriptions {
                context.push_str(&format!("{}\n", gap));
            }
            context.push('\n');
        }

        if !self.focus_areas.is_empty() {
            context.push_str("### Required Focus Areas\n");
            context.push_str("Based on previous verification, focus on:\n");
            for area in &self.focus_areas {
                context.push_str(&format!("- {}\n", area));
            }
            context.push('\n');
        }

        context.push_str("---\n");
        context
    }
}

// ============================================================================
// Branch Verification
// ============================================================================

/// Request to verify a dependency branch before dependent tasks proceed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchVerificationRequest {
    /// Unique identifier for this verification request
    pub id: Uuid,
    /// The tasks that form this dependency branch (in dependency order)
    pub branch_tasks: Vec<Uuid>,
    /// The dependent tasks waiting on this branch
    pub waiting_tasks: Vec<Uuid>,
    /// The sub-objective this branch was supposed to accomplish
    pub branch_objective: String,
    /// Parent goal (if any)
    pub goal_id: Option<Uuid>,
    /// When this request was created
    pub created_at: DateTime<Utc>,
}

impl BranchVerificationRequest {
    pub fn new(branch_tasks: Vec<Uuid>, waiting_tasks: Vec<Uuid>, objective: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            branch_tasks,
            waiting_tasks,
            branch_objective: objective.into(),
            goal_id: None,
            created_at: Utc::now(),
        }
    }

    pub fn with_goal(mut self, goal_id: Uuid) -> Self {
        self.goal_id = Some(goal_id);
        self
    }
}

/// Result of verifying a dependency branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchVerificationResult {
    /// The request this is a result for
    pub request_id: Uuid,
    /// Whether the branch accomplished its objective
    pub branch_satisfied: bool,
    /// Confidence in this evaluation
    pub confidence: f64,
    /// Gaps in the branch's work
    pub gaps: Vec<IntentGap>,
    /// Whether dependent tasks should proceed
    pub dependents_can_proceed: bool,
    /// If dependents can't proceed, why not
    pub blocking_reason: Option<String>,
    /// Augmentations to apply to dependent tasks
    pub dependent_augmentations: Vec<DependentTaskAugmentation>,
    /// When this result was produced
    pub verified_at: DateTime<Utc>,
}

impl BranchVerificationResult {
    pub fn satisfied(request_id: Uuid) -> Self {
        Self {
            request_id,
            branch_satisfied: true,
            confidence: 1.0,
            gaps: Vec::new(),
            dependents_can_proceed: true,
            blocking_reason: None,
            dependent_augmentations: Vec::new(),
            verified_at: Utc::now(),
        }
    }

    pub fn unsatisfied(request_id: Uuid, reason: impl Into<String>) -> Self {
        Self {
            request_id,
            branch_satisfied: false,
            confidence: 0.0,
            gaps: Vec::new(),
            dependents_can_proceed: false,
            blocking_reason: Some(reason.into()),
            dependent_augmentations: Vec::new(),
            verified_at: Utc::now(),
        }
    }

    pub fn partial(request_id: Uuid, confidence: f64) -> Self {
        Self {
            request_id,
            branch_satisfied: false,
            confidence,
            gaps: Vec::new(),
            dependents_can_proceed: confidence >= 0.7, // Allow proceed if fairly confident
            blocking_reason: None,
            dependent_augmentations: Vec::new(),
            verified_at: Utc::now(),
        }
    }

    pub fn with_gap(mut self, gap: IntentGap) -> Self {
        self.gaps.push(gap);
        self
    }

    pub fn with_augmentation(mut self, aug: DependentTaskAugmentation) -> Self {
        self.dependent_augmentations.push(aug);
        self
    }

    pub fn blocking(mut self, reason: impl Into<String>) -> Self {
        self.dependents_can_proceed = false;
        self.blocking_reason = Some(reason.into());
        self
    }
}

/// Augmentation to apply to a dependent task based on branch verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependentTaskAugmentation {
    /// The dependent task to augment
    pub task_id: Uuid,
    /// Context about what the branch accomplished (and what it didn't)
    pub branch_context: String,
    /// Gaps from the branch that this task should be aware of
    pub inherited_gaps: Vec<String>,
    /// Workarounds the dependent task may need to apply
    pub suggested_workarounds: Vec<String>,
}

impl DependentTaskAugmentation {
    pub fn new(task_id: Uuid, context: impl Into<String>) -> Self {
        Self {
            task_id,
            branch_context: context.into(),
            inherited_gaps: Vec::new(),
            suggested_workarounds: Vec::new(),
        }
    }

    pub fn with_inherited_gap(mut self, gap: impl Into<String>) -> Self {
        self.inherited_gaps.push(gap.into());
        self
    }

    pub fn with_workaround(mut self, workaround: impl Into<String>) -> Self {
        self.suggested_workarounds.push(workaround.into());
        self
    }

    /// Format as context for the dependent task's prompt.
    pub fn format_for_prompt(&self) -> String {
        let mut context = String::from("\n\n## Upstream Branch Context\n\n");
        context.push_str(&format!("{}\n\n", self.branch_context));

        if !self.inherited_gaps.is_empty() {
            context.push_str("**Known Gaps from Dependencies:**\n");
            for gap in &self.inherited_gaps {
                context.push_str(&format!("- {}\n", gap));
            }
            context.push('\n');
        }

        if !self.suggested_workarounds.is_empty() {
            context.push_str("**Suggested Workarounds:**\n");
            for wa in &self.suggested_workarounds {
                context.push_str(&format!("- {}\n", wa));
            }
            context.push('\n');
        }

        context.push_str("---\n");
        context
    }
}

// ============================================================================
// Embedding-Based Similarity
// ============================================================================

/// Configuration for embedding-based gap similarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSimilarityConfig {
    /// Minimum cosine similarity to consider gaps as the same (0.0-1.0)
    pub similarity_threshold: f64,
    /// Whether to fall back to Jaccard if embeddings unavailable
    pub fallback_to_jaccard: bool,
    /// Model to use for embeddings (if using an embedding service)
    pub embedding_model: String,
    /// Embedding dimension (depends on model)
    pub embedding_dimension: usize,
}

impl Default for EmbeddingSimilarityConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.85,
            fallback_to_jaccard: true,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimension: 1536,
        }
    }
}

/// Enhanced gap fingerprint with embedding support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedGapFingerprint {
    /// Normalized description
    pub normalized_description: String,
    /// Embedding vector (if computed)
    pub embedding: Option<Vec<f32>>,
    /// Severity
    pub severity: GapSeverity,
    /// Category
    pub category: GapCategory,
    /// First seen iteration
    pub first_seen_iteration: u32,
    /// Occurrence count
    pub occurrence_count: u32,
    /// IDs of similar gaps that were merged into this fingerprint
    pub merged_gap_ids: Vec<Uuid>,
}

impl EmbeddedGapFingerprint {
    pub fn from_gap(gap: &IntentGap, iteration: u32) -> Self {
        Self {
            normalized_description: gap.description.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "),
            embedding: gap.embedding.clone(),
            severity: gap.severity,
            category: gap.category,
            first_seen_iteration: iteration,
            occurrence_count: 1,
            merged_gap_ids: Vec::new(),
        }
    }

    /// Check if another gap is similar to this fingerprint.
    pub fn is_similar_to(&self, other: &IntentGap, config: &EmbeddingSimilarityConfig) -> bool {
        // Try embedding similarity first
        if let (Some(ref self_emb), Some(ref other_emb)) = (&self.embedding, &other.embedding) {
            let similarity = cosine_similarity(self_emb, other_emb);
            return similarity >= config.similarity_threshold;
        }

        // Fall back to Jaccard if configured
        if config.fallback_to_jaccard {
            let other_normalized = other.description.to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            return jaccard_similarity(&self.normalized_description, &other_normalized) > 0.5;
        }

        false
    }

    /// Merge another similar gap into this fingerprint.
    pub fn merge(&mut self, _gap: &IntentGap) {
        self.occurrence_count += 1;
        // Could update embedding to be a weighted average, but for now just count
    }
}

/// Calculate cosine similarity between two embedding vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot_product: f64 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let norm_a: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot_product / (norm_a * norm_b)
}

/// Calculate Jaccard similarity between two normalized strings.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let words_a: std::collections::HashSet<_> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<_> = b.split_whitespace().collect();

    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }

    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();

    intersection as f64 / union as f64
}

/// Enhanced convergence state with embedding-based drift detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedConvergenceState {
    /// Base convergence state
    pub base: ConvergenceState,
    /// Embedded gap fingerprints for better similarity matching
    pub embedded_fingerprints: Vec<EmbeddedGapFingerprint>,
    /// Configuration for similarity matching
    pub similarity_config: EmbeddingSimilarityConfig,
    /// Clusters of related gaps (gaps that address similar issues)
    pub gap_clusters: Vec<GapCluster>,
}

impl EnhancedConvergenceState {
    pub fn new(intent: OriginalIntent) -> Self {
        Self {
            base: ConvergenceState::new(intent),
            embedded_fingerprints: Vec::new(),
            similarity_config: EmbeddingSimilarityConfig::default(),
            gap_clusters: Vec::new(),
        }
    }

    pub fn with_similarity_config(mut self, config: EmbeddingSimilarityConfig) -> Self {
        self.similarity_config = config;
        self
    }

    /// Record a verification result with embedding-based similarity.
    pub fn record_verification(&mut self, result: IntentVerificationResult) {
        let iteration = result.iteration;

        // Update embedded fingerprints
        for gap in result.all_gaps() {
            self.update_embedded_fingerprint(gap, iteration);
        }

        // Check for drift using embedded fingerprints
        self.check_drift();

        // Update base state
        self.base.record_verification(result);
    }

    fn update_embedded_fingerprint(&mut self, gap: &IntentGap, iteration: u32) {
        // Find similar existing fingerprint
        let similar_idx = self.embedded_fingerprints.iter()
            .position(|fp| fp.is_similar_to(gap, &self.similarity_config));

        if let Some(idx) = similar_idx {
            self.embedded_fingerprints[idx].merge(gap);
        } else {
            self.embedded_fingerprints.push(EmbeddedGapFingerprint::from_gap(gap, iteration));
        }
    }

    fn check_drift(&mut self) {
        // Drift if any fingerprint has 3+ occurrences
        self.base.drift_detected = self.embedded_fingerprints.iter()
            .any(|fp| fp.occurrence_count >= 3);
    }

    /// Get recurring gaps with their full context.
    pub fn recurring_gaps_detailed(&self) -> Vec<&EmbeddedGapFingerprint> {
        self.embedded_fingerprints.iter()
            .filter(|fp| fp.occurrence_count > 1)
            .collect()
    }

    /// Delegate to base state
    pub fn converged(&self) -> bool {
        self.base.converged
    }

    pub fn drift_detected(&self) -> bool {
        self.base.drift_detected
    }

    pub fn is_making_progress(&self) -> bool {
        self.base.is_making_progress()
    }

    pub fn end(&mut self) {
        self.base.end();
    }
}

/// A cluster of related gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapCluster {
    /// Unique identifier
    pub id: Uuid,
    /// Representative description for this cluster
    pub representative_description: String,
    /// Gap IDs in this cluster
    pub gap_ids: Vec<Uuid>,
    /// Centroid embedding (average of all gaps)
    pub centroid: Option<Vec<f32>>,
    /// Dominant category
    pub dominant_category: GapCategory,
    /// Maximum severity in cluster
    pub max_severity: GapSeverity,
}

impl GapCluster {
    pub fn new(representative: impl Into<String>, category: GapCategory, severity: GapSeverity) -> Self {
        Self {
            id: Uuid::new_v4(),
            representative_description: representative.into(),
            gap_ids: Vec::new(),
            centroid: None,
            dominant_category: category,
            max_severity: severity,
        }
    }

    pub fn add_gap(&mut self, gap_id: Uuid, severity: GapSeverity) {
        self.gap_ids.push(gap_id);
        if severity > self.max_severity {
            self.max_severity = severity;
        }
    }
}

// ============================================================================
// Swarm Events for Escalation
// ============================================================================

/// Event emitted when human escalation is needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanEscalationEvent {
    /// Unique event ID
    pub id: Uuid,
    /// Related goal (if any)
    pub goal_id: Option<Uuid>,
    /// Related task (if any)
    pub task_id: Option<Uuid>,
    /// The escalation details
    pub escalation: HumanEscalation,
    /// Current state of the convergence loop
    pub convergence_iteration: u32,
    /// When this event was created
    pub created_at: DateTime<Utc>,
}

impl HumanEscalationEvent {
    pub fn new(escalation: HumanEscalation) -> Self {
        Self {
            id: Uuid::new_v4(),
            goal_id: None,
            task_id: None,
            escalation,
            convergence_iteration: 0,
            created_at: Utc::now(),
        }
    }

    pub fn for_goal(mut self, goal_id: Uuid) -> Self {
        self.goal_id = Some(goal_id);
        self
    }

    pub fn for_task(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn at_iteration(mut self, iteration: u32) -> Self {
        self.convergence_iteration = iteration;
        self
    }

    /// Whether this escalation is blocking progress.
    pub fn is_blocking(&self) -> bool {
        self.escalation.urgency == EscalationUrgency::Blocking
    }
}

/// Human response to an escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanEscalationResponse {
    /// The event being responded to
    pub event_id: Uuid,
    /// Decision made by human
    pub decision: EscalationDecision,
    /// Free-form response text
    pub response_text: Option<String>,
    /// Additional context provided
    pub additional_context: Option<String>,
    /// When this response was received
    pub responded_at: DateTime<Utc>,
}

/// Decision made by human in response to escalation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationDecision {
    /// Accept the current state as good enough
    Accept,
    /// Reject and require more work
    Reject,
    /// Provide specific guidance
    Clarify {
        /// Clarification provided
        clarification: String,
    },
    /// Change the original intent
    ModifyIntent {
        /// New requirements to add
        new_requirements: Vec<String>,
        /// Requirements to remove
        removed_requirements: Vec<String>,
    },
    /// Abort the work entirely
    Abort,
    /// Defer decision (come back later)
    Defer {
        /// When to revisit
        revisit_after: Option<DateTime<Utc>>,
    },
}

impl EscalationDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::Clarify { .. } => "clarify",
            Self::ModifyIntent { .. } => "modify_intent",
            Self::Abort => "abort",
            Self::Defer { .. } => "defer",
        }
    }

    /// Whether this decision allows work to continue.
    pub fn allows_continuation(&self) -> bool {
        matches!(self, Self::Accept | Self::Clarify { .. } | Self::ModifyIntent { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_satisfaction_converged() {
        assert!(IntentSatisfaction::Satisfied.is_converged());
        assert!(!IntentSatisfaction::Partial.is_converged());
        assert!(!IntentSatisfaction::Unsatisfied.is_converged());
    }

    #[test]
    fn test_intent_satisfaction_should_retry() {
        assert!(!IntentSatisfaction::Satisfied.should_retry());
        assert!(IntentSatisfaction::Partial.should_retry());
        assert!(IntentSatisfaction::Unsatisfied.should_retry());
        assert!(!IntentSatisfaction::Indeterminate.should_retry());
    }

    #[test]
    fn test_gap_severity_ordering() {
        assert!(GapSeverity::Minor < GapSeverity::Moderate);
        assert!(GapSeverity::Moderate < GapSeverity::Major);
        assert!(GapSeverity::Major < GapSeverity::Critical);
    }

    #[test]
    fn test_original_intent_from_goal() {
        let goal_id = Uuid::new_v4();
        let intent = OriginalIntent::from_goal(goal_id, "Build a web server")
            .with_requirement("Handle HTTP requests")
            .with_success_criterion("Server responds to GET /");

        assert_eq!(intent.source_id, goal_id);
        assert_eq!(intent.source_type, IntentSource::Goal);
        assert_eq!(intent.key_requirements.len(), 1);
        assert_eq!(intent.success_criteria.len(), 1);
    }

    #[test]
    fn test_verification_result_should_iterate() {
        let intent_id = Uuid::new_v4();

        // Satisfied - no iteration
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Satisfied);
        assert!(!result.should_iterate());

        // Partial without guidance - no iteration
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial);
        assert!(!result.should_iterate());

        // Partial with guidance - should iterate
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_reprompt_guidance(RepromptGuidance::new(RepromptApproach::RetryWithContext));
        assert!(result.should_iterate());
    }

    #[test]
    fn test_convergence_config_should_continue() {
        let config = ConvergenceConfig::default();
        let intent_id = Uuid::new_v4();

        // Satisfied - don't continue
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Satisfied);
        assert!(!config.should_continue(&result));

        // Max iterations - don't continue
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_iteration(3);
        assert!(!config.should_continue(&result));

        // Partial with low confidence - continue
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_confidence(0.5)
            .with_reprompt_guidance(RepromptGuidance::new(RepromptApproach::RetryWithContext));
        assert!(config.should_continue(&result));
    }

    #[test]
    fn test_convergence_state_progress() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add first result with 3 gaps
        let result1 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_confidence(0.4)
            .with_gap(IntentGap::new("Gap 1", GapSeverity::Major))
            .with_gap(IntentGap::new("Gap 2", GapSeverity::Moderate))
            .with_gap(IntentGap::new("Gap 3", GapSeverity::Minor));
        state.record_verification(result1);

        // Add second result with 2 gaps (progress!)
        let result2 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(2)
            .with_confidence(0.6)
            .with_gap(IntentGap::new("Gap 1", GapSeverity::Moderate))
            .with_gap(IntentGap::new("Gap 2", GapSeverity::Minor));
        state.record_verification(result2);

        assert!(state.is_making_progress());
        assert_eq!(state.current_iteration, 2);
        assert!(!state.converged);
    }

    #[test]
    fn test_semantic_drift_detection() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add same gap across 3 iterations - should trigger drift
        for i in 1..=3 {
            let result = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
                .with_iteration(i)
                .with_confidence(0.5)
                .with_gap(IntentGap::new("Missing error handling", GapSeverity::Major));
            state.record_verification(result);
        }

        assert!(state.drift_detected);
        assert!(!state.is_making_progress());

        let recurring = state.recurring_gaps();
        assert_eq!(recurring.len(), 1);
        assert_eq!(recurring[0].occurrence_count, 3);
    }

    #[test]
    fn test_gap_similarity_detection() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add similar gaps with slight variations - should be detected as same
        let result1 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_gap(IntentGap::new("Missing error handling in API", GapSeverity::Major));
        state.record_verification(result1);

        let result2 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(2)
            .with_gap(IntentGap::new("error handling missing in API", GapSeverity::Major));
        state.record_verification(result2);

        let result3 = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(3)
            .with_gap(IntentGap::new("API missing error handling", GapSeverity::Major));
        state.record_verification(result3);

        // Should detect drift because the gaps are semantically similar
        assert!(state.drift_detected);
    }

    #[test]
    fn test_task_augmentation_formatting() {
        let task_id = Uuid::new_v4();
        let aug = TaskAugmentation::new(task_id)
            .with_gap("Missing validation")
            .with_gap("No error handling")
            .with_focus("Input validation")
            .with_focus("Error messages");

        let prefix = aug.format_as_description_prefix();
        assert!(prefix.contains("**Gaps to Address:**"));
        assert!(prefix.contains("Missing validation"));
        assert!(prefix.contains("No error handling"));
        assert!(prefix.contains("**Focus Areas:**"));
        assert!(prefix.contains("Input validation"));
        assert!(prefix.contains("**Original Task:**"));
    }

    #[test]
    fn test_task_augmentation_retry() {
        let task_id = Uuid::new_v4();
        let aug = TaskAugmentation::new(task_id)
            .as_retry("partial (confidence: 50%)")
            .with_gap("Still missing tests");

        let prefix = aug.format_as_description_prefix();
        assert!(prefix.contains("**RETRY ATTEMPT**"));
        assert!(prefix.contains("Previous attempt result: partial (confidence: 50%)"));
        assert!(aug.is_retry);
    }

    #[test]
    fn test_build_task_augmentations() {
        let task1_id = Uuid::new_v4();
        let task2_id = Uuid::new_v4();
        let intent_id = Uuid::new_v4();

        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_gap(IntentGap::new("Global gap", GapSeverity::Major))
            .with_reprompt_guidance(
                RepromptGuidance::new(RepromptApproach::RetryWithContext)
                    .with_focus("Error handling")
                    .with_retry(task1_id)
            );

        let pending_tasks = vec![task1_id, task2_id];
        let augmentations = build_task_augmentations(&result, &pending_tasks);

        // Should have augmentation for task1 (retry) and task2 (general gaps)
        assert_eq!(augmentations.len(), 2);

        let task1_aug = augmentations.iter().find(|a| a.task_id == task1_id).unwrap();
        assert!(task1_aug.is_retry);
        assert!(!task1_aug.focus_areas.is_empty());
    }

    #[test]
    fn test_iteration_context_formatting() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Add a verification result
        let result = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
            .with_iteration(1)
            .with_confidence(0.6)
            .with_gap(IntentGap::new("Missing tests", GapSeverity::Major))
            .with_reprompt_guidance(
                RepromptGuidance::new(RepromptApproach::RetryWithContext)
                    .with_focus("Unit tests")
            );
        state.record_verification(result);

        let context = state.build_iteration_context();
        assert_eq!(context.current_iteration, 2); // Next iteration
        assert_eq!(context.total_iterations_so_far, 1);
        assert!(!context.previous_attempt_summaries.is_empty());

        let formatted = context.format_for_prompt();
        assert!(formatted.contains("iteration 2"));
        assert!(formatted.contains("### Previous Attempts"));
    }

    #[test]
    fn test_iteration_context_with_drift() {
        let intent = OriginalIntent::from_goal(Uuid::new_v4(), "Test goal");
        let mut state = ConvergenceState::new(intent);

        // Force drift detection
        for i in 1..=3 {
            let result = IntentVerificationResult::new(Uuid::new_v4(), IntentSatisfaction::Partial)
                .with_iteration(i)
                .with_gap(IntentGap::new("Same recurring gap", GapSeverity::Major));
            state.record_verification(result);
        }

        let context = state.build_iteration_context();
        assert!(context.drift_detected);
        assert!(!context.recurring_gap_descriptions.is_empty());

        let formatted = context.format_for_prompt();
        assert!(formatted.contains("WARNING: Semantic drift detected"));
        assert!(formatted.contains("### Recurring Gaps"));
    }
}

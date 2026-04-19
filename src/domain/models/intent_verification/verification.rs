//! Verification protocol: gaps, satisfaction, constraints, and results.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

use super::escalation::{EscalationUrgency, HumanEscalation};
use super::guidance::RepromptGuidance;

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
    /// Goal constraint violation or non-conformance
    ConstraintViolation,
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
            Self::ConstraintViolation => "constraint_violation",
        }
    }

    /// Whether this category typically requires human judgment
    pub fn typically_needs_human(&self) -> bool {
        matches!(self, Self::Security | Self::Integration)
    }
}

impl FromStr for GapCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "functional" => Ok(Self::Functional),
            "error_handling" | "errorhandling" => Ok(Self::ErrorHandling),
            "integration" => Ok(Self::Integration),
            "testing" => Ok(Self::Testing),
            "security" => Ok(Self::Security),
            "performance" => Ok(Self::Performance),
            "observability" => Ok(Self::Observability),
            "documentation" => Ok(Self::Documentation),
            "maintainability" => Ok(Self::Maintainability),
            "constraint_violation" | "constraintviolation" => Ok(Self::ConstraintViolation),
            _ => Err(format!("unknown gap category: {s}")),
        }
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

/// Evaluation of a single constraint's conformance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintEvaluation {
    /// The constraint text (as provided in key requirements)
    pub constraint: String,
    /// Conformance status
    pub status: ConstraintConformance,
    /// Explanation of the evaluation
    pub explanation: String,
}

/// Conformance status for a constraint evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintConformance {
    /// Work conforms to the constraint
    Conforming,
    /// Work deviates but has justification
    Deviating,
    /// Work violates the constraint
    Violating,
}

impl ConstraintConformance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Conforming => "conforming",
            Self::Deviating => "deviating",
            Self::Violating => "violating",
        }
    }
}

impl FromStr for ConstraintConformance {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "conforming" => Ok(Self::Conforming),
            "deviating" => Ok(Self::Deviating),
            "violating" => Ok(Self::Violating),
            _ => Err(format!("unknown constraint conformance: {s}")),
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
    /// Per-constraint conformance evaluations
    pub constraint_evaluations: Vec<ConstraintEvaluation>,
    /// Human escalation information
    pub escalation: Option<HumanEscalation>,
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
            constraint_evaluations: Vec::new(),
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

    pub fn with_constraint_evaluation(mut self, eval: ConstraintEvaluation) -> Self {
        self.constraint_evaluations.push(eval);
        self
    }

    pub fn with_escalation(mut self, escalation: HumanEscalation) -> Self {
        self.escalation = Some(escalation);
        self
    }

    /// Check if we should attempt another iteration.
    pub fn should_iterate(&self) -> bool {
        // Don't iterate if human escalation is blocking
        if let Some(ref esc) = self.escalation
            && esc.needs_human && esc.urgency == EscalationUrgency::Blocking {
                return false;
            }
        self.satisfaction.should_retry() && self.reprompt_guidance.is_some()
    }

    /// Check if human judgment is required.
    pub fn needs_human(&self) -> bool {
        self.escalation.as_ref().is_some_and(|e| e.needs_human)
    }

    /// Check if progress is blocked pending human input.
    pub fn is_blocked_on_human(&self) -> bool {
        self.escalation.as_ref().is_some_and(|e| {
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
        // Invariant ([MUST]) constraint violations always escalate
        if self.constraint_evaluations.iter().any(|eval| {
            eval.status == ConstraintConformance::Violating
                && eval.constraint.starts_with("[MUST]")
        }) {
            let violated: Vec<_> = self.constraint_evaluations.iter()
                .filter(|eval| eval.status == ConstraintConformance::Violating && eval.constraint.starts_with("[MUST]"))
                .map(|eval| eval.constraint.clone())
                .collect();
            return Some(HumanEscalation::new(
                format!("Invariant constraint violation: {}", violated.join("; "))
            ).with_urgency(EscalationUrgency::High)
             .with_context(format!(
                 "The following invariant constraints were violated: {}",
                 violated.join("; ")
             ))
             .with_question("Should work continue despite these invariant violations?"));
        }

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
    pub dependent_augmentations: Vec<super::guidance::DependentTaskAugmentation>,
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

    pub fn with_augmentation(mut self, aug: super::guidance::DependentTaskAugmentation) -> Self {
        self.dependent_augmentations.push(aug);
        self
    }

    pub fn blocking(mut self, reason: impl Into<String>) -> Self {
        self.dependents_can_proceed = false;
        self.blocking_reason = Some(reason.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::guidance::{RepromptApproach, RepromptGuidance};

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
    fn test_constraint_conformance_from_str() {
        assert_eq!("conforming".parse::<ConstraintConformance>(), Ok(ConstraintConformance::Conforming));
        assert_eq!("deviating".parse::<ConstraintConformance>(), Ok(ConstraintConformance::Deviating));
        assert_eq!("violating".parse::<ConstraintConformance>(), Ok(ConstraintConformance::Violating));
        assert_eq!("Conforming".parse::<ConstraintConformance>(), Ok(ConstraintConformance::Conforming));
        assert!("unknown".parse::<ConstraintConformance>().is_err());
    }

    #[test]
    fn test_constraint_evaluation_on_result() {
        let intent_id = Uuid::new_v4();
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_constraint_evaluation(ConstraintEvaluation {
                constraint: "[MUST] No unsafe code".to_string(),
                status: ConstraintConformance::Conforming,
                explanation: "No unsafe blocks found".to_string(),
            })
            .with_constraint_evaluation(ConstraintEvaluation {
                constraint: "[SHOULD] Use logging".to_string(),
                status: ConstraintConformance::Deviating,
                explanation: "Logging not yet added".to_string(),
            });

        assert_eq!(result.constraint_evaluations.len(), 2);
        assert_eq!(result.constraint_evaluations[0].status, ConstraintConformance::Conforming);
        assert_eq!(result.constraint_evaluations[1].status, ConstraintConformance::Deviating);
    }

    #[test]
    fn test_should_escalate_on_must_violation() {
        let intent_id = Uuid::new_v4();
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_constraint_evaluation(ConstraintEvaluation {
                constraint: "[MUST] No unsafe code".to_string(),
                status: ConstraintConformance::Violating,
                explanation: "Found unsafe blocks".to_string(),
            });

        let escalation = result.should_escalate();
        assert!(escalation.is_some());
        let esc = escalation.unwrap();
        assert!(esc.reason.contains("Invariant constraint violation"));
        assert_eq!(esc.urgency, EscalationUrgency::High);
    }

    #[test]
    fn test_should_not_escalate_on_should_violation() {
        let intent_id = Uuid::new_v4();
        let result = IntentVerificationResult::new(intent_id, IntentSatisfaction::Partial)
            .with_constraint_evaluation(ConstraintEvaluation {
                constraint: "[SHOULD] Use logging".to_string(),
                status: ConstraintConformance::Violating,
                explanation: "No logging found".to_string(),
            });

        // SHOULD violations don't trigger escalation on their own
        let escalation = result.should_escalate();
        assert!(escalation.is_none());
    }

    #[test]
    fn test_gap_category_constraint_violation() {
        assert_eq!(GapCategory::ConstraintViolation.as_str(), "constraint_violation");
        assert_eq!("constraint_violation".parse::<GapCategory>(), Ok(GapCategory::ConstraintViolation));
        assert_eq!("constraintviolation".parse::<GapCategory>(), Ok(GapCategory::ConstraintViolation));
    }
}

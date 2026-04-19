//! Re-prompt guidance and task augmentation generation.

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

use super::escalation::EscalationUrgency;
use super::verification::{GapCategory, GapSeverity, IntentVerificationResult};

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

    /// Whether this approach requires human involvement
    pub fn needs_human(&self) -> bool {
        matches!(self, Self::Escalate { .. })
    }

    /// Whether this approach involves restructuring work
    pub fn is_restructure(&self) -> bool {
        matches!(self, Self::Restructure { .. })
    }
}

impl FromStr for RepromptApproach {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "retry_same" => Ok(Self::RetrySame),
            "retry_with_context" | "retry_context" => Ok(Self::RetryWithContext),
            "retry_augmented" => Ok(Self::RetryAugmented {
                augmentation_instructions: String::new(),
            }),
            "add_tasks" => Ok(Self::AddTasks),
            "retry_and_add_tasks" | "retry_and_add" => Ok(Self::RetryAndAddTasks),
            "restructure" => Ok(Self::Restructure {
                original_problem: String::new(),
                suggested_approach: String::new(),
            }),
            "escalate" => Ok(Self::Escalate {
                reason: String::new(),
            }),
            _ => Err(format!("unknown reprompt approach: {s}")),
        }
    }
}

/// Strategy selection based on gap analysis.
#[derive(Debug, Clone)]
pub struct RepromptStrategySelector;

impl RepromptStrategySelector {
    /// Select the best re-prompt strategy based on verification results.
    pub fn select_strategy(result: &IntentVerificationResult) -> RepromptApproach {
        // Check for escalation triggers first
        if let Some(ref escalation) = result.escalation
            && escalation.needs_human && escalation.urgency == EscalationUrgency::Blocking {
                return RepromptApproach::Escalate {
                    reason: escalation.reason.clone(),
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::verification::{IntentGap, IntentSatisfaction, IntentVerificationResult};

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
}

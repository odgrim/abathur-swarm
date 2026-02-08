//! Goal domain model.
//!
//! Goals represent high-level convergent objectives that guide the swarm.
//! Unlike tasks, goals are never "complete" - they continuously guide work.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a goal in the system.
///
/// Goals are convergent attractors - they guide work but are never "completed."
/// A goal can be:
/// - Active: Currently guiding work toward it
/// - Paused: Temporarily not guiding work (human-initiated only)
/// - Retired: No longer relevant, permanently stopped
///
/// Goal status is never changed by task outcomes. Tasks fail and succeed
/// independently; goals remain active as aspirations that guide evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Goal is actively guiding work
    Active,
    /// Goal is temporarily paused by a human (can be resumed)
    Paused,
    /// Goal has been retired (no longer guides work)
    Retired,
}

impl Default for GoalStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Retired => "retired",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }

    /// Check if this status can transition to another status.
    ///
    /// Goals can never be "completed" - only retired when no longer relevant.
    pub fn can_transition_to(&self, new_status: Self) -> bool {
        matches!(
            (self, new_status),
            // Active can be paused or retired
            (Self::Active, Self::Paused)
                | (Self::Active, Self::Retired)
                // Paused can resume or retire
                | (Self::Paused, Self::Active)
                | (Self::Paused, Self::Retired)
        )
    }

    /// Returns true if this is a terminal state.
    ///
    /// Only Retired is terminal - goals are never "completed."
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Retired)
    }

    /// Returns true if this goal is actively guiding work.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

}

/// Priority level for goals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

impl Default for GoalPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl GoalPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// Type of constraint on a goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintType {
    /// Must never be violated
    Invariant,
    /// Should be followed but can be relaxed
    Preference,
    /// Defines boundaries of acceptable solutions
    Boundary,
}

/// A constraint that applies to goal achievement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalConstraint {
    /// Name of the constraint
    pub name: String,
    /// Description of what the constraint requires
    pub description: String,
    /// Type of constraint
    pub constraint_type: ConstraintType,
}

impl GoalConstraint {
    pub fn new(name: impl Into<String>, description: impl Into<String>, constraint_type: ConstraintType) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            constraint_type,
        }
    }

    pub fn invariant(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, ConstraintType::Invariant)
    }

    pub fn preference(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, ConstraintType::Preference)
    }

    pub fn boundary(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, ConstraintType::Boundary)
    }
}

/// Metadata associated with a goal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalMetadata {
    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,
    /// Custom key-value pairs
    #[serde(default)]
    pub custom: std::collections::HashMap<String, serde_json::Value>,
}

/// A convergent goal that guides the swarm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// Unique identifier
    pub id: Uuid,
    /// Human-readable name
    pub name: String,
    /// Detailed description
    pub description: String,
    /// Current status
    pub status: GoalStatus,
    /// Priority level
    pub priority: GoalPriority,
    /// Parent goal (for sub-goals)
    pub parent_id: Option<Uuid>,
    /// Constraints that apply to this goal
    pub constraints: Vec<GoalConstraint>,
    /// Domains this goal is relevant to (e.g., "code-quality", "security", "testing")
    #[serde(default)]
    pub applicability_domains: Vec<String>,
    /// Additional metadata
    pub metadata: GoalMetadata,
    /// When this goal was created
    pub created_at: DateTime<Utc>,
    /// When this goal was last updated
    pub updated_at: DateTime<Utc>,
    /// Version for optimistic locking
    pub version: u64,
}

impl Goal {
    /// Create a new goal with the given name and description.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            status: GoalStatus::default(),
            priority: GoalPriority::default(),
            parent_id: None,
            constraints: Vec::new(),
            applicability_domains: Vec::new(),
            metadata: GoalMetadata::default(),
            created_at: now,
            updated_at: now,
            version: 1,
        }
    }

    /// Set the priority of this goal.
    pub fn with_priority(mut self, priority: GoalPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set the parent goal.
    pub fn with_parent(mut self, parent_id: Uuid) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    /// Add a constraint to this goal.
    pub fn with_constraint(mut self, constraint: GoalConstraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Add a tag to this goal.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.metadata.tags.push(tag.into());
        self
    }

    /// Add an applicability domain to this goal.
    pub fn with_applicability_domain(mut self, domain: impl Into<String>) -> Self {
        self.applicability_domains.push(domain.into());
        self
    }

    /// Check if this goal is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Check if this goal can transition to the given status.
    pub fn can_transition_to(&self, new_status: GoalStatus) -> bool {
        self.status.can_transition_to(new_status)
    }

    /// Transition to a new status, updating the timestamp.
    pub fn transition_to(&mut self, new_status: GoalStatus) -> Result<(), String> {
        if !self.can_transition_to(new_status) {
            return Err(format!(
                "Cannot transition from {} to {}",
                self.status.as_str(),
                new_status.as_str()
            ));
        }
        self.status = new_status;
        self.updated_at = Utc::now();
        self.version += 1;
        Ok(())
    }

    /// Validate this goal.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Goal name cannot be empty".to_string());
        }
        if self.name.len() > 255 {
            return Err("Goal name cannot exceed 255 characters".to_string());
        }
        Ok(())
    }

    /// Pause this goal (human-initiated only).
    pub fn pause(&mut self) {
        if self.can_transition_to(GoalStatus::Paused) {
            self.status = GoalStatus::Paused;
            self.updated_at = Utc::now();
            self.version += 1;
        }
    }

    /// Resume this goal (from paused state).
    pub fn resume(&mut self) {
        if self.can_transition_to(GoalStatus::Active) {
            self.status = GoalStatus::Active;
            self.updated_at = Utc::now();
            self.version += 1;
        }
    }

    /// Retire this goal.
    pub fn retire(&mut self) {
        if self.can_transition_to(GoalStatus::Retired) {
            self.status = GoalStatus::Retired;
            self.updated_at = Utc::now();
            self.version += 1;
        }
    }
}

/// Builder for creating goals with a fluent API.
#[derive(Debug, Default)]
pub struct GoalBuilder {
    name: Option<String>,
    description: Option<String>,
    priority: GoalPriority,
    parent_id: Option<Uuid>,
    constraints: Vec<GoalConstraint>,
    tags: Vec<String>,
    applicability_domains: Vec<String>,
}

impl GoalBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn priority(mut self, priority: GoalPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn parent(mut self, parent_id: Uuid) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    pub fn constraint(mut self, constraint: GoalConstraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn applicability_domain(mut self, domain: impl Into<String>) -> Self {
        self.applicability_domains.push(domain.into());
        self
    }

    pub fn build(self) -> Result<Goal, String> {
        let name = self.name.ok_or("Goal name is required")?;
        let description = self.description.unwrap_or_default();

        let mut goal = Goal::new(name, description)
            .with_priority(self.priority);

        if let Some(parent_id) = self.parent_id {
            goal = goal.with_parent(parent_id);
        }

        for constraint in self.constraints {
            goal = goal.with_constraint(constraint);
        }

        for tag in self.tags {
            goal = goal.with_tag(tag);
        }

        for domain in self.applicability_domains {
            goal = goal.with_applicability_domain(domain);
        }

        goal.validate()?;
        Ok(goal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_creation() {
        let goal = Goal::new("Test Goal", "A test description");
        assert_eq!(goal.name, "Test Goal");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.priority, GoalPriority::Normal);
    }

    #[test]
    fn test_goal_state_transitions() {
        let mut goal = Goal::new("Test", "Description");

        // Active goal can transition to Paused or Retired
        assert!(goal.can_transition_to(GoalStatus::Paused));
        assert!(goal.can_transition_to(GoalStatus::Retired));

        goal.transition_to(GoalStatus::Paused).unwrap();
        assert_eq!(goal.status, GoalStatus::Paused);

        // Paused can resume (Active) or retire
        assert!(goal.can_transition_to(GoalStatus::Active));
        assert!(goal.can_transition_to(GoalStatus::Retired));
    }

    #[test]
    fn test_goal_pause_and_resume() {
        let mut goal = Goal::new("Test", "Description");

        goal.pause();
        assert_eq!(goal.status, GoalStatus::Paused);

        goal.resume();
        assert_eq!(goal.status, GoalStatus::Active);
    }

    #[test]
    fn test_goals_never_complete() {
        // Goals are convergent attractors - they are never "completed"
        // Only Retired is a terminal state
        assert!(!GoalStatus::Active.is_terminal());
        assert!(!GoalStatus::Paused.is_terminal());
        assert!(GoalStatus::Retired.is_terminal());

        // Unknown statuses return None
        assert!(GoalStatus::from_str("completed").is_none());
        assert!(GoalStatus::from_str("suspended").is_none());
        assert!(GoalStatus::from_str("failed").is_none());
    }

    #[test]
    fn test_goal_builder() {
        let goal = GoalBuilder::new()
            .name("Built Goal")
            .description("Built description")
            .priority(GoalPriority::High)
            .tag("test")
            .constraint(GoalConstraint::invariant("Safety", "Must be safe"))
            .build()
            .unwrap();

        assert_eq!(goal.name, "Built Goal");
        assert_eq!(goal.priority, GoalPriority::High);
        assert_eq!(goal.metadata.tags.len(), 1);
        assert_eq!(goal.constraints.len(), 1);
    }

    #[test]
    fn test_goal_validation() {
        let goal = Goal::new("", "Empty name");
        assert!(goal.validate().is_err());

        let goal = Goal::new("Valid", "Description");
        assert!(goal.validate().is_ok());
    }
}

//! Human escalation types for the intent verification protocol.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

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
}

impl FromStr for EscalationUrgency {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "normal" => Ok(Self::Normal),
            "high" => Ok(Self::High),
            "blocking" => Ok(Self::Blocking),
            _ => Err(format!("unknown escalation urgency: {s}")),
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

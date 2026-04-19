//! Core identity, sequencing, and envelope types for the event bus.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::payload::EventPayload;

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

/// Budget pressure level used by the budget-aware scheduling system.
///
/// Defined here (rather than in `budget_tracker`) so it can be embedded directly
/// in `EventPayload` variants without creating an intra-crate circular dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetPressureLevel {
    /// Consumption is below the caution threshold — full scheduling allowed.
    Normal,
    /// Approaching the warning threshold — low-priority tasks may be deferred.
    Caution,
    /// Approaching the critical threshold — only high/critical tasks dispatch.
    Warning,
    /// Budget nearly exhausted — only critical tasks dispatch.
    Critical,
}

impl BudgetPressureLevel {
    /// Derive a pressure level from a consumed-percentage in `[0.0, 1.0]`.
    ///
    /// Uses canonical thresholds: Caution ≥ 60 %, Warning ≥ 80 %, Critical ≥ 95 %.
    /// `BudgetTracker` applies its own *configurable* thresholds internally; this
    /// method is provided for quick classification outside the tracker.
    pub fn from_pct(consumed_pct: f64) -> Self {
        if consumed_pct >= 0.95 {
            Self::Critical
        } else if consumed_pct >= 0.80 {
            Self::Warning
        } else if consumed_pct >= 0.60 {
            Self::Caution
        } else {
            Self::Normal
        }
    }

    /// Parse from a string, falling back to `Normal` for unrecognized input.
    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" => Self::Critical,
            "warning" => Self::Warning,
            "caution" => Self::Caution,
            _ => Self::Normal,
        }
    }

    /// Return a lowercase static string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Caution => "caution",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

impl std::fmt::Display for BudgetPressureLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
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
    Budget,
    Federation,
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
            Self::Budget => write!(f, "budget"),
            Self::Federation => write!(f, "federation"),
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

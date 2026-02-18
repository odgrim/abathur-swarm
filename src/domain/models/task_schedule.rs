//! Periodic task schedule domain model.
//!
//! A TaskSchedule defines a template for tasks that should be created
//! on a recurring schedule. It bridges the EventScheduler (time-keeping)
//! with the CommandBus (task creation).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::task::TaskPriority;

/// The type of schedule for a task schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskScheduleType {
    /// Fire once at a specific time.
    Once { at: DateTime<Utc> },
    /// Fire at a fixed interval.
    Interval { every_secs: u64 },
    /// Fire according to a cron expression (5-field: min hour dom month dow).
    Cron { expression: String },
}

impl TaskScheduleType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Once { .. } => "once",
            Self::Interval { .. } => "interval",
            Self::Cron { .. } => "cron",
        }
    }

    /// Human-readable description of the schedule.
    pub fn description(&self) -> String {
        match self {
            Self::Once { at } => format!("once at {}", at.format("%Y-%m-%d %H:%M UTC")),
            Self::Interval { every_secs } => {
                if *every_secs >= 3600 {
                    format!("every {} hour(s)", every_secs / 3600)
                } else if *every_secs >= 60 {
                    format!("every {} minute(s)", every_secs / 60)
                } else {
                    format!("every {} second(s)", every_secs)
                }
            }
            Self::Cron { expression } => format!("cron: {}", expression),
        }
    }
}

/// Status of a task schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskScheduleStatus {
    /// Schedule is active and will fire on its schedule.
    Active,
    /// Schedule is paused; will not fire until re-enabled.
    Paused,
    /// Schedule has been completed (one-shot that already fired).
    Completed,
}

impl TaskScheduleStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

/// Policy for handling overlapping scheduled task instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    /// Skip creating a new task if the previous one hasn't reached a terminal state.
    Skip,
    /// Always create a new task regardless of previous task status.
    Allow,
    /// Cancel the previous non-terminal task before creating a new one.
    CancelPrevious,
}

impl Default for OverlapPolicy {
    fn default() -> Self {
        Self::Skip
    }
}

impl OverlapPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Allow => "allow",
            Self::CancelPrevious => "cancel_previous",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "skip" => Some(Self::Skip),
            "allow" => Some(Self::Allow),
            "cancel_previous" | "cancel-previous" => Some(Self::CancelPrevious),
            _ => None,
        }
    }
}

/// A persistent task schedule definition.
///
/// This is the "template" that defines what task to create and when.
/// The actual time-keeping is delegated to the EventScheduler via a
/// companion ScheduledEvent with name `"task-schedule:{id}"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSchedule {
    pub id: Uuid,
    /// Human-readable name for the schedule (unique).
    pub name: String,
    /// Description of what this schedule does.
    pub description: String,

    // -- Schedule configuration --
    pub schedule: TaskScheduleType,

    // -- Task template --
    /// Title for created tasks.
    pub task_title: String,
    /// Description/prompt for created tasks.
    pub task_description: String,
    /// Priority for created tasks.
    pub task_priority: TaskPriority,
    /// Optional agent type to assign created tasks to.
    pub task_agent_type: Option<String>,

    // -- Behavior --
    /// How to handle overlapping task instances.
    pub overlap_policy: OverlapPolicy,
    /// Current status.
    pub status: TaskScheduleStatus,

    // -- Tracking --
    /// ID of the companion ScheduledEvent in the EventScheduler.
    pub scheduled_event_id: Option<Uuid>,
    /// Number of times this schedule has fired (tasks created).
    pub fire_count: u64,
    /// Last time a task was created by this schedule.
    pub last_fired_at: Option<DateTime<Utc>>,
    /// ID of the most recently created task.
    pub last_task_id: Option<Uuid>,

    // -- Timestamps --
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskSchedule {
    /// Create a new task schedule.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schedule: TaskScheduleType,
        task_title: impl Into<String>,
        task_description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            schedule,
            task_title: task_title.into(),
            task_description: task_description.into(),
            task_priority: TaskPriority::Normal,
            task_agent_type: None,
            overlap_policy: OverlapPolicy::Skip,
            status: TaskScheduleStatus::Active,
            scheduled_event_id: None,
            fire_count: 0,
            last_fired_at: None,
            last_task_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    // Builder methods
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.task_priority = priority;
        self
    }

    pub fn with_agent_type(mut self, agent_type: impl Into<String>) -> Self {
        self.task_agent_type = Some(agent_type.into());
        self
    }

    pub fn with_overlap_policy(mut self, policy: OverlapPolicy) -> Self {
        self.overlap_policy = policy;
        self
    }

    /// The EventScheduler event name for this schedule.
    pub fn event_name(&self) -> String {
        format!("task-schedule:{}", self.id)
    }

    /// Generate an idempotency key for the next task creation.
    /// Uses schedule ID + fire count to prevent duplicates.
    pub fn next_idempotency_key(&self) -> String {
        format!("sched:{}:{}", self.id, self.fire_count + 1)
    }
}

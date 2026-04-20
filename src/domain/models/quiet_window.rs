//! Quiet window domain model for cost-control scheduling.
//!
//! A quiet window defines a recurring time window (via cron expressions)
//! during which the swarm should not dispatch new work, allowing cost
//! control during high-pricing periods.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a quiet window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuietWindowStatus {
    /// Window is active and will suppress dispatch when inside the window.
    Enabled,
    /// Window is disabled and will not affect dispatch.
    Disabled,
}

impl QuietWindowStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// A recurring quiet window definition.
///
/// The window is "active" (i.e., the swarm is in quiet mode) when the current
/// time falls between the most recent `start_cron` fire and the most recent
/// `end_cron` fire. Concretely:
///
/// 1. Compute `last_start` = most recent fire time of `start_cron` before now.
/// 2. Compute `last_end` = most recent fire time of `end_cron` before now.
/// 3. If `last_start > last_end`, we are inside the quiet window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuietWindow {
    pub id: Uuid,
    /// Human-readable name (unique).
    pub name: String,
    /// Description of the window's purpose.
    pub description: String,
    /// Cron expression for window start (5-field: min hour dom month dow).
    pub start_cron: String,
    /// Cron expression for window end.
    pub end_cron: String,
    /// IANA timezone for cron evaluation (e.g., "America/New_York").
    pub timezone: String,
    /// Current status.
    pub status: QuietWindowStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl QuietWindow {
    /// Create a new quiet window.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        start_cron: impl Into<String>,
        end_cron: impl Into<String>,
        timezone: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            start_cron: start_cron.into(),
            end_cron: end_cron.into(),
            timezone: timezone.into(),
            status: QuietWindowStatus::Enabled,
            created_at: now,
            updated_at: now,
        }
    }
}

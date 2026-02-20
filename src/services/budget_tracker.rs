//! Budget-aware scheduling tracker.
//!
//! Tracks token-budget consumption across one or more windows (daily, weekly,
//! monthly, or custom), computes the aggregate [`BudgetPressureLevel`], and
//! emits [`EventPayload::BudgetPressureChanged`] / [`EventPayload::BudgetOpportunityDetected`]
//! events on the swarm's [`EventBus`].
//!
//! Other subsystems (scheduler, overmind) consume the public query methods
//! (`should_dispatch_task`, `effective_max_agents`, `should_pause_new_work`)
//! to adapt their behaviour to the current budget pressure without polling
//! an external API directly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::models::TaskPriority;
use super::config::BudgetConfig;
use super::event_bus::{BudgetPressureLevel, EventBus, EventCategory, EventPayload, EventSeverity};
use super::event_factory;

// ============================================================================
// Supporting types
// ============================================================================

/// Classification of a budget-reset window by period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetWindowType {
    Daily,
    Weekly,
    Monthly,
    /// Arbitrary custom window identified by a string label.
    Custom(String),
}

/// Snapshot of one billing / token-quota window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetWindow {
    /// Unique identifier for this window (e.g. `"anthropic-daily"`).
    pub id: String,
    /// The reset period for this window.
    pub window_type: BudgetWindowType,
    /// Fraction of quota consumed, in `[0.0, 1.0]`.
    pub consumed_pct: f64,
    /// Absolute remaining token count.
    pub remaining_tokens: u64,
    /// Seconds until the window resets.
    pub time_to_reset_secs: u64,
    /// When this window entry was last updated.
    pub last_updated: DateTime<Utc>,
}

/// An opportunity window: budget consumption is low enough that the scheduler
/// can be more aggressive about dispatching tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetOpportunity {
    /// The window that triggered the opportunity signal.
    pub window_id: String,
    /// Absolute remaining tokens in that window.
    pub remaining_tokens: u64,
    /// Seconds until the window resets.
    pub time_to_reset_secs: u64,
    /// Normalized opportunity score in `[0.0, 1.0]` — higher is better.
    pub opportunity_score: f64,
}

/// Point-in-time snapshot of the tracker's aggregate budget state.
#[derive(Debug, Clone)]
pub struct BudgetState {
    /// Current aggregate pressure level (worst-case across all windows).
    pub pressure_level: BudgetPressureLevel,
    /// All known windows and their latest data.
    pub windows: Vec<BudgetWindow>,
    /// Cumulative tokens recorded via [`BudgetTracker::record_tokens_used`].
    pub total_tokens_recorded: u64,
    /// Most recently detected opportunity, if any.
    pub last_opportunity: Option<BudgetOpportunity>,
}

// ============================================================================
// Configuration
// ============================================================================

/// Runtime configuration for [`BudgetTracker`].
///
/// Thresholds are fractions in `[0.0, 1.0]` of total quota consumed.
#[derive(Debug, Clone)]
pub struct BudgetTrackerConfig {
    /// Consumed-% at or above which pressure becomes `Caution`.
    pub caution_threshold_pct: f64,
    /// Consumed-% at or above which pressure becomes `Warning`.
    pub warning_threshold_pct: f64,
    /// Consumed-% at or above which pressure becomes `Critical`.
    pub critical_threshold_pct: f64,
    /// Consumed-% *below* which an opportunity is signalled.
    pub opportunity_threshold_pct: f64,
    /// Minimum remaining tokens for an opportunity to be announced.
    pub min_opportunity_tokens: u64,
    /// Maximum concurrent agents allowed under `Normal` pressure.
    pub max_agents_normal: u32,
    /// Maximum concurrent agents allowed under `Caution` pressure.
    pub max_agents_caution: u32,
    /// Maximum concurrent agents allowed under `Warning` pressure.
    pub max_agents_warning: u32,
    /// Maximum concurrent agents allowed under `Critical` pressure.
    pub max_agents_critical: u32,
}

impl Default for BudgetTrackerConfig {
    fn default() -> Self {
        Self {
            caution_threshold_pct: 0.60,
            warning_threshold_pct: 0.80,
            critical_threshold_pct: 0.95,
            opportunity_threshold_pct: 0.30,
            min_opportunity_tokens: 10_000,
            max_agents_normal: 5,
            max_agents_caution: 4,
            max_agents_warning: 2,
            max_agents_critical: 1,
        }
    }
}

impl BudgetTrackerConfig {
    /// Construct from a [`BudgetConfig`] loaded from `abathur.toml`.
    pub fn from_budget_config(cfg: &BudgetConfig) -> Self {
        Self {
            caution_threshold_pct: cfg.caution_threshold_pct,
            warning_threshold_pct: cfg.warning_threshold_pct,
            critical_threshold_pct: cfg.critical_threshold_pct,
            opportunity_threshold_pct: cfg.opportunity_threshold_pct,
            min_opportunity_tokens: cfg.min_opportunity_tokens,
            max_agents_normal: cfg.max_agents_normal,
            max_agents_caution: cfg.max_agents_caution,
            max_agents_warning: cfg.max_agents_warning,
            max_agents_critical: cfg.max_agents_critical,
        }
    }
}

// ============================================================================
// Internal mutable state (held behind RwLock)
// ============================================================================

struct Inner {
    windows: Vec<BudgetWindow>,
    pressure_level: BudgetPressureLevel,
    total_tokens_recorded: u64,
    last_opportunity: Option<BudgetOpportunity>,
}

// ============================================================================
// BudgetTracker
// ============================================================================

/// Central service for budget-aware scheduling decisions.
///
/// # Usage
///
/// 1. Call [`report_budget_signal`](Self::report_budget_signal) whenever the
///    billing adapter receives updated quota metrics.
/// 2. Query [`should_dispatch_task`](Self::should_dispatch_task) and
///    [`effective_max_agents`](Self::effective_max_agents) before dispatching
///    work.
/// 3. Subscribe to [`EventPayload::BudgetPressureChanged`] events on the bus
///    to react reactively in other subsystems.
pub struct BudgetTracker {
    config: BudgetTrackerConfig,
    event_bus: Arc<EventBus>,
    inner: Arc<RwLock<Inner>>,
}

impl BudgetTracker {
    /// Create a new tracker with the given configuration and event bus.
    pub fn new(config: BudgetTrackerConfig, event_bus: Arc<EventBus>) -> Self {
        Self {
            config,
            event_bus,
            inner: Arc::new(RwLock::new(Inner {
                windows: Vec::new(),
                pressure_level: BudgetPressureLevel::Normal,
                total_tokens_recorded: 0,
                last_opportunity: None,
            })),
        }
    }

    // -------------------------------------------------------------------------
    // Ingestion
    // -------------------------------------------------------------------------

    /// Record an updated budget signal for `window_id`.
    ///
    /// - Updates (or creates) the window entry.
    /// - Recomputes the aggregate pressure level.
    /// - Emits [`EventPayload::BudgetPressureChanged`] if the level changed.
    /// - Emits [`EventPayload::BudgetOpportunityDetected`] if applicable.
    pub async fn report_budget_signal(
        &self,
        window_id: impl Into<String>,
        window_type: BudgetWindowType,
        consumed_pct: f64,
        remaining_tokens: u64,
        time_to_reset_secs: u64,
    ) {
        let window_id = window_id.into();
        let new_level = self.level_from_pct(consumed_pct);

        let previous_level = {
            let mut inner = self.inner.write().await;

            // Update or insert the window.
            if let Some(w) = inner.windows.iter_mut().find(|w| w.id == window_id) {
                w.consumed_pct = consumed_pct;
                w.remaining_tokens = remaining_tokens;
                w.time_to_reset_secs = time_to_reset_secs;
                w.last_updated = Utc::now();
            } else {
                inner.windows.push(BudgetWindow {
                    id: window_id.clone(),
                    window_type,
                    consumed_pct,
                    remaining_tokens,
                    time_to_reset_secs,
                    last_updated: Utc::now(),
                });
            }

            let previous = inner.pressure_level;
            inner.pressure_level = new_level;
            previous
        };

        // Emit pressure change if the level actually changed.
        if previous_level != new_level {
            let severity = if new_level > previous_level {
                EventSeverity::Warning
            } else {
                EventSeverity::Info
            };
            let event = event_factory::make_event(
                severity,
                EventCategory::Budget,
                None,
                None,
                EventPayload::BudgetPressureChanged {
                    previous_level,
                    new_level,
                    consumed_pct,
                    window_id: window_id.clone(),
                },
            );
            self.event_bus.publish(event).await;
        }

        // Check for an opportunity window and emit if warranted.
        self.detect_opportunity_and_emit(&window_id, consumed_pct, remaining_tokens, time_to_reset_secs)
            .await;
    }

    /// Increment the internal cumulative token counter.
    ///
    /// Call this on every task completion to maintain an accurate tally of
    /// tokens consumed by the swarm (independent of external quota signals).
    pub async fn record_tokens_used(&self, _task_id: Uuid, tokens: u64) {
        let mut inner = self.inner.write().await;
        inner.total_tokens_recorded += tokens;
    }

    // -------------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------------

    /// Return a point-in-time snapshot of the current budget state.
    pub async fn current_state(&self) -> BudgetState {
        let inner = self.inner.read().await;
        BudgetState {
            pressure_level: inner.pressure_level,
            windows: inner.windows.clone(),
            total_tokens_recorded: inner.total_tokens_recorded,
            last_opportunity: inner.last_opportunity.clone(),
        }
    }

    /// Return the effective maximum concurrent agents given a caller-supplied
    /// base maximum, capped by the pressure-level constraint.
    ///
    /// ```text
    /// effective = min(base_max, config.max_agents_<level>)
    /// ```
    pub async fn effective_max_agents(&self, base_max: u32) -> u32 {
        let pressure = {
            let inner = self.inner.read().await;
            inner.pressure_level
        };
        let configured_max = match pressure {
            BudgetPressureLevel::Normal => self.config.max_agents_normal,
            BudgetPressureLevel::Caution => self.config.max_agents_caution,
            BudgetPressureLevel::Warning => self.config.max_agents_warning,
            BudgetPressureLevel::Critical => self.config.max_agents_critical,
        };
        base_max.min(configured_max)
    }

    /// Return `true` if a task with `priority` should be dispatched immediately.
    ///
    /// | Pressure | Dispatches            |
    /// |----------|-----------------------|
    /// | Normal   | All priorities        |
    /// | Caution  | Normal, High, Critical|
    /// | Warning  | High, Critical        |
    /// | Critical | Critical only         |
    pub async fn should_dispatch_task(&self, priority: TaskPriority) -> bool {
        let pressure = {
            let inner = self.inner.read().await;
            inner.pressure_level
        };
        match pressure {
            BudgetPressureLevel::Normal => true,
            BudgetPressureLevel::Caution => priority >= TaskPriority::Normal,
            BudgetPressureLevel::Warning => priority >= TaskPriority::High,
            BudgetPressureLevel::Critical => priority == TaskPriority::Critical,
        }
    }

    /// Return `true` if all new non-critical work should be paused.
    ///
    /// Currently triggers only at `Critical` pressure.
    pub async fn should_pause_new_work(&self) -> bool {
        let inner = self.inner.read().await;
        inner.pressure_level == BudgetPressureLevel::Critical
    }

    // -------------------------------------------------------------------------
    // Maintenance
    // -------------------------------------------------------------------------

    /// Recompute the aggregate pressure level as the worst case across all
    /// known windows, and emit a `BudgetPressureChanged` event if it changed.
    ///
    /// Call this periodically (e.g. from the event scheduler) to reconcile
    /// the cached level with observed window data.
    pub async fn recompute_state(&self) {
        // Snapshot outside the write-lock to minimise contention.
        let (windows_snapshot, old_level) = {
            let inner = self.inner.read().await;
            (inner.windows.clone(), inner.pressure_level)
        };

        let worst_level = windows_snapshot
            .iter()
            .map(|w| self.level_from_pct(w.consumed_pct))
            .max()
            .unwrap_or(BudgetPressureLevel::Normal);

        let changed = {
            let mut inner = self.inner.write().await;
            inner.pressure_level = worst_level;
            old_level != worst_level
        };

        if changed {
            let max_consumed = windows_snapshot
                .iter()
                .map(|w| w.consumed_pct)
                .fold(f64::NEG_INFINITY, f64::max);

            let event = event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Budget,
                None,
                None,
                EventPayload::BudgetPressureChanged {
                    previous_level: old_level,
                    new_level: worst_level,
                    consumed_pct: max_consumed,
                    window_id: "aggregate".to_string(),
                },
            );
            self.event_bus.publish(event).await;
        }
    }

    /// Check all known windows for an opportunity signal and return the best
    /// one if found (highest `opportunity_score`).
    ///
    /// Does not emit an event — call [`report_budget_signal`](Self::report_budget_signal)
    /// to get automatic opportunity emission on incoming signals.
    pub async fn detect_opportunity(&self) -> Option<BudgetOpportunity> {
        let inner = self.inner.read().await;
        inner
            .windows
            .iter()
            .filter(|w| {
                w.consumed_pct < self.config.opportunity_threshold_pct
                    && w.remaining_tokens >= self.config.min_opportunity_tokens
            })
            .map(|w| {
                let score = (self.config.opportunity_threshold_pct - w.consumed_pct)
                    / self.config.opportunity_threshold_pct;
                BudgetOpportunity {
                    window_id: w.id.clone(),
                    remaining_tokens: w.remaining_tokens,
                    time_to_reset_secs: w.time_to_reset_secs,
                    opportunity_score: score,
                }
            })
            .max_by(|a, b| a.opportunity_score.partial_cmp(&b.opportunity_score).unwrap_or(std::cmp::Ordering::Equal))
    }

    // -------------------------------------------------------------------------
    // Private helpers
    // -------------------------------------------------------------------------

    /// Map a consumed-percentage to a pressure level using this tracker's config.
    fn level_from_pct(&self, consumed_pct: f64) -> BudgetPressureLevel {
        if consumed_pct >= self.config.critical_threshold_pct {
            BudgetPressureLevel::Critical
        } else if consumed_pct >= self.config.warning_threshold_pct {
            BudgetPressureLevel::Warning
        } else if consumed_pct >= self.config.caution_threshold_pct {
            BudgetPressureLevel::Caution
        } else {
            BudgetPressureLevel::Normal
        }
    }

    /// Internally check and emit an opportunity event for an incoming signal.
    async fn detect_opportunity_and_emit(
        &self,
        window_id: &str,
        consumed_pct: f64,
        remaining_tokens: u64,
        time_to_reset_secs: u64,
    ) {
        if consumed_pct < self.config.opportunity_threshold_pct
            && remaining_tokens >= self.config.min_opportunity_tokens
        {
            let score = (self.config.opportunity_threshold_pct - consumed_pct)
                / self.config.opportunity_threshold_pct;

            let opp = BudgetOpportunity {
                window_id: window_id.to_string(),
                remaining_tokens,
                time_to_reset_secs,
                opportunity_score: score,
            };

            // Persist the opportunity in shared state.
            {
                let mut inner = self.inner.write().await;
                inner.last_opportunity = Some(opp);
            }

            // Emit the event (lock is released).
            let event = event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Budget,
                None,
                None,
                EventPayload::BudgetOpportunityDetected {
                    window_id: window_id.to_string(),
                    remaining_tokens,
                    time_to_reset_secs,
                    opportunity_score: score,
                },
            );
            self.event_bus.publish(event).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::EventBusConfig;

    fn make_tracker() -> BudgetTracker {
        let bus = Arc::new(EventBus::new(EventBusConfig::default()));
        BudgetTracker::new(BudgetTrackerConfig::default(), bus)
    }

    #[tokio::test]
    async fn test_initial_state_is_normal() {
        let tracker = make_tracker();
        let state = tracker.current_state().await;
        assert_eq!(state.pressure_level, BudgetPressureLevel::Normal);
        assert!(state.windows.is_empty());
    }

    #[tokio::test]
    async fn test_pressure_level_escalates() {
        let tracker = make_tracker();
        tracker
            .report_budget_signal("daily", BudgetWindowType::Daily, 0.85, 15_000, 3600)
            .await;
        let state = tracker.current_state().await;
        assert_eq!(state.pressure_level, BudgetPressureLevel::Warning);
    }

    #[tokio::test]
    async fn test_should_dispatch_task_normal() {
        let tracker = make_tracker();
        assert!(tracker.should_dispatch_task(TaskPriority::Low).await);
    }

    #[tokio::test]
    async fn test_should_dispatch_task_critical_pressure() {
        let tracker = make_tracker();
        tracker
            .report_budget_signal("daily", BudgetWindowType::Daily, 0.97, 300, 3600)
            .await;
        assert!(!tracker.should_dispatch_task(TaskPriority::Low).await);
        assert!(!tracker.should_dispatch_task(TaskPriority::Normal).await);
        assert!(!tracker.should_dispatch_task(TaskPriority::High).await);
        assert!(tracker.should_dispatch_task(TaskPriority::Critical).await);
    }

    #[tokio::test]
    async fn test_opportunity_detected_when_low() {
        let tracker = make_tracker();
        tracker
            .report_budget_signal("daily", BudgetWindowType::Daily, 0.10, 90_000, 86400)
            .await;
        let opp = tracker.detect_opportunity().await;
        assert!(opp.is_some());
        let opp = opp.unwrap();
        assert!(opp.opportunity_score > 0.0);
    }

    #[tokio::test]
    async fn test_effective_max_agents_capped_under_pressure() {
        let tracker = make_tracker();
        // Normal: base_max=10 → min(10, 5) = 5
        assert_eq!(tracker.effective_max_agents(10).await, 5);

        tracker
            .report_budget_signal("daily", BudgetWindowType::Daily, 0.96, 400, 3600)
            .await;
        // Critical: base_max=10 → min(10, 1) = 1
        assert_eq!(tracker.effective_max_agents(10).await, 1);
    }

    #[tokio::test]
    async fn test_record_tokens_accumulates() {
        let tracker = make_tracker();
        let id = Uuid::new_v4();
        tracker.record_tokens_used(id, 1000).await;
        tracker.record_tokens_used(id, 500).await;
        let state = tracker.current_state().await;
        assert_eq!(state.total_tokens_recorded, 1500);
    }
}

//! Convergence system events for observability.
//!
//! All convergence events flow through the EventBus (spec Part 10.1). Events are
//! emitted inline throughout the convergence engine at the points described in
//! Part 6 of the convergence specification.
//!
//! Events are grouped into three families:
//!
//! - **Lifecycle events** -- emitted at trajectory creation and finalization.
//! - **Per-iteration events** -- emitted during each iteration of the
//!   convergence loop.
//! - **Intervention events** -- emitted when the engine detects a condition
//!   that requires special handling (context degradation, budget extension,
//!   specification ambiguity, decomposition).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::*;

// ---------------------------------------------------------------------------
// ConvergenceEvent
// ---------------------------------------------------------------------------

/// A convergence system event emitted for observability and downstream
/// processing.
///
/// Each variant corresponds to a specific moment in the convergence lifecycle.
/// Events carry enough context for downstream consumers (dashboards, loggers,
/// reactors) to understand what happened without needing to look up the full
/// trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvergenceEvent {
    // -------------------------------------------------------------------
    // Lifecycle events
    // -------------------------------------------------------------------

    /// A new convergence trajectory has been created and is entering the
    /// iteration loop.
    ///
    /// Emitted once per trajectory at the start of `converge()`.
    TrajectoryStarted {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// The task this trajectory is converging on.
        task_id: String,
        /// The parent goal, if any.
        goal_id: Option<String>,
        /// The allocated convergence budget.
        budget: ConvergenceBudget,
        /// When the trajectory was started.
        timestamp: DateTime<Utc>,
    },

    /// The trajectory has reached a fixed-point attractor and the result
    /// meets the acceptance threshold.
    ///
    /// Emitted once at finalization when convergence succeeds.
    TrajectoryConverged {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Total number of observations (iterations) in the trajectory.
        total_observations: u32,
        /// Total tokens consumed across all iterations.
        total_tokens_used: u64,
        /// Total number of fresh starts that occurred.
        total_fresh_starts: u32,
        /// When the trajectory converged.
        timestamp: DateTime<Utc>,
    },

    /// The trajectory's budget has been exhausted without reaching the
    /// acceptance threshold.
    ///
    /// Emitted once at finalization when the budget runs out.
    TrajectoryExhausted {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Sequence number of the best observation achieved.
        best_observation_sequence: u32,
        /// Fraction of the budget consumed (should be ~1.0).
        budget_consumed_fraction: f64,
        /// Human-readable reason for exhaustion.
        reason: String,
        /// When the trajectory was finalized.
        timestamp: DateTime<Utc>,
    },

    /// The trajectory is trapped in a limit cycle with no remaining
    /// escape strategies.
    ///
    /// Emitted once at finalization when all escape strategies have been
    /// exhausted for a detected limit cycle.
    TrajectoryTrapped {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// The attractor type the trajectory is trapped in.
        attractor_type: AttractorType,
        /// The detected cycle period (for limit cycles).
        cycle_period: Option<u32>,
        /// Number of escape attempts that were tried.
        escape_attempts: u32,
        /// When the trajectory was finalized.
        timestamp: DateTime<Utc>,
    },

    // -------------------------------------------------------------------
    // Per-iteration events
    // -------------------------------------------------------------------

    /// An observation has been recorded after a strategy execution.
    ///
    /// Emitted once per iteration after overseer signals are collected and
    /// convergence metrics are computed.
    ObservationRecorded {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Sequence number of this observation within the trajectory.
        observation_sequence: u32,
        /// Change in convergence level from the previous observation.
        convergence_delta: f64,
        /// Absolute convergence level after this observation (0.0 -- 1.0).
        convergence_level: f64,
        /// The strategy that produced this observation.
        strategy_used: StrategyKind,
        /// Fraction of the budget remaining after this observation.
        budget_remaining_fraction: f64,
    },

    /// The attractor classifier has (re-)classified the trajectory.
    ///
    /// Emitted after every observation once enough data is available for
    /// classification.
    AttractorClassified {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// The classified attractor type.
        attractor_type: AttractorType,
        /// Confidence in this classification (0.0 -- 1.0).
        confidence: f64,
    },

    /// A strategy has been selected for the next iteration.
    ///
    /// Emitted after attractor classification and before strategy execution.
    StrategySelected {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// The strategy selected for the next iteration.
        strategy: StrategyKind,
        /// The current attractor classification that informed the selection.
        attractor_type: AttractorType,
        /// Human-readable reason for this selection.
        reason: String,
        /// Fraction of the budget remaining at selection time.
        budget_remaining_fraction: f64,
    },

    // -------------------------------------------------------------------
    // Intervention events
    // -------------------------------------------------------------------

    /// Context degradation has been detected in the LLM's working context.
    ///
    /// Emitted when the context health score drops below the degradation
    /// threshold, triggering a forced fresh start (spec 6.4).
    ContextDegradationDetected {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// The signal-to-noise ratio or health score that triggered detection.
        health_score: f64,
        /// Which fresh start number this will trigger.
        fresh_start_number: u32,
    },

    /// A budget extension has been requested because the trajectory is
    /// approaching a fixed point but running low on resources.
    ///
    /// Emitted when the engine determines that additional budget would
    /// allow convergence to complete.
    BudgetExtensionRequested {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Fraction of the original budget consumed so far.
        current_usage_fraction: f64,
        /// Additional tokens being requested.
        requested_extension_tokens: u64,
        /// Evidence supporting the extension request (e.g. convergence trend).
        convergence_evidence: String,
    },

    /// A budget extension request has been granted.
    BudgetExtensionGranted {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Additional tokens that were granted.
        additional_tokens: u64,
    },

    /// A budget extension request has been denied.
    BudgetExtensionDenied {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Reason the extension was denied.
        reason: String,
    },

    /// The specification has been amended (spec 1.6).
    ///
    /// Emitted whenever the specification evolves, whether due to user hints,
    /// overseer discoveries, architect review, or test disambiguation.
    SpecificationAmended {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Where this amendment originated.
        amendment_source: AmendmentSource,
        /// Human-readable summary of the amendment.
        amendment_summary: String,
    },

    /// Specification ambiguity has been detected during preparation.
    ///
    /// Emitted when generated tests or verification reveal contradictions
    /// in the specification (spec 10.1).
    SpecificationAmbiguityDetected {
        /// The task whose specification contains ambiguity.
        task_id: String,
        /// Descriptions of the contradictions found.
        contradictions: Vec<String>,
        /// Suggested clarifications to resolve the ambiguity.
        suggested_clarifications: Vec<String>,
    },

    /// Decomposition has been recommended during the DECIDE phase.
    ///
    /// Emitted when proactive decomposition analysis determines that
    /// breaking the task into subtasks would improve convergence (spec 10.1).
    DecompositionRecommended {
        /// The task for which decomposition is recommended.
        task_id: String,
        /// Number of proposed subtasks.
        subtask_count: usize,
        /// Estimated savings from decomposition (0.0 -- 1.0 fraction of
        /// budget saved compared to monolithic convergence).
        savings_estimate: f64,
    },

    /// Decomposition has been triggered and child trajectories are being
    /// created.
    ///
    /// Emitted when the Decompose strategy is actually executed and the
    /// parent trajectory transitions to the Coordinating phase.
    DecompositionTriggered {
        /// The parent trajectory that is being decomposed.
        parent_trajectory_id: String,
        /// Number of child trajectories created.
        child_count: usize,
    },

    /// Parallel convergence sampling has started for a trajectory.
    ///
    /// Emitted when the convergence mode is `Parallel` and multiple
    /// independent trajectory samples are being spawned (spec 6.6).
    ParallelConvergenceStarted {
        /// Unique identifier for the trajectory.
        trajectory_id: String,
        /// Number of parallel trajectory samples being spawned.
        parallel_count: usize,
    },
}

impl ConvergenceEvent {
    /// Returns a human-readable name for this event variant, suitable for
    /// logging and event bus categorization.
    pub fn event_name(&self) -> &'static str {
        match self {
            ConvergenceEvent::TrajectoryStarted { .. } => "trajectory_started",
            ConvergenceEvent::TrajectoryConverged { .. } => "trajectory_converged",
            ConvergenceEvent::TrajectoryExhausted { .. } => "trajectory_exhausted",
            ConvergenceEvent::TrajectoryTrapped { .. } => "trajectory_trapped",
            ConvergenceEvent::ObservationRecorded { .. } => "observation_recorded",
            ConvergenceEvent::AttractorClassified { .. } => "attractor_classified",
            ConvergenceEvent::StrategySelected { .. } => "strategy_selected",
            ConvergenceEvent::ContextDegradationDetected { .. } => {
                "context_degradation_detected"
            }
            ConvergenceEvent::BudgetExtensionRequested { .. } => {
                "budget_extension_requested"
            }
            ConvergenceEvent::BudgetExtensionGranted { .. } => "budget_extension_granted",
            ConvergenceEvent::BudgetExtensionDenied { .. } => "budget_extension_denied",
            ConvergenceEvent::SpecificationAmended { .. } => "specification_amended",
            ConvergenceEvent::SpecificationAmbiguityDetected { .. } => {
                "specification_ambiguity_detected"
            }
            ConvergenceEvent::DecompositionRecommended { .. } => {
                "decomposition_recommended"
            }
            ConvergenceEvent::DecompositionTriggered { .. } => "decomposition_triggered",
            ConvergenceEvent::ParallelConvergenceStarted { .. } => {
                "parallel_convergence_started"
            }
        }
    }

    /// Returns the trajectory ID associated with this event, if applicable.
    ///
    /// Most events are scoped to a trajectory. The exceptions are
    /// `SpecificationAmbiguityDetected` and `DecompositionRecommended`,
    /// which are scoped to a task.
    pub fn trajectory_id(&self) -> Option<&str> {
        match self {
            ConvergenceEvent::TrajectoryStarted { trajectory_id, .. }
            | ConvergenceEvent::TrajectoryConverged { trajectory_id, .. }
            | ConvergenceEvent::TrajectoryExhausted { trajectory_id, .. }
            | ConvergenceEvent::TrajectoryTrapped { trajectory_id, .. }
            | ConvergenceEvent::ObservationRecorded { trajectory_id, .. }
            | ConvergenceEvent::AttractorClassified { trajectory_id, .. }
            | ConvergenceEvent::StrategySelected { trajectory_id, .. }
            | ConvergenceEvent::ContextDegradationDetected { trajectory_id, .. }
            | ConvergenceEvent::BudgetExtensionRequested { trajectory_id, .. }
            | ConvergenceEvent::BudgetExtensionGranted { trajectory_id, .. }
            | ConvergenceEvent::BudgetExtensionDenied { trajectory_id, .. }
            | ConvergenceEvent::SpecificationAmended { trajectory_id, .. }
            | ConvergenceEvent::ParallelConvergenceStarted { trajectory_id, .. } => {
                Some(trajectory_id)
            }
            ConvergenceEvent::DecompositionTriggered {
                parent_trajectory_id,
                ..
            } => Some(parent_trajectory_id),
            ConvergenceEvent::SpecificationAmbiguityDetected { .. }
            | ConvergenceEvent::DecompositionRecommended { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_name_returns_snake_case() {
        let event = ConvergenceEvent::TrajectoryStarted {
            trajectory_id: "traj-1".to_string(),
            task_id: "task-1".to_string(),
            goal_id: None,
            budget: ConvergenceBudget::default(),
            timestamp: Utc::now(),
        };
        assert_eq!(event.event_name(), "trajectory_started");
    }

    #[test]
    fn test_trajectory_id_lifecycle_events() {
        let event = ConvergenceEvent::TrajectoryConverged {
            trajectory_id: "traj-42".to_string(),
            total_observations: 5,
            total_tokens_used: 100_000,
            total_fresh_starts: 1,
            timestamp: Utc::now(),
        };
        assert_eq!(event.trajectory_id(), Some("traj-42"));
    }

    #[test]
    fn test_trajectory_id_task_scoped_events() {
        let event = ConvergenceEvent::SpecificationAmbiguityDetected {
            task_id: "task-99".to_string(),
            contradictions: vec!["test A vs test B".to_string()],
            suggested_clarifications: vec!["clarify expected status code".to_string()],
        };
        assert_eq!(event.trajectory_id(), None);
    }

    #[test]
    fn test_trajectory_id_decomposition_triggered() {
        let event = ConvergenceEvent::DecompositionTriggered {
            parent_trajectory_id: "parent-1".to_string(),
            child_count: 3,
        };
        assert_eq!(event.trajectory_id(), Some("parent-1"));
    }

    #[test]
    fn test_all_event_names_are_unique() {
        let events: Vec<ConvergenceEvent> = vec![
            ConvergenceEvent::TrajectoryStarted {
                trajectory_id: String::new(),
                task_id: String::new(),
                goal_id: None,
                budget: ConvergenceBudget::default(),
                timestamp: Utc::now(),
            },
            ConvergenceEvent::TrajectoryConverged {
                trajectory_id: String::new(),
                total_observations: 0,
                total_tokens_used: 0,
                total_fresh_starts: 0,
                timestamp: Utc::now(),
            },
            ConvergenceEvent::TrajectoryExhausted {
                trajectory_id: String::new(),
                best_observation_sequence: 0,
                budget_consumed_fraction: 0.0,
                reason: String::new(),
                timestamp: Utc::now(),
            },
            ConvergenceEvent::TrajectoryTrapped {
                trajectory_id: String::new(),
                attractor_type: AttractorType::Indeterminate {
                    tendency: ConvergenceTendency::Flat,
                },
                cycle_period: None,
                escape_attempts: 0,
                timestamp: Utc::now(),
            },
            ConvergenceEvent::ObservationRecorded {
                trajectory_id: String::new(),
                observation_sequence: 0,
                convergence_delta: 0.0,
                convergence_level: 0.0,
                strategy_used: StrategyKind::RetryWithFeedback,
                budget_remaining_fraction: 1.0,
            },
            ConvergenceEvent::AttractorClassified {
                trajectory_id: String::new(),
                attractor_type: AttractorType::Indeterminate {
                    tendency: ConvergenceTendency::Flat,
                },
                confidence: 0.0,
            },
            ConvergenceEvent::StrategySelected {
                trajectory_id: String::new(),
                strategy: StrategyKind::RetryWithFeedback,
                attractor_type: AttractorType::Indeterminate {
                    tendency: ConvergenceTendency::Flat,
                },
                reason: String::new(),
                budget_remaining_fraction: 1.0,
            },
            ConvergenceEvent::ContextDegradationDetected {
                trajectory_id: String::new(),
                health_score: 0.0,
                fresh_start_number: 0,
            },
            ConvergenceEvent::BudgetExtensionRequested {
                trajectory_id: String::new(),
                current_usage_fraction: 0.0,
                requested_extension_tokens: 0,
                convergence_evidence: String::new(),
            },
            ConvergenceEvent::BudgetExtensionGranted {
                trajectory_id: String::new(),
                additional_tokens: 0,
            },
            ConvergenceEvent::BudgetExtensionDenied {
                trajectory_id: String::new(),
                reason: String::new(),
            },
            ConvergenceEvent::SpecificationAmended {
                trajectory_id: String::new(),
                amendment_source: AmendmentSource::UserHint,
                amendment_summary: String::new(),
            },
            ConvergenceEvent::SpecificationAmbiguityDetected {
                task_id: String::new(),
                contradictions: Vec::new(),
                suggested_clarifications: Vec::new(),
            },
            ConvergenceEvent::DecompositionRecommended {
                task_id: String::new(),
                subtask_count: 0,
                savings_estimate: 0.0,
            },
            ConvergenceEvent::DecompositionTriggered {
                parent_trajectory_id: String::new(),
                child_count: 0,
            },
            ConvergenceEvent::ParallelConvergenceStarted {
                trajectory_id: String::new(),
                parallel_count: 0,
            },
        ];

        let mut names: Vec<&str> = events.iter().map(|e| e.event_name()).collect();
        let count_before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            count_before,
            "all event_name() values must be unique"
        );
    }

    #[test]
    fn test_observation_recorded_serde_roundtrip() {
        let event = ConvergenceEvent::ObservationRecorded {
            trajectory_id: "traj-1".to_string(),
            observation_sequence: 3,
            convergence_delta: 0.15,
            convergence_level: 0.65,
            strategy_used: StrategyKind::FocusedRepair,
            budget_remaining_fraction: 0.42,
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConvergenceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_name(), "observation_recorded");
    }

    #[test]
    fn test_strategy_selected_serde_roundtrip() {
        let event = ConvergenceEvent::StrategySelected {
            trajectory_id: "traj-2".to_string(),
            strategy: StrategyKind::Reframe,
            attractor_type: AttractorType::LimitCycle {
                period: 2,
                cycle_signatures: vec!["sig1".to_string(), "sig2".to_string()],
            },
            reason: "limit cycle detected, exploration required".to_string(),
            budget_remaining_fraction: 0.55,
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConvergenceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event_name(), "strategy_selected");
    }
}

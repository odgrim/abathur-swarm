//! Goal federation domain model.
//!
//! Supports delegating goals to child swarms and tracking their convergence.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// State of a federated goal as tracked by the overmind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederatedGoalState {
    /// Not yet delegated to a cerebrate.
    Pending,
    /// Sent to child swarm, awaiting acknowledgment.
    Delegated,
    /// Child swarm is actively working on the goal.
    Active,
    /// Positive convergence signals received.
    Converging,
    /// Convergence contract satisfied.
    Converged,
    /// Child swarm reports failure.
    Failed,
    /// Blocked on a cross-swarm dependency.
    Gated,
}

impl FederatedGoalState {
    /// Return the string representation of this state.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delegated => "delegated",
            Self::Active => "active",
            Self::Converging => "converging",
            Self::Converged => "converged",
            Self::Failed => "failed",
            Self::Gated => "gated",
        }
    }

    /// Returns true if this is a terminal state (no further transitions expected).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Converged | Self::Failed)
    }

    /// Check whether a transition from this state to `target` is valid.
    pub fn can_transition_to(&self, target: Self) -> bool {
        matches!(
            (self, target),
            // Pending can be delegated or gated
            (Self::Pending, Self::Delegated)
            | (Self::Pending, Self::Gated)
            | (Self::Pending, Self::Failed)
            // Delegated can become active, failed, or gated
            | (Self::Delegated, Self::Active)
            | (Self::Delegated, Self::Failed)
            | (Self::Delegated, Self::Gated)
            // Active can converge, fail, or become gated
            | (Self::Active, Self::Converging)
            | (Self::Active, Self::Failed)
            | (Self::Active, Self::Gated)
            // Converging can fully converge, regress to active, or fail
            | (Self::Converging, Self::Converged)
            | (Self::Converging, Self::Active)
            | (Self::Converging, Self::Failed)
            // Gated goes to Delegated once dependencies are met (not back to Pending)
            | (Self::Gated, Self::Delegated)
            | (Self::Gated, Self::Failed)
        )
    }

    /// Parse from a string (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "delegated" => Some(Self::Delegated),
            "active" => Some(Self::Active),
            "converging" => Some(Self::Converging),
            "converged" => Some(Self::Converged),
            "failed" => Some(Self::Failed),
            "gated" => Some(Self::Gated),
            _ => None,
        }
    }
}

/// A signal type that must be satisfied for convergence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContractSignal {
    /// CI build must be passing.
    BuildPassing,
    /// Tests must pass at or above the given rate (0.0 - 1.0).
    TestsPassing { min_pass_rate: f64 },
    /// Overall convergence level must meet or exceed the threshold.
    ConvergenceLevel { min_level: f64 },
    /// At least this many tasks must be completed.
    TaskCompletionThreshold { min_completed: u32 },
    /// A custom signal evaluated by name and predicate expression.
    Custom { name: String, predicate: String },
}

/// A contract defining what signals must be satisfied for a federated goal
/// to be considered converged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceContract {
    /// Signals that must all be satisfied.
    pub required_signals: Vec<ContractSignal>,
    /// How often to poll for convergence (seconds).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    60
}

impl Default for ConvergenceContract {
    fn default() -> Self {
        Self {
            required_signals: Vec::new(),
            poll_interval_secs: default_poll_interval(),
        }
    }
}

impl ConvergenceContract {
    /// Check whether all required signals are satisfied by the given snapshot.
    ///
    /// An empty contract is trivially satisfied (by design). However, if there
    /// are required signals but the snapshot contains no data at all, we return
    /// `false` — the child hasn't reported yet and we must not prematurely converge.
    pub fn is_satisfied(&self, snapshot: &ConvergenceSignalSnapshot) -> bool {
        // Guard: required signals exist but no data received yet.
        if !self.required_signals.is_empty()
            && snapshot.signals.is_empty()
            && snapshot.convergence_level == 0.0
        {
            return false; // No data received yet
        }

        self.required_signals.iter().all(|signal| {
            match signal {
                ContractSignal::BuildPassing => snapshot
                    .signals
                    .get("build_passing")
                    .map(|v| *v >= 1.0)
                    .unwrap_or(false),
                ContractSignal::TestsPassing { min_pass_rate } => snapshot
                    .signals
                    .get("test_pass_rate")
                    .map(|v| *v >= *min_pass_rate)
                    .unwrap_or(false),
                ContractSignal::ConvergenceLevel { min_level } => {
                    snapshot.convergence_level >= *min_level
                }
                ContractSignal::TaskCompletionThreshold { min_completed } => {
                    snapshot.task_summary.completed >= *min_completed
                }
                ContractSignal::Custom { name, .. } => {
                    // Custom signals are satisfied if the named signal >= 1.0
                    snapshot
                        .signals
                        .get(name)
                        .map(|v| *v >= 1.0)
                        .unwrap_or(false)
                }
            }
        })
    }
}

/// Summary of task statuses within a federated goal scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskStatusSummary {
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
    pub running: u32,
    pub pending: u32,
}

/// A point-in-time snapshot of convergence signals from a child swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceSignalSnapshot {
    /// When this snapshot was taken.
    pub timestamp: DateTime<Utc>,
    /// Named signal values (e.g., "build_passing" -> 1.0, "test_pass_rate" -> 0.95).
    pub signals: HashMap<String, f64>,
    /// Overall convergence level (0.0 - 1.0).
    pub convergence_level: f64,
    /// Task status summary from the child swarm.
    pub task_summary: TaskStatusSummary,
}

/// A goal that has been federated (delegated) to a child swarm (cerebrate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedGoal {
    /// Unique identifier for this federated goal record.
    pub id: Uuid,
    /// The local goal ID in the overmind that this federation serves.
    pub local_goal_id: Uuid,
    /// The cerebrate (child swarm) this goal is delegated to.
    pub cerebrate_id: String,
    /// The A2A task ID in the child swarm (once delegation is accepted).
    pub remote_task_id: Option<String>,
    /// The goal ID in the child swarm (once created there).
    pub remote_goal_id: Option<Uuid>,
    /// The intent / description of what the child should achieve.
    pub intent: String,
    /// Constraints the child must follow.
    pub constraints: Vec<String>,
    /// The convergence contract that defines when this goal is satisfied.
    pub convergence_contract: ConvergenceContract,
    /// Current state of this federated goal.
    pub state: FederatedGoalState,
    /// Last received convergence signals from the child.
    pub last_signals: Option<ConvergenceSignalSnapshot>,
    /// When this federated goal was created.
    pub created_at: DateTime<Utc>,
    /// When this federated goal was last updated.
    pub updated_at: DateTime<Utc>,
}

impl FederatedGoal {
    /// Create a new federated goal in the Pending state.
    pub fn new(
        local_goal_id: Uuid,
        cerebrate_id: impl Into<String>,
        intent: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            local_goal_id,
            cerebrate_id: cerebrate_id.into(),
            remote_task_id: None,
            remote_goal_id: None,
            intent: intent.into(),
            constraints: Vec::new(),
            convergence_contract: ConvergenceContract::default(),
            state: FederatedGoalState::Pending,
            last_signals: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a constraint.
    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraints.push(constraint.into());
        self
    }

    /// Set the convergence contract.
    pub fn with_convergence_contract(mut self, contract: ConvergenceContract) -> Self {
        self.convergence_contract = contract;
        self
    }

    /// Set the remote task ID.
    pub fn with_remote_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.remote_task_id = Some(task_id.into());
        self
    }

    /// Set the remote goal ID.
    pub fn with_remote_goal_id(mut self, goal_id: Uuid) -> Self {
        self.remote_goal_id = Some(goal_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions_from_pending() {
        let pending = FederatedGoalState::Pending;
        assert!(pending.can_transition_to(FederatedGoalState::Delegated));
        assert!(pending.can_transition_to(FederatedGoalState::Gated));
        assert!(pending.can_transition_to(FederatedGoalState::Failed));
        assert!(!pending.can_transition_to(FederatedGoalState::Active));
        assert!(!pending.can_transition_to(FederatedGoalState::Converged));
        assert!(!pending.can_transition_to(FederatedGoalState::Pending));
    }

    #[test]
    fn test_state_transitions_from_delegated() {
        let delegated = FederatedGoalState::Delegated;
        assert!(delegated.can_transition_to(FederatedGoalState::Active));
        assert!(delegated.can_transition_to(FederatedGoalState::Failed));
        assert!(delegated.can_transition_to(FederatedGoalState::Gated));
        assert!(!delegated.can_transition_to(FederatedGoalState::Converged));
        assert!(!delegated.can_transition_to(FederatedGoalState::Pending));
    }

    #[test]
    fn test_state_transitions_from_active() {
        let active = FederatedGoalState::Active;
        assert!(active.can_transition_to(FederatedGoalState::Converging));
        assert!(active.can_transition_to(FederatedGoalState::Failed));
        assert!(active.can_transition_to(FederatedGoalState::Gated));
        assert!(!active.can_transition_to(FederatedGoalState::Converged));
        assert!(!active.can_transition_to(FederatedGoalState::Delegated));
    }

    #[test]
    fn test_state_transitions_from_converging() {
        let converging = FederatedGoalState::Converging;
        assert!(converging.can_transition_to(FederatedGoalState::Converged));
        assert!(converging.can_transition_to(FederatedGoalState::Active));
        assert!(converging.can_transition_to(FederatedGoalState::Failed));
        assert!(!converging.can_transition_to(FederatedGoalState::Pending));
    }

    #[test]
    fn test_terminal_states() {
        assert!(FederatedGoalState::Converged.is_terminal());
        assert!(FederatedGoalState::Failed.is_terminal());
        assert!(!FederatedGoalState::Pending.is_terminal());
        assert!(!FederatedGoalState::Active.is_terminal());
        assert!(!FederatedGoalState::Gated.is_terminal());
    }

    #[test]
    fn test_terminal_states_cannot_transition() {
        let converged = FederatedGoalState::Converged;
        assert!(!converged.can_transition_to(FederatedGoalState::Active));
        assert!(!converged.can_transition_to(FederatedGoalState::Failed));

        let failed = FederatedGoalState::Failed;
        assert!(!failed.can_transition_to(FederatedGoalState::Active));
        assert!(!failed.can_transition_to(FederatedGoalState::Converged));
    }

    #[test]
    fn test_convergence_contract_all_satisfied() {
        let contract = ConvergenceContract {
            required_signals: vec![
                ContractSignal::BuildPassing,
                ContractSignal::TestsPassing { min_pass_rate: 0.9 },
                ContractSignal::ConvergenceLevel { min_level: 0.8 },
                ContractSignal::TaskCompletionThreshold { min_completed: 5 },
            ],
            poll_interval_secs: 30,
        };

        let snapshot = ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::from([
                ("build_passing".to_string(), 1.0),
                ("test_pass_rate".to_string(), 0.95),
            ]),
            convergence_level: 0.85,
            task_summary: TaskStatusSummary {
                total: 10,
                completed: 7,
                failed: 0,
                running: 2,
                pending: 1,
            },
        };

        assert!(contract.is_satisfied(&snapshot));
    }

    #[test]
    fn test_convergence_contract_build_failing() {
        let contract = ConvergenceContract {
            required_signals: vec![ContractSignal::BuildPassing],
            poll_interval_secs: 60,
        };

        let snapshot = ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::from([("build_passing".to_string(), 0.0)]),
            convergence_level: 0.5,
            task_summary: TaskStatusSummary::default(),
        };

        assert!(!contract.is_satisfied(&snapshot));
    }

    #[test]
    fn test_convergence_contract_missing_signal() {
        let contract = ConvergenceContract {
            required_signals: vec![ContractSignal::BuildPassing],
            poll_interval_secs: 60,
        };

        // No signals at all
        let snapshot = ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::new(),
            convergence_level: 0.0,
            task_summary: TaskStatusSummary::default(),
        };

        assert!(!contract.is_satisfied(&snapshot));
    }

    #[test]
    fn test_convergence_contract_tests_below_threshold() {
        let contract = ConvergenceContract {
            required_signals: vec![ContractSignal::TestsPassing {
                min_pass_rate: 0.95,
            }],
            poll_interval_secs: 60,
        };

        let snapshot = ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::from([("test_pass_rate".to_string(), 0.80)]),
            convergence_level: 0.5,
            task_summary: TaskStatusSummary::default(),
        };

        assert!(!contract.is_satisfied(&snapshot));
    }

    #[test]
    fn test_convergence_contract_custom_signal() {
        let contract = ConvergenceContract {
            required_signals: vec![ContractSignal::Custom {
                name: "lint_clean".to_string(),
                predicate: "lint_warnings == 0".to_string(),
            }],
            poll_interval_secs: 60,
        };

        let mut satisfied_snapshot = ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::from([("lint_clean".to_string(), 1.0)]),
            convergence_level: 0.5,
            task_summary: TaskStatusSummary::default(),
        };

        assert!(contract.is_satisfied(&satisfied_snapshot));

        satisfied_snapshot
            .signals
            .insert("lint_clean".to_string(), 0.0);
        assert!(!contract.is_satisfied(&satisfied_snapshot));
    }

    #[test]
    fn test_convergence_contract_empty_is_satisfied() {
        let contract = ConvergenceContract::default();
        let snapshot = ConvergenceSignalSnapshot {
            timestamp: Utc::now(),
            signals: HashMap::new(),
            convergence_level: 0.0,
            task_summary: TaskStatusSummary::default(),
        };
        // Empty contract is trivially satisfied
        assert!(contract.is_satisfied(&snapshot));
    }

    #[test]
    fn test_federated_goal_serde_roundtrip() {
        let goal = FederatedGoal::new(Uuid::new_v4(), "cerebrate-alpha", "Implement feature X")
            .with_constraint("Must not break CI")
            .with_constraint("Follow coding standards")
            .with_convergence_contract(ConvergenceContract {
                required_signals: vec![
                    ContractSignal::BuildPassing,
                    ContractSignal::TestsPassing { min_pass_rate: 0.9 },
                ],
                poll_interval_secs: 30,
            });

        let json = serde_json::to_string(&goal).unwrap();
        let roundtrip: FederatedGoal = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtrip.id, goal.id);
        assert_eq!(roundtrip.local_goal_id, goal.local_goal_id);
        assert_eq!(roundtrip.cerebrate_id, "cerebrate-alpha");
        assert_eq!(roundtrip.intent, "Implement feature X");
        assert_eq!(roundtrip.constraints.len(), 2);
        assert_eq!(roundtrip.state, FederatedGoalState::Pending);
        assert_eq!(roundtrip.convergence_contract.required_signals.len(), 2);
        assert_eq!(roundtrip.convergence_contract.poll_interval_secs, 30);
    }

    #[test]
    fn test_federated_goal_state_as_str() {
        assert_eq!(FederatedGoalState::Pending.as_str(), "pending");
        assert_eq!(FederatedGoalState::Delegated.as_str(), "delegated");
        assert_eq!(FederatedGoalState::Active.as_str(), "active");
        assert_eq!(FederatedGoalState::Converging.as_str(), "converging");
        assert_eq!(FederatedGoalState::Converged.as_str(), "converged");
        assert_eq!(FederatedGoalState::Failed.as_str(), "failed");
        assert_eq!(FederatedGoalState::Gated.as_str(), "gated");
    }

    #[test]
    fn test_federated_goal_state_from_str() {
        assert_eq!(
            FederatedGoalState::parse("pending"),
            Some(FederatedGoalState::Pending)
        );
        assert_eq!(
            FederatedGoalState::parse("ACTIVE"),
            Some(FederatedGoalState::Active)
        );
        assert_eq!(FederatedGoalState::parse("invalid"), None);
    }
}

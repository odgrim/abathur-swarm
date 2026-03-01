//! Workflow state machine types.
//!
//! Stored as a JSON blob in `task.context.custom["workflow_state"]`.
//! The state machine drives phase ordering and tracks completion while
//! the Overmind provides gate decisions and agent creation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// State of a task's progression through a workflow.
///
/// ```text
/// Pending → PhaseRunning → PhaseGate → ... → Completed | Rejected | Failed
///                        ↘ FanningOut → Aggregating → ...
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkflowState {
    /// Enrolled in workflow, awaiting first phase advance.
    Pending {
        workflow_name: String,
    },
    /// Subtask(s) executing current phase; overmind creates agent(s) to work on them.
    PhaseRunning {
        workflow_name: String,
        phase_index: usize,
        phase_name: String,
        subtask_ids: Vec<Uuid>,
    },
    /// Phase has been split into parallel slices; subtasks executing.
    FanningOut {
        workflow_name: String,
        phase_index: usize,
        phase_name: String,
        subtask_ids: Vec<Uuid>,
        slice_count: usize,
    },
    /// All fan-out subtasks done; aggregation running before advancing.
    Aggregating {
        workflow_name: String,
        phase_index: usize,
        phase_name: String,
        subtask_ids: Vec<Uuid>,
    },
    /// Intent verification in progress; subtasks done, verifier running.
    Verifying {
        workflow_name: String,
        phase_index: usize,
        phase_name: String,
        subtask_ids: Vec<Uuid>,
        retry_count: u32,
    },
    /// Previous phase completed; overmind decides single vs fan-out.
    PhaseReady {
        workflow_name: String,
        phase_index: usize,
        phase_name: String,
    },
    /// Gate phase; subtasks done, awaiting overmind verdict.
    PhaseGate {
        workflow_name: String,
        phase_index: usize,
        phase_name: String,
    },
    /// All phases done.
    Completed {
        workflow_name: String,
    },
    /// Gate verdict rejected the task.
    Rejected {
        workflow_name: String,
        phase_index: usize,
        reason: String,
    },
    /// Unrecoverable error.
    Failed {
        workflow_name: String,
        error: String,
    },
}

impl WorkflowState {
    /// Get the workflow name regardless of current state.
    pub fn workflow_name(&self) -> &str {
        match self {
            Self::Pending { workflow_name }
            | Self::PhaseRunning { workflow_name, .. }
            | Self::FanningOut { workflow_name, .. }
            | Self::Aggregating { workflow_name, .. }
            | Self::Verifying { workflow_name, .. }
            | Self::PhaseReady { workflow_name, .. }
            | Self::PhaseGate { workflow_name, .. }
            | Self::Completed { workflow_name }
            | Self::Rejected { workflow_name, .. }
            | Self::Failed { workflow_name, .. } => workflow_name,
        }
    }

    /// Whether this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Rejected { .. } | Self::Failed { .. })
    }

    /// Get the current phase index, if in an active phase.
    pub fn phase_index(&self) -> Option<usize> {
        match self {
            Self::PhaseRunning { phase_index, .. }
            | Self::FanningOut { phase_index, .. }
            | Self::Aggregating { phase_index, .. }
            | Self::Verifying { phase_index, .. }
            | Self::PhaseReady { phase_index, .. }
            | Self::PhaseGate { phase_index, .. } => Some(*phase_index),
            _ => None,
        }
    }
}

/// Verdict provided by the overmind at a gate phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVerdict {
    /// Approve and advance to the next phase.
    Approve,
    /// Reject the task entirely.
    Reject,
    /// Re-run the current phase.
    Rework,
}

/// A slice for fan-out: creates one parallel subtask per slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutSlice {
    /// Description of this slice's work.
    pub description: String,
    /// Agent template to assign to this subtask (sets `agent_type` at creation time).
    #[serde(default)]
    pub agent: Option<String>,
    /// Additional context for the subtask.
    #[serde(default)]
    pub context: std::collections::HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_state_serde_roundtrip() {
        let state = WorkflowState::PhaseRunning {
            workflow_name: "code".to_string(),
            phase_index: 2,
            phase_name: "implement".to_string(),
            subtask_ids: vec![Uuid::new_v4()],
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: WorkflowState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn test_workflow_state_terminal() {
        assert!(!WorkflowState::Pending { workflow_name: "code".to_string() }.is_terminal());
        assert!(WorkflowState::Completed { workflow_name: "code".to_string() }.is_terminal());
        assert!(WorkflowState::Rejected {
            workflow_name: "code".to_string(),
            phase_index: 0,
            reason: "bad".to_string(),
        }.is_terminal());
        assert!(WorkflowState::Failed {
            workflow_name: "code".to_string(),
            error: "oops".to_string(),
        }.is_terminal());
    }

    #[test]
    fn test_gate_verdict_serde() {
        let v = GateVerdict::Approve;
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"approve\"");
        let deserialized: GateVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, GateVerdict::Approve);
    }

    #[test]
    fn test_verifying_state_serde_roundtrip() {
        let state = WorkflowState::Verifying {
            workflow_name: "code".to_string(),
            phase_index: 2,
            phase_name: "implement".to_string(),
            subtask_ids: vec![Uuid::new_v4()],
            retry_count: 1,
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: WorkflowState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
        assert!(!state.is_terminal());
        assert_eq!(state.phase_index(), Some(2));
        assert_eq!(state.workflow_name(), "code");
    }

    #[test]
    fn test_workflow_name_accessor() {
        let state = WorkflowState::PhaseGate {
            workflow_name: "analysis".to_string(),
            phase_index: 1,
            phase_name: "analyze".to_string(),
        };
        assert_eq!(state.workflow_name(), "analysis");
    }

    #[test]
    fn test_fanning_out_state_serde_roundtrip() {
        let state = WorkflowState::FanningOut {
            workflow_name: "code".to_string(),
            phase_index: 3,
            phase_name: "implement".to_string(),
            subtask_ids: vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
            slice_count: 3,
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: WorkflowState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
        assert!(!state.is_terminal());
        assert_eq!(state.phase_index(), Some(3));
        assert_eq!(state.workflow_name(), "code");
    }

    #[test]
    fn test_aggregating_state_serde_roundtrip() {
        let state = WorkflowState::Aggregating {
            workflow_name: "code".to_string(),
            phase_index: 2,
            phase_name: "research".to_string(),
            subtask_ids: vec![Uuid::new_v4()],
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: WorkflowState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
        assert!(!state.is_terminal());
        assert_eq!(state.phase_index(), Some(2));
        assert_eq!(state.workflow_name(), "code");
    }
}

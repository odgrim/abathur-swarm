//! Workflow state/status invariants and phase classification helpers.

use crate::domain::models::task::TaskStatus;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::workflow_template::WorkflowTemplate;

/// Validate that a TaskStatus is consistent with a WorkflowState.
///
/// Returns `Ok(())` if the pairing is valid, or `Err(description)` if not.
/// This is purely for observability — callers should log warnings, not crash.
pub(super) fn validate_state_consistency(
    task_status: TaskStatus,
    workflow_state: &WorkflowState,
) -> Result<(), String> {
    match (task_status, workflow_state) {
        // Validating is only valid with Verifying
        (TaskStatus::Validating, WorkflowState::Verifying { .. }) => Ok(()),
        (TaskStatus::Validating, ws) => Err(format!(
            "TaskStatus::Validating is only valid with WorkflowState::Verifying, got {:?}",
            ws
        )),

        // Running is valid with active workflow states
        (TaskStatus::Running, WorkflowState::PhaseRunning { .. })
        | (TaskStatus::Running, WorkflowState::FanningOut { .. })
        | (TaskStatus::Running, WorkflowState::Aggregating { .. })
        | (TaskStatus::Running, WorkflowState::PhaseReady { .. })
        | (TaskStatus::Running, WorkflowState::PhaseGate { .. }) => Ok(()),
        (TaskStatus::Running, ws) => Err(format!(
            "TaskStatus::Running is only valid with PhaseRunning/FanningOut/Aggregating/PhaseReady/PhaseGate, got {:?}",
            ws
        )),

        // Terminal TaskStatus is compatible with terminal WorkflowState or any
        // state (workflow may not have caught up yet)
        (TaskStatus::Complete, _) | (TaskStatus::Failed, _) | (TaskStatus::Canceled, _) => Ok(()),

        // Pending/Ready/Blocked are compatible with Pending workflow state
        (TaskStatus::Pending, WorkflowState::Pending { .. })
        | (TaskStatus::Ready, WorkflowState::Pending { .. })
        | (TaskStatus::Blocked, WorkflowState::Pending { .. }) => Ok(()),
        (TaskStatus::Pending, ws) | (TaskStatus::Ready, ws) | (TaskStatus::Blocked, ws) => {
            Err(format!(
                "TaskStatus::Pending/Ready/Blocked is only valid with WorkflowState::Pending, got {:?}",
                ws
            ))
        }
    }
}

/// Whether a phase is a gate phase.
///
/// Gate phases park at `PhaseGate` and require an overmind verdict.
/// Checks the `gate` field on the phase in the workflow template.
/// Falls back to hardcoded name matching if the template or phase is
/// not found (backward compatibility for in-flight workflows).
pub(super) fn is_gate_phase(
    templates: &std::collections::HashMap<String, WorkflowTemplate>,
    workflow_name: &str,
    phase_index: usize,
    phase_name: &str,
) -> bool {
    if let Some(template) = templates.get(workflow_name)
        && let Some(phase) = template.phases.get(phase_index)
    {
        return phase.gate;
    }
    // Fallback for backward compatibility
    matches!(phase_name, "triage" | "validation" | "review")
}

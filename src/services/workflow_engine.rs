//! Deterministic workflow state machine.
//!
//! The `WorkflowEngine` drives phase ordering and tracks completion for tasks
//! enrolled in a workflow. It is purely deterministic — no LLM calls. The
//! Overmind provides gate decisions and creates agents for each phase.
//!
//! Workflow state is stored as a JSON blob in `task.context.custom["workflow_state"]`.

use std::sync::Arc;

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::task::{ExecutionMode, Task, TaskSource, TaskStatus, TaskType};
use crate::domain::models::workflow_state::{FanOutSlice, GateVerdict, WorkflowState};
use crate::domain::models::workflow_template::WorkflowTemplate;
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::{EventBus, EventCategory, EventPayload, EventSeverity};
use crate::services::event_factory;
use crate::services::task_service::TaskService;

/// Whether a phase name is a gate phase.
///
/// Gate phases park at `PhaseGate` and require an overmind verdict.
fn is_gate_phase(phase_name: &str) -> bool {
    matches!(phase_name, "triage" | "review")
}

/// Result of an `advance` call, giving the overmind enough info to fan out.
#[derive(Debug, Clone)]
pub enum AdvanceResult {
    /// The next phase is ready; the overmind should call `fan_out` to create subtasks.
    PhaseReady {
        phase_index: usize,
        phase_name: String,
    },
    /// All phases are done; the workflow completed successfully.
    Completed,
}

/// Result of a `fan_out` call.
#[derive(Debug, Clone)]
pub struct FanOutResult {
    pub subtask_ids: Vec<Uuid>,
    pub phase_index: usize,
    pub phase_name: String,
}

/// Workflow status summary returned by `get_state`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkflowStatus {
    pub task_id: Uuid,
    pub workflow_name: String,
    pub state: WorkflowState,
    pub total_phases: usize,
    pub current_phase_index: Option<usize>,
    pub current_phase_name: Option<String>,
    pub is_verifying: bool,
    pub verification_retry_count: Option<u32>,
}

/// The deterministic workflow engine.
pub struct WorkflowEngine<T: TaskRepository> {
    task_repo: Arc<T>,
    task_service: TaskService<T>,
    event_bus: Arc<EventBus>,
    templates: std::collections::HashMap<String, WorkflowTemplate>,
    verification_enabled: bool,
}

impl<T: TaskRepository + 'static> WorkflowEngine<T> {
    pub fn new(
        task_repo: Arc<T>,
        task_service: TaskService<T>,
        event_bus: Arc<EventBus>,
        verification_enabled: bool,
    ) -> Self {
        Self {
            task_repo,
            task_service,
            event_bus,
            templates: WorkflowTemplate::builtin_templates(),
            verification_enabled,
        }
    }

    /// Look up the workflow template by name.
    fn get_template(&self, name: &str) -> DomainResult<&WorkflowTemplate> {
        self.templates.get(name).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Unknown workflow template: {}", name))
        })
    }

    /// Read workflow state from task context.
    fn read_state(task: &Task) -> Option<WorkflowState> {
        task.context
            .custom
            .get("workflow_state")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Public accessor for reading workflow state from a task (used by handlers).
    pub fn read_state_from_task(task: &Task) -> Option<WorkflowState> {
        Self::read_state(task)
    }

    /// Write workflow state to task context and persist via TaskService
    /// (with retry-on-conflict).
    async fn write_state(&self, task_id: Uuid, state: &WorkflowState) -> DomainResult<()> {
        self.task_service.update_workflow_state(task_id, state).await
    }

    // ========================================================================
    // Public API
    // ========================================================================

    // enroll() — REMOVED
    // Auto-enrollment happens in TaskService::submit_task() with template
    // validation (fix C). The MCP enroll tool was also removed. Tests use
    // submit_task() or write workflow state directly.

    /// Change the workflow spine for a task that is still in `Pending` state.
    ///
    /// This allows the overmind to override the auto-selected workflow before
    /// the first phase starts. Validates the template exists, verifies the
    /// task is in `Pending` state, and updates both the workflow state and
    /// routing hints.
    pub async fn select_workflow(
        &self,
        task_id: Uuid,
        workflow_name: &str,
    ) -> DomainResult<WorkflowStatus> {
        // Validate the target template exists
        let _template = self.get_template(workflow_name)?;

        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        // Only allow spine change while still in Pending
        match &state {
            WorkflowState::Pending { .. } => {}
            other => {
                return Err(DomainError::ValidationFailed(format!(
                    "Cannot change workflow spine for task {} — must be in Pending state (current: {:?})",
                    task_id, other
                )));
            }
        }

        // Overwrite workflow state and routing hints atomically.
        // We already loaded `task` fresh above (with version), so set both
        // fields and persist in one write. On conflict, the caller can retry.
        let new_state = WorkflowState::Pending {
            workflow_name: workflow_name.to_string(),
        };
        let value = serde_json::to_value(&new_state)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        task.context
            .custom
            .insert("workflow_state".to_string(), value);
        task.routing_hints.workflow_name = Some(workflow_name.to_string());
        task.updated_at = chrono::Utc::now();
        self.task_repo.update(&task).await?;

        tracing::info!(
            task_id = %task_id,
            workflow_name = %workflow_name,
            "Workflow spine changed via select_workflow"
        );

        self.get_state(task_id).await
    }

    /// Advance to the next phase.
    ///
    /// Creates the next phase's subtask and transitions to `PhaseRunning`.
    /// Returns the subtask info so the overmind can create an agent for it.
    pub async fn advance(&self, task_id: Uuid) -> DomainResult<AdvanceResult> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let (workflow_name, next_index) = match &state {
            WorkflowState::Pending { workflow_name } => (workflow_name.clone(), 0),
            WorkflowState::PhaseReady { .. } => {
                return Err(DomainError::ValidationFailed(format!(
                    "Task {} is already in PhaseReady — call fan_out to create subtasks",
                    task_id
                )));
            }
            WorkflowState::PhaseGate {
                workflow_name,
                phase_index,
                ..
            } => (workflow_name.clone(), phase_index + 1),
            WorkflowState::PhaseRunning {
                workflow_name,
                phase_index,
                subtask_ids,
                ..
            }
            | WorkflowState::FanningOut {
                workflow_name,
                phase_index,
                subtask_ids,
                ..
            }
            | WorkflowState::Aggregating {
                workflow_name,
                phase_index,
                subtask_ids,
                ..
            }
            | WorkflowState::Verifying {
                workflow_name,
                phase_index,
                subtask_ids,
                ..
            } => {
                // Guard: only allow advance from active phase states if all
                // subtasks are terminal. This prevents double-advance from
                // concurrent callers (spawn_task_agent + workflow_advance MCP tool).
                let all_done = self.all_subtasks_done(subtask_ids).await?;
                if !all_done {
                    return Err(DomainError::ValidationFailed(format!(
                        "Cannot advance task {} — phase subtask(s) are still running",
                        task_id
                    )));
                }
                (workflow_name.clone(), phase_index + 1)
            }
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Cannot advance task {} from state {:?}",
                    task_id, state
                )));
            }
        };

        let template = self.get_template(&workflow_name)?;

        if next_index >= template.phases.len() {
            // All phases done
            let completed = WorkflowState::Completed {
                workflow_name: workflow_name.clone(),
            };
            self.write_state(task_id, &completed).await?;

            // Complete the parent task via TaskService (emits TaskCompleted + TaskExecutionRecorded)
            self.complete_parent_task(task_id).await?;

            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    None,
                    Some(task_id),
                    EventPayload::WorkflowCompleted { task_id },
                ))
                .await;

            return Ok(AdvanceResult::Completed);
        }

        let from_phase = state.phase_index();
        let phase = &template.phases[next_index];

        // Transition to PhaseReady — the overmind must call fan_out() next
        let new_state = WorkflowState::PhaseReady {
            workflow_name: workflow_name.clone(),
            phase_index: next_index,
            phase_name: phase.name.clone(),
        };
        self.write_state(task_id, &new_state).await?;

        // Emit events
        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                None,
                Some(task_id),
                EventPayload::WorkflowPhaseReady {
                    task_id,
                    phase_index: next_index,
                    phase_name: phase.name.clone(),
                },
            ))
            .await;

        if let Some(from) = from_phase {
            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    None,
                    Some(task_id),
                    EventPayload::WorkflowAdvanced {
                        task_id,
                        from_phase: from,
                        to_phase: next_index,
                    },
                ))
                .await;
        }

        Ok(AdvanceResult::PhaseReady {
            phase_index: next_index,
            phase_name: phase.name.clone(),
        })
    }

    /// Handle a phase subtask completing.
    ///
    /// Supports PhaseRunning (single subtask), FanningOut (parallel slices),
    /// and Aggregating (aggregation subtask) states.
    ///
    /// When all subtasks for the current phase are done:
    /// - FanningOut → call handle_fan_in() to create aggregation task
    /// - Aggregating → aggregation subtask done, proceed to verification/gate/advance
    /// - If all subtasks converged → skip verification
    /// - If phase has `verify: true` and verification is enabled → `Verifying` (parent → Validating)
    /// - Else if gate phase → `PhaseGate`
    /// - Otherwise → auto-advance to next phase
    pub async fn handle_phase_complete(
        &self,
        parent_task_id: Uuid,
        subtask_id: Uuid,
    ) -> DomainResult<()> {
        let task = self
            .task_repo
            .get(parent_task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(parent_task_id))?;

        let state = match Self::read_state(&task) {
            Some(s) => s,
            None => return Ok(()), // Not a workflow task
        };

        let (workflow_name, phase_index, phase_name, subtask_ids) = match &state {
            WorkflowState::PhaseRunning {
                workflow_name,
                phase_index,
                phase_name,
                subtask_ids,
            } => (
                workflow_name.clone(),
                *phase_index,
                phase_name.clone(),
                subtask_ids.clone(),
            ),
            WorkflowState::FanningOut {
                workflow_name,
                phase_index,
                phase_name,
                subtask_ids,
                ..
            } => {
                // Check if all fan-out subtasks are done
                let all_done = self.all_subtasks_done(subtask_ids).await?;
                if !all_done {
                    return Ok(());
                }
                let any_failed = self.any_subtask_failed(subtask_ids).await?;
                if any_failed {
                    // Try to retry failed subtasks before giving up
                    let retried = self
                        .retry_failed_phase_subtasks(
                            parent_task_id,
                            *phase_index,
                            phase_name,
                            subtask_ids,
                        )
                        .await?;
                    if retried {
                        return Ok(());
                    }
                    // Exhausted retries → fail
                    let phase_retry_key =
                        format!("phase_{}_retry_count", phase_index);
                    let parent = self
                        .task_repo
                        .get(parent_task_id)
                        .await?
                        .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                    let phase_retry_count = parent
                        .context
                        .custom
                        .get(&phase_retry_key)
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let error_msg = format!(
                        "Phase '{}' fan-out subtask failed after {} retries",
                        phase_name, phase_retry_count,
                    );
                    self.fail_workflow_phase(
                        parent_task_id,
                        &workflow_name,
                        *phase_index,
                        &phase_name,
                        &error_msg,
                    ).await?;
                    return Ok(());
                }
                // All fan-out subtasks done → handle fan-in
                return self.handle_fan_in(parent_task_id).await;
            }
            WorkflowState::Aggregating {
                workflow_name,
                phase_index,
                phase_name,
                subtask_ids,
            } => {
                // The aggregation subtask (last in subtask_ids) must be done
                let all_done = self.all_subtasks_done(subtask_ids).await?;
                if !all_done {
                    return Ok(());
                }
                // Aggregation complete → proceed to verification/gate/advance
                (
                    workflow_name.clone(),
                    *phase_index,
                    phase_name.clone(),
                    subtask_ids.clone(),
                )
            }
            _ => return Ok(()), // Not in a completable state
        };

        // Guard: ignore completions for subtasks not in the current phase
        if !subtask_ids.contains(&subtask_id) {
            tracing::warn!(
                parent_id = %parent_task_id,
                subtask_id = %subtask_id,
                "Ignoring stale phase completion — subtask not in current phase"
            );
            return Ok(());
        }

        // Check if ALL subtasks for this phase are done
        let all_done = self.all_subtasks_done(&subtask_ids).await?;
        if !all_done {
            return Ok(());
        }

        // Check if any subtask failed — retry at the phase level before
        // failing the entire workflow.
        let any_failed = self.any_subtask_failed(&subtask_ids).await?;
        if any_failed {
            let retried = self
                .retry_failed_phase_subtasks(
                    parent_task_id,
                    phase_index,
                    &phase_name,
                    &subtask_ids,
                )
                .await?;
            if retried {
                return Ok(());
            }
            // Exhausted phase retries (or no subtask was retryable) → fail the workflow
            let phase_retry_key = format!("phase_{}_retry_count", phase_index);
            let parent = self
                .task_repo
                .get(parent_task_id)
                .await?
                .ok_or(DomainError::TaskNotFound(parent_task_id))?;
            let phase_retry_count = parent
                .context
                .custom
                .get(&phase_retry_key)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let error_msg = format!(
                "Phase '{}' subtask failed after {} retries",
                phase_name, phase_retry_count
            );
            self.fail_workflow_phase(
                parent_task_id,
                &workflow_name,
                phase_index,
                &phase_name,
                &error_msg,
            ).await?;
            return Ok(());
        }

        // Check if this phase requires verification
        let template = self.get_template(&workflow_name)?;
        let phase = &template.phases[phase_index];

        // Skip verification if all subtasks already converged (Step 5.2)
        let all_converged = self.all_subtasks_converged(&subtask_ids).await?;

        if phase.verify && self.verification_enabled && !all_converged {
            // Transition parent TaskStatus to Validating via TaskService
            {
                let (_task, events) = self.task_service.transition_to_validating(parent_task_id).await?;
                for evt in events {
                    self.event_bus.publish(evt).await;
                }
            }

            // Read the current verification retry count from task context so that
            // rework attempts produce a distinct idempotency key in the handler.
            let current_retry_count = {
                let parent = self
                    .task_repo
                    .get(parent_task_id)
                    .await?
                    .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                parent
                    .context
                    .custom
                    .get("verification_retry_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32
            };

            // Transition to Verifying state
            let verifying_state = WorkflowState::Verifying {
                workflow_name: workflow_name.clone(),
                phase_index,
                phase_name: phase_name.clone(),
                subtask_ids: subtask_ids.clone(),
                retry_count: current_retry_count,
            };
            self.write_state(parent_task_id, &verifying_state).await?;

            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    None,
                    Some(parent_task_id),
                    EventPayload::WorkflowVerificationRequested {
                        task_id: parent_task_id,
                        phase_index,
                        phase_name,
                        retry_count: current_retry_count,
                    },
                ))
                .await;

            return Ok(());
        }

        // Is this a gate phase?
        if is_gate_phase(&phase_name) {
            let gate_state = WorkflowState::PhaseGate {
                workflow_name: workflow_name.clone(),
                phase_index,
                phase_name: phase_name.clone(),
            };
            self.write_state(parent_task_id, &gate_state).await?;

            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    None,
                    Some(parent_task_id),
                    EventPayload::WorkflowGateReached {
                        task_id: parent_task_id,
                        phase_index,
                        phase_name,
                    },
                ))
                .await;

            return Ok(());
        }

        // Auto-advance to next phase (Step 6.2)
        let next_index = phase_index + 1;
        if next_index >= template.phases.len() {
            // All phases done
            let completed = WorkflowState::Completed {
                workflow_name: workflow_name.clone(),
            };
            self.write_state(parent_task_id, &completed).await?;

            // Complete the parent task via TaskService
            self.complete_parent_task(parent_task_id).await?;

            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    None,
                    Some(parent_task_id),
                    EventPayload::WorkflowCompleted {
                        task_id: parent_task_id,
                    },
                ))
                .await;
        } else {
            // Transition to PhaseReady — the overmind decides single vs fan-out
            let next_phase = &template.phases[next_index];
            let ready_state = WorkflowState::PhaseReady {
                workflow_name: workflow_name.clone(),
                phase_index: next_index,
                phase_name: next_phase.name.clone(),
            };
            self.write_state(parent_task_id, &ready_state).await?;

            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    None,
                    Some(parent_task_id),
                    EventPayload::WorkflowPhaseReady {
                        task_id: parent_task_id,
                        phase_index: next_index,
                        phase_name: next_phase.name.clone(),
                    },
                ))
                .await;

            tracing::info!(
                task_id = %parent_task_id,
                phase = %next_phase.name,
                phase_index = next_index,
                "Workflow phase ready — awaiting overmind advance/fan_out"
            );
        }

        Ok(())
    }

    /// Handle the result of intent verification for a phase.
    ///
    /// Only valid when the workflow is in `Verifying` state.
    /// - Satisfied → advance (gate or complete)
    /// - Failed with retries remaining → auto-rework
    /// - Failed with retries exhausted → escalate to PhaseGate
    pub async fn handle_verification_result(
        &self,
        parent_task_id: Uuid,
        satisfied: bool,
        summary: &str,
    ) -> DomainResult<()> {
        let task = self
            .task_repo
            .get(parent_task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(parent_task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", parent_task_id))
        })?;

        let (workflow_name, phase_index, phase_name, _subtask_ids, retry_count) = match &state {
            WorkflowState::Verifying {
                workflow_name,
                phase_index,
                phase_name,
                subtask_ids,
                retry_count,
            } => (
                workflow_name.clone(),
                *phase_index,
                phase_name.clone(),
                subtask_ids.clone(),
                *retry_count,
            ),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Task {} is not in Verifying state (current: {:?})",
                    parent_task_id, state
                )));
            }
        };

        let template = self.get_template(&workflow_name)?;
        let max_retries = template.max_verification_retries;

        // Emit completion event
        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                None,
                Some(parent_task_id),
                EventPayload::WorkflowVerificationCompleted {
                    task_id: parent_task_id,
                    phase_index,
                    phase_name: phase_name.clone(),
                    satisfied,
                    retry_count,
                    summary: summary.to_string(),
                },
            ))
            .await;

        if satisfied {
            // Transition parent TaskStatus: Validating -> Running via TaskService
            let _ = self.task_service.transition_to_running(parent_task_id).await?;

            // Verification passed — proceed
            if is_gate_phase(&phase_name) {
                let gate_state = WorkflowState::PhaseGate {
                    workflow_name: workflow_name.clone(),
                    phase_index,
                    phase_name: phase_name.clone(),
                };
                self.write_state(parent_task_id, &gate_state).await?;

                self.event_bus
                    .publish(event_factory::make_event(
                        EventSeverity::Info,
                        EventCategory::Workflow,
                        None,
                        Some(parent_task_id),
                        EventPayload::WorkflowGateReached {
                            task_id: parent_task_id,
                            phase_index,
                            phase_name,
                        },
                    ))
                    .await;
            } else {
                // Transition to PhaseReady — overmind decides single vs fan-out (Step 6.1)
                let next_index = phase_index + 1;
                if next_index >= template.phases.len() {
                    let completed = WorkflowState::Completed {
                        workflow_name: workflow_name.clone(),
                    };
                    self.write_state(parent_task_id, &completed).await?;

                    // Complete the parent task via TaskService
                    self.complete_parent_task(parent_task_id).await?;

                    self.event_bus
                        .publish(event_factory::make_event(
                            EventSeverity::Info,
                            EventCategory::Workflow,
                            None,
                            Some(parent_task_id),
                            EventPayload::WorkflowCompleted {
                                task_id: parent_task_id,
                            },
                        ))
                        .await;
                } else {
                    // Transition to PhaseReady
                    let next_phase = &template.phases[next_index];
                    let ready_state = WorkflowState::PhaseReady {
                        workflow_name: workflow_name.clone(),
                        phase_index: next_index,
                        phase_name: next_phase.name.clone(),
                    };
                    self.write_state(parent_task_id, &ready_state).await?;

                    self.event_bus
                        .publish(event_factory::make_event(
                            EventSeverity::Info,
                            EventCategory::Workflow,
                            None,
                            Some(parent_task_id),
                            EventPayload::WorkflowPhaseReady {
                                task_id: parent_task_id,
                                phase_index: next_index,
                                phase_name: next_phase.name.clone(),
                            },
                        ))
                        .await;

                    tracing::info!(
                        task_id = %parent_task_id,
                        phase = %next_phase.name,
                        phase_index = next_index,
                        "Workflow phase ready after verification — awaiting overmind advance/fan_out"
                    );
                }
            }
        } else if retry_count < max_retries {
            // Failed with retries remaining — auto-rework
            self.store_verification_feedback(parent_task_id, summary).await?;

            // Reset state so advance() re-creates the phase subtask
            if phase_index == 0 {
                let pending = WorkflowState::Pending {
                    workflow_name: workflow_name.clone(),
                };
                self.write_state(parent_task_id, &pending).await?;
            } else {
                let prev_phase_name = template
                    .phases
                    .get(phase_index - 1)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let rework_state = WorkflowState::PhaseGate {
                    workflow_name: workflow_name.clone(),
                    phase_index: phase_index - 1,
                    phase_name: prev_phase_name,
                };
                self.write_state(parent_task_id, &rework_state).await?;
            }

            // Auto-advance to prepare the phase for rework
            match self.advance(parent_task_id).await {
                Ok(AdvanceResult::PhaseReady { phase_name: rework_phase, .. }) => {
                    tracing::info!(
                        task_id = %parent_task_id,
                        phase = %rework_phase,
                        retry = retry_count + 1,
                        "Workflow auto-rework: phase ready for rework with verification feedback"
                    );

                    // Update retry count on the parent task via TaskService
                    self.task_service.update_task_context(
                        parent_task_id,
                        vec![("verification_retry_count".to_string(), serde_json::json!(retry_count + 1))],
                    ).await?;
                }
                Ok(AdvanceResult::Completed) => {
                    tracing::info!(
                        task_id = %parent_task_id,
                        retry = retry_count + 1,
                        "Workflow completed during rework advance"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %parent_task_id,
                        error = %e,
                        "Workflow auto-rework advance failed, escalating to gate"
                    );
                    // Fall through to gate escalation
                    let gate_state = WorkflowState::PhaseGate {
                        workflow_name,
                        phase_index,
                        phase_name: phase_name.clone(),
                    };
                    self.write_state(parent_task_id, &gate_state).await?;

                    self.event_bus
                        .publish(event_factory::make_event(
                            EventSeverity::Warning,
                            EventCategory::Workflow,
                            None,
                            Some(parent_task_id),
                            EventPayload::WorkflowGateReached {
                                task_id: parent_task_id,
                                phase_index,
                                phase_name,
                            },
                        ))
                        .await;
                }
            }
        } else {
            // Retries exhausted — escalate to PhaseGate for overmind decision
            self.store_verification_feedback(parent_task_id, summary).await?;

            let gate_state = WorkflowState::PhaseGate {
                workflow_name,
                phase_index,
                phase_name: phase_name.clone(),
            };
            self.write_state(parent_task_id, &gate_state).await?;

            self.event_bus
                .publish(event_factory::make_event(
                    EventSeverity::Warning,
                    EventCategory::Workflow,
                    None,
                    Some(parent_task_id),
                    EventPayload::WorkflowGateReached {
                        task_id: parent_task_id,
                        phase_index,
                        phase_name,
                    },
                ))
                .await;
        }

        Ok(())
    }

    /// Provide a verdict at a gate phase.
    ///
    /// Returns `Some(AdvanceResult)` for `Approve` (since it auto-advances),
    /// or `None` for `Reject` / `Rework`.
    pub async fn provide_verdict(
        &self,
        task_id: Uuid,
        verdict: GateVerdict,
        reason: &str,
    ) -> DomainResult<Option<AdvanceResult>> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let (workflow_name, phase_index, _phase_name) = match &state {
            WorkflowState::PhaseGate {
                workflow_name,
                phase_index,
                phase_name,
            } => (workflow_name.clone(), *phase_index, phase_name.clone()),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Task {} is not at a gate phase",
                    task_id
                )));
            }
        };

        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                None,
                Some(task_id),
                EventPayload::WorkflowGateVerdict {
                    task_id,
                    phase_index,
                    verdict: format!("{:?}", verdict),
                },
            ))
            .await;

        match verdict {
            GateVerdict::Approve => {
                // Auto-advance to the next phase after approval
                let result = self.advance(task_id).await?;
                Ok(Some(result))
            }
            GateVerdict::Reject => {
                let rejection_reason = reason.to_string();
                let rejected = WorkflowState::Rejected {
                    workflow_name,
                    phase_index,
                    reason: rejection_reason.clone(),
                };
                self.write_state(task_id, &rejected).await?;

                // Fail parent task via TaskService
                let error_msg = format!("Workflow rejected at phase {}: {}", phase_index, rejection_reason);
                self.fail_parent_task(task_id, &error_msg).await?;

                Ok(None)
            }
            GateVerdict::Rework => {
                // Re-run: go back to Pending-like state so advance() re-creates the phase subtask
                // We set it to PhaseGate at (phase_index - 1) so advance() will create phase_index again
                if phase_index == 0 {
                    let pending = WorkflowState::Pending {
                        workflow_name,
                    };
                    self.write_state(task_id, &pending).await?;
                } else {
                    let prev_phase_name = self
                        .get_template(&workflow_name)?
                        .phases
                        .get(phase_index - 1)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    let rework_state = WorkflowState::PhaseGate {
                        workflow_name,
                        phase_index: phase_index - 1,
                        phase_name: prev_phase_name,
                    };
                    self.write_state(task_id, &rework_state).await?;
                }
                Ok(None)
            }
        }
    }

    /// Fan out the current phase into N parallel subtasks.
    pub async fn fan_out(
        &self,
        task_id: Uuid,
        slices: Vec<FanOutSlice>,
    ) -> DomainResult<FanOutResult> {
        if slices.is_empty() {
            return Err(DomainError::ValidationFailed(
                "fan_out requires at least one slice".to_string(),
            ));
        }

        // Every slice must specify an agent — the Overmind is responsible for
        // creating/selecting the right specialist before fanning out.
        for (i, slice) in slices.iter().enumerate() {
            if slice.agent.is_none() {
                return Err(DomainError::ValidationFailed(
                    format!(
                        "fan_out slice {} is missing required `agent` field — \
                         create or select an agent template and set `agent` on every slice",
                        i
                    ),
                ));
            }
        }

        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let (workflow_name, phase_index, phase_name) = match &state {
            // fan_out only accepts PhaseReady — overmind must call advance() first
            WorkflowState::PhaseReady {
                workflow_name,
                phase_index,
                phase_name,
            } => (workflow_name.clone(), *phase_index, phase_name.clone()),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Cannot fan_out task {} from state {:?} — call advance() first to reach PhaseReady",
                    task_id, state
                )));
            }
        };

        let template = self.get_template(&workflow_name)?;
        let phase = &template.phases[phase_index];

        let mut subtask_ids = Vec::new();
        for (i, slice) in slices.iter().enumerate() {
            let title = format!(
                "[{}/{}:{}] {} (slice {}/{})",
                workflow_name,
                phase_index,
                phase.name,
                slice.description.chars().take(50).collect::<String>(),
                i + 1,
                slices.len()
            );
            let description = format!(
                "Workflow: {}\nPhase: {} ({}/{})\nSlice {}/{}:\n\n{}\n\nParent task: {}",
                workflow_name,
                phase.name,
                phase_index + 1,
                template.phases.len(),
                i + 1,
                slices.len(),
                slice.description,
                task.description
            );

            let mut subtask = Task::with_title(&title, &description);
            subtask.parent_id = Some(task_id);
            subtask.source = TaskSource::SubtaskOf(task_id);
            let _ = subtask.transition_to(TaskStatus::Ready);

            // Assign agent_type inline if the slice specifies one
            if let Some(ref agent) = slice.agent {
                subtask.agent_type = Some(agent.clone());
            }

            // Inherit worktree from parent
            subtask.worktree_path = task.worktree_path.clone();

            // Copy slice context into subtask
            for (k, v) in &slice.context {
                subtask.context.custom.insert(k.clone(), v.clone());
            }
            subtask.context.custom.insert(
                "workflow_phase".to_string(),
                serde_json::json!({
                    "workflow_name": workflow_name,
                    "phase_index": phase_index,
                    "phase_name": phase_name,
                    "slice_index": i,
                    "total_slices": slices.len(),
                }),
            );

            self.task_repo.create(&subtask).await?;
            subtask_ids.push(subtask.id);
        }

        let slice_count = slices.len();
        let new_state = WorkflowState::FanningOut {
            workflow_name: workflow_name.clone(),
            phase_index,
            phase_name: phase_name.clone(),
            subtask_ids: subtask_ids.clone(),
            slice_count,
        };
        self.write_state(task_id, &new_state).await?;

        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                None,
                Some(task_id),
                EventPayload::WorkflowPhaseStarted {
                    task_id,
                    phase_index,
                    phase_name: phase_name.clone(),
                    subtask_ids: subtask_ids.clone(),
                },
            ))
            .await;

        Ok(FanOutResult {
            subtask_ids,
            phase_index,
            phase_name,
        })
    }

    /// Get the current workflow state for a task.
    pub async fn get_state(&self, task_id: Uuid) -> DomainResult<WorkflowStatus> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let workflow_name = state.workflow_name().to_string();
        let total_phases = self
            .get_template(&workflow_name)
            .map(|t| t.phases.len())
            .unwrap_or(0);

        let (current_phase_index, current_phase_name, is_verifying, verification_retry_count) = match &state {
            WorkflowState::PhaseRunning {
                phase_index,
                phase_name,
                ..
            }
            | WorkflowState::FanningOut {
                phase_index,
                phase_name,
                ..
            }
            | WorkflowState::Aggregating {
                phase_index,
                phase_name,
                ..
            } => (Some(*phase_index), Some(phase_name.clone()), false, None),
            WorkflowState::Verifying {
                phase_index,
                phase_name,
                retry_count,
                ..
            } => (Some(*phase_index), Some(phase_name.clone()), true, Some(*retry_count)),
            WorkflowState::PhaseGate {
                phase_index,
                phase_name,
                ..
            }
            | WorkflowState::PhaseReady {
                phase_index,
                phase_name,
                ..
            } => (Some(*phase_index), Some(phase_name.clone()), false, None),
            _ => (None, None, false, None),
        };

        Ok(WorkflowStatus {
            task_id,
            workflow_name,
            state,
            total_phases,
            current_phase_index,
            current_phase_name,
            is_verifying,
            verification_retry_count,
        })
    }

    // ========================================================================
    // Internal helpers — task mutations via TaskService
    // ========================================================================

    /// Complete the parent workflow task via TaskService.
    ///
    /// Routes through `TaskService::complete_task()` so that
    /// `TaskCompleted` + `TaskExecutionRecorded` events are emitted
    /// and optimistic locking is handled correctly.
    async fn complete_parent_task(&self, task_id: Uuid) -> DomainResult<()> {
        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;
        if task.status.is_terminal() {
            return Ok(());
        }
        let (_task, events) = self.task_service.complete_task(task_id).await?;
        for evt in events {
            self.event_bus.publish(evt).await;
        }
        Ok(())
    }

    /// Fail the parent workflow task via TaskService.
    ///
    /// Routes through `TaskService::fail_task()` so that
    /// `TaskFailed` + `TaskExecutionRecorded` events are emitted.
    async fn fail_parent_task(&self, task_id: Uuid, error: &str) -> DomainResult<()> {
        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;
        if task.status.is_terminal() {
            return Ok(());
        }
        let (_task, events) = self.task_service
            .fail_task(task_id, Some(error.to_string()))
            .await?;
        for evt in events {
            self.event_bus.publish(evt).await;
        }
        Ok(())
    }

    /// Fail a workflow phase: write Failed workflow state, fail the parent task,
    /// and emit `WorkflowPhaseFailed`. Consolidates the duplicated failure pattern.
    async fn fail_workflow_phase(
        &self,
        parent_task_id: Uuid,
        workflow_name: &str,
        phase_index: usize,
        phase_name: &str,
        error_msg: &str,
    ) -> DomainResult<()> {
        let failed_state = WorkflowState::Failed {
            workflow_name: workflow_name.to_string(),
            error: error_msg.to_string(),
        };
        self.write_state(parent_task_id, &failed_state).await?;

        // Fail parent task via TaskService (emits TaskFailed + TaskExecutionRecorded)
        self.fail_parent_task(parent_task_id, error_msg).await?;

        // Emit workflow-specific event
        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Error,
                EventCategory::Task,
                None,
                Some(parent_task_id),
                EventPayload::WorkflowPhaseFailed {
                    task_id: parent_task_id,
                    phase_index,
                    phase_name: phase_name.to_string(),
                    reason: error_msg.to_string(),
                },
            ))
            .await;

        Ok(())
    }

    // ========================================================================
    // Internal helpers — reads and subtask queries
    // ========================================================================

    /// Store verification feedback in the parent task context.
    ///
    /// Appends to the `verification_feedback` array in the task's custom context
    /// so rework agents can see what failed. Uses TaskService for retry-on-conflict.
    async fn store_verification_feedback(
        &self,
        task_id: Uuid,
        summary: &str,
    ) -> DomainResult<()> {
        // Read current feedback array, append, then write back via TaskService.
        let task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let mut feedback = task
            .context
            .custom
            .get("verification_feedback")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        feedback.push(serde_json::json!(summary));

        self.task_service.update_task_context(
            task_id,
            vec![("verification_feedback".to_string(), serde_json::json!(feedback))],
        ).await
    }

    /// Handle fan-in: all fan-out subtasks are done, create aggregation task.
    ///
    /// Transitions from FanningOut → Aggregating and creates a read-only
    /// aggregation subtask that synthesizes results from parallel slices.
    async fn handle_fan_in(&self, parent_task_id: Uuid) -> DomainResult<()> {
        let task = self
            .task_repo
            .get(parent_task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(parent_task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", parent_task_id))
        })?;

        let (workflow_name, phase_index, phase_name, fan_out_subtask_ids) = match &state {
            WorkflowState::FanningOut {
                workflow_name,
                phase_index,
                phase_name,
                subtask_ids,
                ..
            } => (
                workflow_name.clone(),
                *phase_index,
                phase_name.clone(),
                subtask_ids.clone(),
            ),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Task {} is not in FanningOut state",
                    parent_task_id
                )));
            }
        };

        // Collect summaries from completed fan-out subtasks
        let mut slice_summaries = Vec::new();
        for (i, id) in fan_out_subtask_ids.iter().enumerate() {
            if let Ok(Some(subtask)) = self.task_repo.get(*id).await {
                let artifact_refs: Vec<String> = subtask
                    .artifacts
                    .iter()
                    .map(|a| a.uri.clone())
                    .collect();
                slice_summaries.push(format!(
                    "Slice {} (id: {}): {} (status: {}, artifacts: {})",
                    i + 1,
                    id,
                    subtask.title,
                    subtask.status.as_str(),
                    if artifact_refs.is_empty() {
                        "none".to_string()
                    } else {
                        artifact_refs.join(", ")
                    }
                ));
            }
        }

        let subtask_id_list: Vec<String> = fan_out_subtask_ids
            .iter()
            .map(|id| id.to_string())
            .collect();

        let aggregation_desc = format!(
            "Aggregate results from {} parallel slices of phase '{}'.\n\n\
             ## Subtask IDs\n{}\n\n\
             ## Slice Results\n{}\n\n\
             ## Your Role\n\
             1. Use `task_get` for each subtask ID above to read full results.\n\
             2. Use `memory_search` to find memories stored by these subtasks \
             (search for the subtask titles and phase name '{}').\n\
             3. Synthesize findings into a unified summary.\n\
             4. Decide whether the NEXT phase should fan out to multiple parallel \
             tasks or collapse to a single task. Base this on:\n\
                - Whether subtask results are independent or need integration\n\
                - Whether the work ahead is naturally parallelizable\n\
                - Whether conflicts/gaps require a single coordinating agent\n\
             5. Store your summary AND fan-out/collapse recommendation via `memory_store`.\n\n\
             Parent task: {}",
            fan_out_subtask_ids.len(),
            phase_name,
            subtask_id_list.join("\n"),
            slice_summaries.join("\n"),
            phase_name,
            task.description
        );

        let mut agg_subtask = Task::with_title(
            format!("[{}/{}:{}] Aggregate fan-out results", workflow_name, phase_index, phase_name),
            &aggregation_desc,
        );
        agg_subtask.parent_id = Some(parent_task_id);
        agg_subtask.source = TaskSource::SubtaskOf(parent_task_id);
        agg_subtask.task_type = TaskType::Standard;
        agg_subtask.execution_mode = ExecutionMode::Direct; // Aggregation is always read-only
        agg_subtask.worktree_path = task.worktree_path.clone();
        let _ = agg_subtask.transition_to(TaskStatus::Ready);
        agg_subtask.agent_type = Some("aggregator".to_string());

        agg_subtask.context.custom.insert(
            "workflow_phase".to_string(),
            serde_json::json!({
                "workflow_name": workflow_name,
                "phase_index": phase_index,
                "phase_name": phase_name,
                "is_aggregation": true,
            }),
        );

        self.task_repo.create(&agg_subtask).await?;

        // Include all original subtask_ids plus the aggregation subtask
        let mut all_ids = fan_out_subtask_ids;
        all_ids.push(agg_subtask.id);

        let aggregating_state = WorkflowState::Aggregating {
            workflow_name: workflow_name.clone(),
            phase_index,
            phase_name: phase_name.clone(),
            subtask_ids: all_ids,
        };
        self.write_state(parent_task_id, &aggregating_state).await?;

        tracing::info!(
            task_id = %parent_task_id,
            aggregation_subtask = %agg_subtask.id,
            "Fan-in: created aggregation subtask"
        );

        Ok(())
    }

    /// Check if all subtask IDs are in a terminal state.
    async fn all_subtasks_done(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) if t.status.is_terminal() => continue,
                Some(_) => return Ok(false),
                None => {
                    tracing::warn!(subtask_id = %id, "Workflow subtask missing — treating as terminal");
                    continue;
                }
            }
        }
        Ok(true)
    }

    /// Check if any subtask failed or was canceled.
    async fn any_subtask_failed(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) if t.status == TaskStatus::Failed || t.status == TaskStatus::Canceled => {
                    return Ok(true)
                }
                _ => continue,
            }
        }
        Ok(false)
    }

    /// Attempt to retry failed subtasks within a phase. Returns:
    /// - `Ok(true)` if any subtasks were retried (caller should return `Ok(())`)
    /// - `Ok(false)` if retries are exhausted or no subtask was retryable (caller should fail)
    async fn retry_failed_phase_subtasks(
        &self,
        parent_task_id: Uuid,
        phase_index: usize,
        phase_name: &str,
        subtask_ids: &[Uuid],
    ) -> DomainResult<bool> {
        const MAX_PHASE_RETRIES: u64 = 2;

        let phase_retry_key = format!("phase_{}_retry_count", phase_index);
        let parent = self
            .task_repo
            .get(parent_task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(parent_task_id))?;
        let phase_retry_count = parent
            .context
            .custom
            .get(&phase_retry_key)
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if phase_retry_count < MAX_PHASE_RETRIES {
            let mut retried_any = false;
            for &sid in subtask_ids {
                if let Some(sub) = self.task_repo.get(sid).await?
                    && sub.status == TaskStatus::Failed
                    && sub.can_retry()
                {
                    let (_task, events) = self.task_service.retry_task(sid).await?;
                    for evt in events {
                        self.event_bus.publish(evt).await;
                    }
                    retried_any = true;
                }
            }

            if retried_any {
                let new_retry_count = phase_retry_count + 1;
                self.task_service.update_task_context(
                    parent_task_id,
                    vec![(phase_retry_key, serde_json::Value::Number(new_retry_count.into()))],
                ).await?;
                tracing::info!(
                    parent_id = %parent_task_id,
                    phase = %phase_name,
                    retry = new_retry_count,
                    max = MAX_PHASE_RETRIES,
                    "Retrying failed phase subtasks"
                );
                self.event_bus
                    .publish(event_factory::make_event(
                        EventSeverity::Warning,
                        EventCategory::Task,
                        None,
                        Some(parent_task_id),
                        EventPayload::WorkflowPhaseRetried {
                            task_id: parent_task_id,
                            phase_index,
                            phase_name: phase_name.to_string(),
                            retry_count: new_retry_count,
                        },
                    ))
                    .await;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if all subtasks have convergence_outcome == "converged".
    ///
    /// Used to skip redundant workflow verification when convergent execution
    /// already verified each subtask.
    async fn all_subtasks_converged(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) => {
                    let converged = t
                        .context
                        .custom
                        .get("convergence_outcome")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "converged" || s == "partial_accepted")
                        .unwrap_or(false);
                    if !converged {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{
        create_migrated_test_pool, task_repository::SqliteTaskRepository,
    };
    use crate::services::event_bus::{EventBus, EventBusConfig};

    #[test]
    fn test_is_gate_phase() {
        assert!(is_gate_phase("triage"));
        assert!(is_gate_phase("review"));
        assert!(!is_gate_phase("implement"));
        assert!(!is_gate_phase("research"));
    }

    #[tokio::test]
    async fn test_handle_phase_complete_fails_parent_task() {
        // Setup
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

        let task_service = TaskService::new(task_repo.clone());
        let engine = WorkflowEngine::new(task_repo.clone(), task_service, event_bus.clone(), false);

        // Create a parent task enrolled in a workflow, in Running state
        let mut parent = Task::with_title("Parent workflow task", "Do work");
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();

        // Set workflow state to PhaseRunning with a subtask
        let mut subtask = Task::with_title("Phase subtask", "Subtask work");
        subtask.parent_id = Some(parent.id);
        subtask.source = TaskSource::SubtaskOf(parent.id);
        subtask.transition_to(TaskStatus::Ready).unwrap();
        subtask.transition_to(TaskStatus::Running).unwrap();
        subtask.transition_to(TaskStatus::Failed).unwrap();
        subtask.max_retries = 0; // No retries allowed

        task_repo.create(&parent).await.unwrap();
        task_repo.create(&subtask).await.unwrap();

        // Write PhaseRunning workflow state on parent
        let ws = WorkflowState::PhaseRunning {
            workflow_name: "code".to_string(),
            phase_index: 0,
            phase_name: "implement".to_string(),
            subtask_ids: vec![subtask.id],
        };
        engine.write_state(parent.id, &ws).await.unwrap();

        // Now call handle_phase_complete — the subtask has failed and has no retries
        let result = engine.handle_phase_complete(parent.id, subtask.id).await;
        assert!(result.is_ok(), "handle_phase_complete should succeed: {:?}", result.err());

        // Verify parent task is now Failed
        let updated_parent = task_repo.get(parent.id).await.unwrap().unwrap();
        assert_eq!(
            updated_parent.status,
            TaskStatus::Failed,
            "Parent task should be Failed after phase failure"
        );

        // Verify workflow state is Failed
        let ws_val = updated_parent.context.custom.get("workflow_state").unwrap();
        let ws: WorkflowState = serde_json::from_value(ws_val.clone()).unwrap();
        assert!(
            matches!(ws, WorkflowState::Failed { .. }),
            "workflow_state should be Failed"
        );
    }

    #[tokio::test]
    async fn test_validating_to_canceled_transition_allowed() {
        // This tests Fix 1: Validating → Canceled is a valid state transition
        let mut task = Task::new("Test validating cancel");
        task.transition_to(TaskStatus::Ready).unwrap();
        task.transition_to(TaskStatus::Running).unwrap();
        task.transition_to(TaskStatus::Validating).unwrap();

        // Validating → Canceled should succeed
        assert!(
            task.can_transition_to(TaskStatus::Canceled),
            "Validating → Canceled should be a valid transition"
        );
        task.transition_to(TaskStatus::Canceled).unwrap();
        assert_eq!(task.status, TaskStatus::Canceled);
        assert!(task.is_terminal());
        assert!(task.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_workflow_phase_retried_event_emitted_on_retry() {
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

        let task_service = TaskService::new(task_repo.clone());
        let engine = WorkflowEngine::new(task_repo.clone(), task_service, event_bus.clone(), false);

        // Create parent task in Running state
        let mut parent = Task::with_title("Parent workflow task", "Do work");
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();

        // Create a failed subtask that CAN be retried
        let mut subtask = Task::with_title("Phase subtask", "Subtask work");
        subtask.parent_id = Some(parent.id);
        subtask.source = TaskSource::SubtaskOf(parent.id);
        subtask.max_retries = 3;
        subtask.transition_to(TaskStatus::Ready).unwrap();
        subtask.transition_to(TaskStatus::Running).unwrap();
        subtask.transition_to(TaskStatus::Failed).unwrap();

        task_repo.create(&parent).await.unwrap();
        task_repo.create(&subtask).await.unwrap();

        // Write PhaseRunning workflow state on parent
        let ws = WorkflowState::PhaseRunning {
            workflow_name: "code".to_string(),
            phase_index: 1,
            phase_name: "implement".to_string(),
            subtask_ids: vec![subtask.id],
        };
        engine.write_state(parent.id, &ws).await.unwrap();

        // Subscribe to events before the action
        let mut rx = event_bus.subscribe();

        // Call handle_phase_complete — subtask failed but can retry
        let result = engine.handle_phase_complete(parent.id, subtask.id).await;
        assert!(result.is_ok(), "handle_phase_complete should succeed: {:?}", result.err());

        // Collect emitted events
        let mut found_retried = false;
        while let Ok(event) = rx.try_recv() {
            if let EventPayload::WorkflowPhaseRetried {
                task_id,
                phase_index,
                phase_name,
                retry_count,
            } = &event.payload
            {
                assert_eq!(*task_id, parent.id);
                assert_eq!(*phase_index, 1);
                assert_eq!(phase_name, "implement");
                assert_eq!(*retry_count, 1);
                found_retried = true;
            }
        }
        assert!(found_retried, "WorkflowPhaseRetried event should have been emitted");
    }

    #[tokio::test]
    async fn test_workflow_phase_failed_event_emitted_when_retries_exhausted() {
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));

        let task_service = TaskService::new(task_repo.clone());
        let engine = WorkflowEngine::new(task_repo.clone(), task_service, event_bus.clone(), false);

        // Create parent task in Running state
        let mut parent = Task::with_title("Parent workflow task", "Do work");
        parent.transition_to(TaskStatus::Ready).unwrap();
        parent.transition_to(TaskStatus::Running).unwrap();

        // Create a failed subtask with NO retries
        let mut subtask = Task::with_title("Phase subtask", "Subtask work");
        subtask.parent_id = Some(parent.id);
        subtask.source = TaskSource::SubtaskOf(parent.id);
        subtask.max_retries = 0;
        subtask.transition_to(TaskStatus::Ready).unwrap();
        subtask.transition_to(TaskStatus::Running).unwrap();
        subtask.transition_to(TaskStatus::Failed).unwrap();

        task_repo.create(&parent).await.unwrap();
        task_repo.create(&subtask).await.unwrap();

        // Write PhaseRunning workflow state on parent
        let ws = WorkflowState::PhaseRunning {
            workflow_name: "code".to_string(),
            phase_index: 0,
            phase_name: "implement".to_string(),
            subtask_ids: vec![subtask.id],
        };
        engine.write_state(parent.id, &ws).await.unwrap();

        // Subscribe to events before the action
        let mut rx = event_bus.subscribe();

        // Call handle_phase_complete — subtask failed with no retries
        let result = engine.handle_phase_complete(parent.id, subtask.id).await;
        assert!(result.is_ok(), "handle_phase_complete should succeed: {:?}", result.err());

        // Collect emitted events
        let mut found_failed = false;
        while let Ok(event) = rx.try_recv() {
            if let EventPayload::WorkflowPhaseFailed {
                task_id,
                phase_index,
                phase_name,
                reason,
            } = &event.payload
            {
                assert_eq!(*task_id, parent.id);
                assert_eq!(*phase_index, 0);
                assert_eq!(phase_name, "implement");
                assert!(reason.contains("failed after"), "reason should mention retries: {}", reason);
                found_failed = true;
            }
        }
        assert!(found_failed, "WorkflowPhaseFailed event should have been emitted");
    }
}

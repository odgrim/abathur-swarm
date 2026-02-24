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

/// Whether a phase name is a gate phase.
///
/// Gate phases park at `PhaseGate` and require an overmind verdict.
fn is_gate_phase(phase_name: &str) -> bool {
    matches!(phase_name, "triage" | "review")
}

/// Result of an `advance` call, giving the overmind enough info to create an agent.
#[derive(Debug, Clone)]
pub enum AdvanceResult {
    /// A new phase subtask was created; the overmind should create an agent for it.
    PhaseStarted {
        subtask_id: Uuid,
        phase_index: usize,
        phase_name: String,
        subtask_title: String,
        subtask_description: String,
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
    event_bus: Arc<EventBus>,
    templates: std::collections::HashMap<String, WorkflowTemplate>,
    verification_enabled: bool,
}

impl<T: TaskRepository + 'static> WorkflowEngine<T> {
    pub fn new(task_repo: Arc<T>, event_bus: Arc<EventBus>, verification_enabled: bool) -> Self {
        Self {
            task_repo,
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

    /// Write workflow state to task context and persist.
    async fn write_state(&self, task_id: Uuid, state: &WorkflowState) -> DomainResult<()> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;
        let value = serde_json::to_value(state)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        task.context.custom.insert("workflow_state".to_string(), value);
        task.updated_at = chrono::Utc::now();
        self.task_repo.update(&task).await?;
        Ok(())
    }

    // ========================================================================
    // Public API
    // ========================================================================

    // enroll() — REMOVED
    // Auto-enrollment happens in TaskService::submit_task() with template
    // validation (fix C). The MCP enroll tool was also removed. Tests use
    // submit_task() or write workflow state directly.

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

            // Complete the parent task
            {
                let mut parent = self.task_repo.get(task_id).await?
                    .ok_or(DomainError::TaskNotFound(task_id))?;
                if !parent.status.is_terminal() {
                    let _ = parent.transition_to(TaskStatus::Complete);
                    self.task_repo.update(&parent).await?;
                }
            }

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

        // Create subtask for this phase
        let subtask = self.create_phase_subtask(task_id, &workflow_name, next_index, phase, &task).await?;
        let subtask_id = subtask.id;

        let new_state = WorkflowState::PhaseRunning {
            workflow_name: workflow_name.clone(),
            phase_index: next_index,
            phase_name: phase.name.clone(),
            subtask_ids: vec![subtask_id],
        };
        self.write_state(task_id, &new_state).await?;

        // Emit events
        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                None,
                Some(task_id),
                EventPayload::WorkflowPhaseStarted {
                    task_id,
                    phase_index: next_index,
                    phase_name: phase.name.clone(),
                    subtask_ids: vec![subtask_id],
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

        Ok(AdvanceResult::PhaseStarted {
            subtask_id,
            phase_index: next_index,
            phase_name: phase.name.clone(),
            subtask_title: subtask.title,
            subtask_description: subtask.description,
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
                phase_index: _,
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
                    let failed_state = WorkflowState::Failed {
                        workflow_name: workflow_name.clone(),
                        error: format!("Phase '{}' fan-out subtask failed", phase_name),
                    };
                    self.write_state(parent_task_id, &failed_state).await?;
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
            tracing::debug!(
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
            const MAX_PHASE_RETRIES: u64 = 2;

            // Track phase retry count on the parent task's custom context
            let phase_retry_key = format!("phase_{}_retry_count", phase_index);
            let mut parent = self.task_repo.get(parent_task_id).await?
                .ok_or(DomainError::TaskNotFound(parent_task_id))?;
            let phase_retry_count = parent.context.custom
                .get(&phase_retry_key)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if phase_retry_count < MAX_PHASE_RETRIES {
                // Retry failed subtasks at the phase level
                let mut retried_any = false;
                for &sid in &subtask_ids {
                    if let Some(mut sub) = self.task_repo.get(sid).await? {
                        if sub.status == TaskStatus::Failed && sub.can_retry() {
                            if sub.retry().is_ok() {
                                self.task_repo.update(&sub).await?;
                                self.event_bus
                                    .publish(event_factory::make_event(
                                        EventSeverity::Info,
                                        EventCategory::Task,
                                        None,
                                        Some(sid),
                                        EventPayload::TaskReady {
                                            task_id: sid,
                                            task_title: sub.title.clone(),
                                        },
                                    ))
                                    .await;
                                retried_any = true;
                            }
                        }
                    }
                }

                if retried_any {
                    // Increment phase retry count and stay in PhaseRunning
                    parent.context.custom.insert(
                        phase_retry_key,
                        serde_json::Value::Number((phase_retry_count + 1).into()),
                    );
                    self.task_repo.update(&parent).await?;
                    tracing::info!(
                        parent_id = %parent_task_id,
                        phase = %phase_name,
                        retry = phase_retry_count + 1,
                        max = MAX_PHASE_RETRIES,
                        "Retrying failed phase subtasks"
                    );
                    return Ok(());
                }
            }

            // Exhausted phase retries (or no subtask was retryable) → fail the workflow
            let failed_state = WorkflowState::Failed {
                workflow_name,
                error: format!(
                    "Phase '{}' subtask failed after {} retries",
                    phase_name, phase_retry_count
                ),
            };
            self.write_state(parent_task_id, &failed_state).await?;
            return Ok(());
        }

        // Check if this phase requires verification
        let template = self.get_template(&workflow_name)?;
        let phase = &template.phases[phase_index];

        // Skip verification if all subtasks already converged (Step 5.2)
        let all_converged = self.all_subtasks_converged(&subtask_ids).await?;

        if phase.verify && self.verification_enabled && !all_converged {
            // Transition parent TaskStatus to Validating (Step 4.2)
            {
                let mut parent = self.task_repo.get(parent_task_id).await?
                    .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                if parent.status == TaskStatus::Running {
                    let _ = parent.transition_to(TaskStatus::Validating);
                    self.task_repo.update(&parent).await?;
                    self.event_bus
                        .publish(event_factory::make_event(
                            EventSeverity::Info,
                            EventCategory::Task,
                            None,
                            Some(parent_task_id),
                            EventPayload::TaskValidating { task_id: parent_task_id },
                        ))
                        .await;
                }
            }

            // Transition to Verifying state
            let verifying_state = WorkflowState::Verifying {
                workflow_name: workflow_name.clone(),
                phase_index,
                phase_name: phase_name.clone(),
                subtask_ids: subtask_ids.clone(),
                retry_count: 0,
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
                        retry_count: 0,
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

            // Complete the parent task
            {
                let mut parent = self.task_repo.get(parent_task_id).await?
                    .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                if !parent.status.is_terminal() {
                    let _ = parent.transition_to(TaskStatus::Complete);
                    self.task_repo.update(&parent).await?;
                }
            }

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
            // Auto-advance to the next phase
            match self.advance(parent_task_id).await {
                Ok(AdvanceResult::PhaseStarted { phase_name, .. }) => {
                    tracing::info!(
                        task_id = %parent_task_id,
                        phase = %phase_name,
                        "Workflow auto-advanced to next phase"
                    );
                }
                Ok(AdvanceResult::Completed) => {
                    tracing::info!(
                        task_id = %parent_task_id,
                        "Workflow completed during auto-advance"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %parent_task_id,
                        error = %e,
                        "Workflow auto-advance failed"
                    );
                    return Err(e);
                }
            }
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
            // Transition parent TaskStatus: Validating -> Running (to continue)
            {
                let mut parent = self.task_repo.get(parent_task_id).await?
                    .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                if parent.status == TaskStatus::Validating {
                    let _ = parent.transition_to(TaskStatus::Running);
                    self.task_repo.update(&parent).await?;
                }
            }

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
                // Auto-advance to next phase (Step 6.1)
                let next_index = phase_index + 1;
                if next_index >= template.phases.len() {
                    let completed = WorkflowState::Completed {
                        workflow_name: workflow_name.clone(),
                    };
                    self.write_state(parent_task_id, &completed).await?;

                    // Complete the parent task
                    {
                        let mut parent = self.task_repo.get(parent_task_id).await?
                            .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                        if !parent.status.is_terminal() {
                            let _ = parent.transition_to(TaskStatus::Complete);
                            self.task_repo.update(&parent).await?;
                        }
                    }

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
                    // Auto-advance to next phase
                    match self.advance(parent_task_id).await {
                        Ok(AdvanceResult::PhaseStarted { phase_name: next_phase, .. }) => {
                            tracing::info!(
                                task_id = %parent_task_id,
                                phase = %next_phase,
                                "Workflow auto-advanced after verification"
                            );
                        }
                        Ok(AdvanceResult::Completed) => {
                            tracing::info!(
                                task_id = %parent_task_id,
                                "Workflow completed after verification"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                task_id = %parent_task_id,
                                error = %e,
                                "Workflow auto-advance after verification failed"
                            );
                            return Err(e);
                        }
                    }
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

            // Auto-advance to re-create the phase with feedback
            match self.advance(parent_task_id).await {
                Ok(AdvanceResult::PhaseStarted { phase_name: rework_phase, .. }) => {
                    tracing::info!(
                        task_id = %parent_task_id,
                        phase = %rework_phase,
                        retry = retry_count + 1,
                        "Workflow auto-rework: re-created phase subtask with verification feedback"
                    );

                    // Update retry count on the parent task regardless of workflow state
                    let mut current_task = self.task_repo.get(parent_task_id).await?
                        .ok_or(DomainError::TaskNotFound(parent_task_id))?;
                    current_task.context.custom.insert(
                        "verification_retry_count".to_string(),
                        serde_json::json!(retry_count + 1),
                    );
                    current_task.updated_at = chrono::Utc::now();
                    self.task_repo.update(&current_task).await?;
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
                let rejected = WorkflowState::Rejected {
                    workflow_name,
                    phase_index,
                    reason: reason.to_string(),
                };
                self.write_state(task_id, &rejected).await?;
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

        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let (workflow_name, phase_index, phase_name) = match &state {
            // Allow fan_out from Pending (replaces the first advance) or from PhaseGate
            WorkflowState::Pending { workflow_name } => {
                let template = self.get_template(workflow_name)?;
                let phase = &template.phases[0];
                (workflow_name.clone(), 0, phase.name.clone())
            }
            WorkflowState::PhaseGate {
                workflow_name,
                phase_index,
                ..
            } => {
                let template = self.get_template(workflow_name)?;
                let next_idx = phase_index + 1;
                if next_idx >= template.phases.len() {
                    return Err(DomainError::ValidationFailed(
                        "No more phases to fan out".to_string(),
                    ));
                }
                let phase = &template.phases[next_idx];
                (workflow_name.clone(), next_idx, phase.name.clone())
            }
            WorkflowState::PhaseRunning {
                workflow_name,
                phase_index,
                phase_name,
                ..
            } => (workflow_name.clone(), *phase_index, phase_name.clone()),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Cannot fan_out task {} from state {:?}",
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
    // Internal helpers
    // ========================================================================

    /// Store verification feedback in the parent task context.
    ///
    /// Appends to the `verification_feedback` array in the task's custom context
    /// so rework agents can see what failed.
    async fn store_verification_feedback(
        &self,
        task_id: Uuid,
        summary: &str,
    ) -> DomainResult<()> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let feedback_array = task
            .context
            .custom
            .entry("verification_feedback".to_string())
            .or_insert_with(|| serde_json::json!([]));

        if let Some(arr) = feedback_array.as_array_mut() {
            arr.push(serde_json::json!(summary));
        }

        task.updated_at = chrono::Utc::now();
        self.task_repo.update(&task).await?;
        Ok(())
    }

    /// Create a subtask for a workflow phase.
    async fn create_phase_subtask(
        &self,
        parent_task_id: Uuid,
        workflow_name: &str,
        phase_index: usize,
        phase: &crate::domain::models::workflow_template::WorkflowPhase,
        parent_task: &Task,
    ) -> DomainResult<Task> {
        let template = self.get_template(workflow_name)?;
        let title = format!(
            "[{}/{}:{}] {}",
            workflow_name,
            phase_index,
            phase.name,
            phase.description.chars().take(60).collect::<String>()
        );

        // Build description, including verification feedback if present (rework scenario)
        let mut description = format!(
            "Workflow: {}\nPhase: {} ({}/{})\nPhase role: {}\nPhase description: {}\n\nParent task: {}",
            workflow_name,
            phase.name,
            phase_index + 1,
            template.phases.len(),
            phase.role,
            phase.description,
            parent_task.description
        );

        // Append verification feedback for rework subtasks
        if let Some(feedback) = parent_task.context.custom.get("verification_feedback") {
            if let Some(arr) = feedback.as_array() {
                if !arr.is_empty() {
                    description.push_str("\n\n--- VERIFICATION FEEDBACK (from previous attempt) ---\n");
                    for (i, item) in arr.iter().enumerate() {
                        if let Some(s) = item.as_str() {
                            description.push_str(&format!("Attempt {}: {}\n", i + 1, s));
                        }
                    }
                    description.push_str("--- Please address the above feedback in this attempt. ---\n");
                }
            }
        }

        let mut subtask = Task::with_title(&title, &description);
        subtask.parent_id = Some(parent_task_id);
        subtask.source = TaskSource::SubtaskOf(parent_task_id);
        let _ = subtask.transition_to(TaskStatus::Ready);

        // Phase-driven execution mode (Step 2.1):
        // read_only phases -> Direct; phases with write/edit/shell tools -> Convergent
        let has_write_tools = phase.tools.iter().any(|t| {
            let lower = t.to_lowercase();
            lower == "write" || lower == "edit" || lower == "shell"
        });
        if phase.read_only || !has_write_tools {
            subtask.execution_mode = ExecutionMode::Direct;
        } else {
            subtask.execution_mode = ExecutionMode::Convergent { parallel_samples: None };
        }

        // Inherit worktree from parent task
        subtask.worktree_path = parent_task.worktree_path.clone();

        // Tag subtask with workflow metadata
        subtask.context.custom.insert(
            "workflow_phase".to_string(),
            serde_json::json!({
                "workflow_name": workflow_name,
                "phase_index": phase_index,
                "phase_name": phase.name,
            }),
        );

        self.task_repo.create(&subtask).await?;
        Ok(subtask)
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
                    "Slice {}: {} (status: {}, artifacts: {})",
                    i + 1,
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

        let aggregation_desc = format!(
            "Aggregate results from {} parallel slices of phase '{}'.\n\n\
             ## Slice Results\n{}\n\n\
             ## Your Role\n\
             Synthesize the results from all slices into a coherent phase output. \
             Identify any conflicts or gaps between slices and produce a unified summary.\n\n\
             Parent task: {}",
            fan_out_subtask_ids.len(),
            phase_name,
            slice_summaries.join("\n"),
            task.description
        );

        let mut agg_subtask = Task::with_title(
            &format!("[{}/{}:{}] Aggregate fan-out results", workflow_name, phase_index, phase_name),
            &aggregation_desc,
        );
        agg_subtask.parent_id = Some(parent_task_id);
        agg_subtask.source = TaskSource::SubtaskOf(parent_task_id);
        agg_subtask.task_type = TaskType::Standard;
        agg_subtask.execution_mode = ExecutionMode::Direct; // Aggregation is always read-only
        agg_subtask.worktree_path = task.worktree_path.clone();
        let _ = agg_subtask.transition_to(TaskStatus::Ready);

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

    /// Check if any subtask failed.
    async fn any_subtask_failed(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) if t.status == TaskStatus::Failed => return Ok(true),
                _ => continue,
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

    // Workflow engine tests require a real TaskRepository (SQLite) due to
    // the persist-on-every-transition design. Integration tests should
    // be added in the integration test suite.

    #[test]
    fn test_is_gate_phase() {
        assert!(is_gate_phase("triage"));
        assert!(is_gate_phase("review"));
        assert!(!is_gate_phase("implement"));
        assert!(!is_gate_phase("research"));
    }
}

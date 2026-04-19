//! Phase transitions and the workflow state machine.
//!
//! Handles the main transitions `advance`, `handle_phase_complete`,
//! `handle_verification_result`, and `provide_verdict`, plus the internal
//! helpers that mutate the parent task (complete/fail/feedback) and the
//! phase-level retry logic.

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::task::TaskStatus;
use crate::domain::models::workflow_state::{GateVerdict, WorkflowState};
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::{
    EventCategory, EventPayload, EventSeverity, WorkflowVerificationCompletedPayload,
};
use crate::services::event_factory;

use super::validators::is_gate_phase;
use super::{AdvanceResult, WorkflowEngine};

impl<T: TaskRepository + 'static> WorkflowEngine<T> {
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
                // Auto-correct: if TaskStatus is Validating while WorkflowState is
                // PhaseReady, the task got stuck in an inconsistent state (validation
                // deadlock). Correct TaskStatus to Running so the workflow can proceed.
                if task.status == TaskStatus::Validating {
                    tracing::warn!(
                        task_id = %task_id,
                        "State inconsistency detected: TaskStatus::Validating with WorkflowState::PhaseReady — \
                         auto-correcting TaskStatus to Running"
                    );
                    if let Err(e) = self.task_service.transition_to_running(task_id).await {
                        tracing::warn!(task_id = %task_id, error = %e, "auto-correct: transition_to_running failed during stale-Validating recovery");
                    }
                }
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
                        "Phase '{}' fan-out subtask failed after {} retries",
                        phase_name, phase_retry_count,
                    );
                    self.fail_workflow_phase(
                        parent_task_id,
                        workflow_name,
                        *phase_index,
                        phase_name,
                        &error_msg,
                    )
                    .await?;
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
                .retry_failed_phase_subtasks(parent_task_id, phase_index, &phase_name, &subtask_ids)
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
            )
            .await?;
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
                let (_task, events) = self
                    .task_service
                    .transition_to_validating(parent_task_id)
                    .await?;
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
        if is_gate_phase(&self.templates, &workflow_name, phase_index, &phase_name) {
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
                EventPayload::WorkflowVerificationCompleted(WorkflowVerificationCompletedPayload {
                    task_id: parent_task_id,
                    phase_index,
                    phase_name: phase_name.clone(),
                    satisfied,
                    retry_count,
                    summary: summary.to_string(),
                }),
            ))
            .await;

        if satisfied {
            // Transition parent TaskStatus: Validating -> Running via TaskService
            self.task_service
                .transition_to_running(parent_task_id)
                .await?;

            // Verification passed — proceed
            if is_gate_phase(&self.templates, &workflow_name, phase_index, &phase_name) {
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
            self.store_verification_feedback(parent_task_id, summary)
                .await?;

            // Transition parent TaskStatus: Validating -> Running via TaskService
            // (mirrors the satisfied branch above)
            self.task_service
                .transition_to_running(parent_task_id)
                .await?;

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
                Ok(AdvanceResult::PhaseReady {
                    phase_name: rework_phase,
                    ..
                }) => {
                    tracing::info!(
                        task_id = %parent_task_id,
                        phase = %rework_phase,
                        retry = retry_count + 1,
                        "Workflow auto-rework: phase ready for rework with verification feedback"
                    );

                    // Update retry count on the parent task via TaskService
                    self.task_service
                        .update_task_context(
                            parent_task_id,
                            vec![(
                                "verification_retry_count".to_string(),
                                serde_json::json!(retry_count + 1),
                            )],
                        )
                        .await?;
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
            self.store_verification_feedback(parent_task_id, summary)
                .await?;

            // Transition parent TaskStatus: Validating -> Running via TaskService
            // (mirrors the satisfied branch above)
            self.task_service
                .transition_to_running(parent_task_id)
                .await?;

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

        let (workflow_name, phase_index, phase_name) = match &state {
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

                // Emit a dedicated rejection event so downstream handlers
                // (e.g. adapter lifecycle sync) can react specifically to
                // gate rejections rather than generic task failures.
                self.event_bus
                    .publish(event_factory::make_event(
                        EventSeverity::Warning,
                        EventCategory::Workflow,
                        None,
                        Some(task_id),
                        EventPayload::WorkflowGateRejected {
                            task_id,
                            phase_index,
                            phase_name: phase_name.clone(),
                            reason: rejection_reason.clone(),
                        },
                    ))
                    .await;

                // Fail parent task via TaskService
                let error_msg = format!(
                    "Workflow rejected at phase {}: {}",
                    phase_index, rejection_reason
                );
                self.fail_parent_task(task_id, &error_msg).await?;

                Ok(None)
            }
            GateVerdict::Rework => {
                // Re-run: go back to Pending-like state so advance() re-creates the phase subtask
                // We set it to PhaseGate at (phase_index - 1) so advance() will create phase_index again
                if phase_index == 0 {
                    let pending = WorkflowState::Pending { workflow_name };
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

    // ========================================================================
    // Internal helpers — task mutations via TaskService
    // ========================================================================

    /// Complete the parent workflow task via TaskService.
    ///
    /// Routes through `TaskService::complete_task()` so that
    /// `TaskCompleted` + `TaskExecutionRecorded` events are emitted
    /// and optimistic locking is handled correctly.
    pub(super) async fn complete_parent_task(&self, task_id: Uuid) -> DomainResult<()> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
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
    pub(super) async fn fail_parent_task(&self, task_id: Uuid, error: &str) -> DomainResult<()> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;
        if task.status.is_terminal() {
            return Ok(());
        }
        let (_task, events) = self
            .task_service
            .fail_task(task_id, Some(error.to_string()))
            .await?;
        for evt in events {
            self.event_bus.publish(evt).await;
        }
        Ok(())
    }

    /// Fail a workflow phase: write Failed workflow state, fail the parent task,
    /// and emit `WorkflowPhaseFailed`. Consolidates the duplicated failure pattern.
    pub(super) async fn fail_workflow_phase(
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
                EventCategory::Workflow,
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

    /// Store verification feedback in the parent task context.
    ///
    /// Appends to the `verification_feedback` array in the task's custom context
    /// so rework agents can see what failed. Uses TaskService for retry-on-conflict.
    pub(super) async fn store_verification_feedback(
        &self,
        task_id: Uuid,
        summary: &str,
    ) -> DomainResult<()> {
        // Read current feedback array, append, then write back via TaskService.
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let mut feedback = task
            .context
            .custom
            .get("verification_feedback")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        feedback.push(serde_json::json!(summary));

        self.task_service
            .update_task_context(
                task_id,
                vec![(
                    "verification_feedback".to_string(),
                    serde_json::json!(feedback),
                )],
            )
            .await
    }

    /// Attempt to retry failed subtasks within a phase. Returns:
    /// - `Ok(true)` if any subtasks were retried (caller should return `Ok(())`)
    /// - `Ok(false)` if retries are exhausted or no subtask was retryable (caller should fail)
    pub(super) async fn retry_failed_phase_subtasks(
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
                self.task_service
                    .update_task_context(
                        parent_task_id,
                        vec![(
                            phase_retry_key,
                            serde_json::Value::Number(new_retry_count.into()),
                        )],
                    )
                    .await?;
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
                        EventCategory::Workflow,
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
}

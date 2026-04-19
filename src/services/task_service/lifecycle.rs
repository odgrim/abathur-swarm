//! Task lifecycle transitions: claim, complete, fail, retry, cancel,
//! transition_to_*, force_transition, and workflow/context state mutators.

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{Task, TaskStatus};
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity, UnifiedEvent};

use super::TaskService;

impl<T: TaskRepository> TaskService<T> {
    /// Transition task to Running state (claim it).
    pub async fn claim_task(
        &self,
        task_id: Uuid,
        agent_type: &str,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        tracing::info!(%task_id, agent_type, "claiming task");
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.status != TaskStatus::Ready {
            tracing::warn!(%task_id, current_status = ?task.status, agent_type, "claim rejected: task not in Ready state");
            return Err(DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "running".to_string(),
                reason: "task must be in Ready state to be claimed".to_string(),
            });
        }

        task.agent_type = Some(agent_type.to_string());
        task.transition_to(TaskStatus::Running).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "running".to_string(),
                reason: e,
            }
        })?;

        self.task_repo.update(&task).await?;
        tracing::info!(%task_id, agent_type, "task claimed successfully");

        let goal_id = Self::extract_goal_id(&task);
        let events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskClaimed {
                task_id,
                agent_type: agent_type.to_string(),
            },
        )];

        self.publish_events(&events).await;
        Ok((task, events))
    }

    /// Mark task as complete.
    ///
    /// In addition to the standard TaskCompleted event, emits a
    /// `TaskExecutionRecorded` event for opportunistic convergence memory
    /// recording (spec Part 10.3). This lightweight event captures the task's
    /// complexity, execution mode, and success/failure signal. An event handler
    /// downstream persists this data to build the dataset used by the
    /// classification heuristic to learn which complexity levels benefit from
    /// convergence.
    pub async fn complete_task(&self, task_id: Uuid) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        tracing::info!(%task_id, "completing task");
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Complete).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "complete".to_string(),
                reason: e,
            }
        })?;

        self.task_repo.update(&task).await?;
        tracing::info!(%task_id, execution_mode = ?task.execution_mode, "task completed successfully");

        // Metrics: terminal completion. Labels are cardinality-bounded
        // (template = task_type variant, outcome ∈ {succeeded}).
        let template = task.task_type.as_str();
        metrics::counter!(
            "abathur_tasks_completed_total",
            "template" => template,
            "outcome" => "succeeded"
        )
        .increment(1);
        if let Some(started) = task.started_at {
            let secs = (chrono::Utc::now() - started).num_milliseconds().max(0) as f64 / 1000.0;
            metrics::histogram!(
                "abathur_task_duration_seconds",
                "template" => template,
                "outcome" => "succeeded"
            )
            .record(secs);
        }

        let goal_id = Self::extract_goal_id(&task);
        let mut events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskCompleted {
                task_id,
                tokens_used: 0,
            },
        )];

        // Opportunistic convergence memory recording (Part 10.3).
        // Emit a lightweight event so that a downstream handler can persist
        // execution metrics. This builds the dataset that informs the
        // classification heuristic over time.
        let execution_mode_str = if task.execution_mode.is_convergent() {
            "convergent".to_string()
        } else {
            "direct".to_string()
        };
        let complexity_str = format!("{:?}", task.routing_hints.complexity).to_lowercase();

        events.push(Self::make_event(
            EventSeverity::Debug,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskExecutionRecorded {
                task_id,
                execution_mode: execution_mode_str,
                complexity: complexity_str,
                succeeded: true,
                tokens_used: 0, // Actual token count filled by orchestrator-level event
            },
        ));

        self.publish_events(&events).await;
        Ok((task, events))
    }

    /// Mark task as failed.
    ///
    /// Also emits a `TaskExecutionRecorded` event for opportunistic convergence
    /// memory recording, mirroring the event emitted on success (Part 10.3).
    pub async fn fail_task(
        &self,
        task_id: Uuid,
        error_message: Option<String>,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        tracing::warn!(%task_id, error = ?error_message, "failing task");
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Failed).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "failed".to_string(),
                reason: e,
            }
        })?;

        let error_str = error_message.clone().unwrap_or_default();
        if let Some(msg) = error_message {
            task.context.push_hint_bounded(format!("Error: {}", msg));
        }

        self.task_repo.update(&task).await?;
        tracing::warn!(%task_id, retry_count = task.retry_count, execution_mode = ?task.execution_mode, "task marked as failed");

        // Metrics: terminal failure.
        let template_lbl = task.task_type.as_str();
        metrics::counter!(
            "abathur_tasks_completed_total",
            "template" => template_lbl,
            "outcome" => "failed"
        )
        .increment(1);
        if let Some(started) = task.started_at {
            let secs = (chrono::Utc::now() - started).num_milliseconds().max(0) as f64 / 1000.0;
            metrics::histogram!(
                "abathur_task_duration_seconds",
                "template" => template_lbl,
                "outcome" => "failed"
            )
            .record(secs);
        }

        let execution_mode_str = if task.execution_mode.is_convergent() {
            "convergent".to_string()
        } else {
            "direct".to_string()
        };
        let complexity_str = format!("{:?}", task.routing_hints.complexity).to_lowercase();

        let goal_id = Self::extract_goal_id(&task);
        let events = vec![
            Self::make_event(
                EventSeverity::Error,
                EventCategory::Task,
                goal_id,
                Some(task_id),
                EventPayload::TaskFailed {
                    task_id,
                    error: error_str,
                    retry_count: task.retry_count,
                },
            ),
            // Opportunistic convergence memory recording (Part 10.3).
            Self::make_event(
                EventSeverity::Debug,
                EventCategory::Task,
                goal_id,
                Some(task_id),
                EventPayload::TaskExecutionRecorded {
                    task_id,
                    execution_mode: execution_mode_str,
                    complexity: complexity_str,
                    succeeded: false,
                    tokens_used: 0,
                },
            ),
        ];

        self.publish_events(&events).await;
        Ok((task, events))
    }

    /// Transition a task to Ready status.
    ///
    /// Used by SYSTEM handlers (e.g. `TaskCompletedReadinessHandler`) to cascade
    /// readiness through the task DAG. This method follows the same pattern as
    /// `complete_task()`: fetch → validate transition → persist → emit event.
    ///
    /// If the task is already in `Ready` status, the transition is treated as
    /// idempotent and returns `Ok` with an empty events vec.
    pub async fn transition_to_ready(
        &self,
        task_id: Uuid,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        // Idempotent: already ready
        if task.status == TaskStatus::Ready {
            return Ok((task, Vec::new()));
        }

        task.transition_to(TaskStatus::Ready)
            .map_err(|e| DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "ready".to_string(),
                reason: e,
            })?;

        self.task_repo.update(&task).await?;
        tracing::debug!(%task_id, "task transitioned to ready");

        let goal_id = Self::extract_goal_id(&task);
        let events = vec![Self::make_event(
            EventSeverity::Debug,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskReady {
                task_id,
                task_title: task.title.clone(),
            },
        )];

        self.publish_events(&events).await;
        Ok((task, events))
    }

    /// Transition a task to Blocked status.
    ///
    /// Used by SYSTEM handlers (e.g. `TaskFailedBlockHandler`) to block
    /// dependent tasks when an upstream task fails with exhausted retries.
    ///
    /// If the task is already in `Blocked` or a terminal status, the transition
    /// is treated as idempotent and returns `Ok` with an empty events vec.
    pub async fn transition_to_blocked(
        &self,
        task_id: Uuid,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        // Idempotent: already blocked or terminal
        if task.status == TaskStatus::Blocked || task.status.is_terminal() {
            return Ok((task, Vec::new()));
        }

        task.transition_to(TaskStatus::Blocked).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "blocked".to_string(),
                reason: e,
            }
        })?;

        self.task_repo.update(&task).await?;
        tracing::debug!(%task_id, "task transitioned to blocked");

        // No events emitted for blocked transitions (matches existing Reaction::None behavior)
        Ok((task, Vec::new()))
    }

    /// Retry a failed task.
    ///
    /// For convergent tasks (`trajectory_id.is_some()`), the retry intentionally
    /// preserves the trajectory_id. The convergent execution path in the
    /// orchestrator detects `task.trajectory_id.is_some()` and resumes the
    /// existing trajectory (loading accumulated observations, attractor state,
    /// and bandit learning) rather than creating a new one from scratch. This
    /// ensures retry attempts build on previous convergence progress rather
    /// than discarding it. See spec Part 4.2 for full details.
    ///
    /// When a convergent task failed due to being trapped in an attractor
    /// (indicated by an `Error: trapped` hint in context), a `convergence:fresh_start`
    /// hint is added to signal the convergent execution path to force a FreshStart
    /// strategy on the next iteration. This helps escape the attractor by
    /// resetting the working state while carrying forward learned context.
    pub async fn retry_task(&self, task_id: Uuid) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        tracing::info!(%task_id, "retrying task");
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if !task.can_retry() {
            return Err(DomainError::ValidationFailed(
                "Task cannot be retried: either not failed or max retries exceeded".to_string(),
            ));
        }

        // For convergent tasks that failed due to being trapped, signal
        // the convergent execution path to force a FreshStart strategy.
        // The trap detection looks for "Error: trapped" hints added by
        // fail_task() when the convergence loop reports a Trapped outcome.
        if task.execution_mode.is_convergent() && task.trajectory_id.is_some() {
            let is_trapped = task
                .context
                .hints
                .iter()
                .any(|h| h.to_lowercase().contains("trapped"));
            if is_trapped {
                task.context
                    .push_hint_bounded("convergence:fresh_start".to_string());
            }
        }

        // Clear verification and workflow state so a retried task starts
        // with fresh idempotency keys.  Without this, the idempotency guard
        // in WorkflowVerificationHandler will match the old verification
        // task and silently skip, leaving the task stuck in Verifying.
        task.clear_verification_retry_count();
        task.clear_verification_feedback();
        task.clear_verification_idempotency_key();
        task.clear_verification_phase_context();
        task.clear_verification_aggregation_summary();

        // Reset workflow_state to Pending so the workflow restarts from
        // phase 0.  The workflow_name is preserved via routing_hints.
        if let Some(ref wf_name) = task.routing_hints.workflow_name {
            let wf_state = WorkflowState::Pending {
                workflow_name: wf_name.clone(),
            };
            let _ = task.set_workflow_state(&wf_state);
        }

        task.retry().map_err(DomainError::ValidationFailed)?;
        self.task_repo.update(&task).await?;
        tracing::info!(%task_id, attempt = task.retry_count, max_retries = task.max_retries, "task retry initiated");

        // Metrics: retry attempts.
        metrics::counter!(
            "abathur_task_retries_total",
            "template" => task.task_type.as_str()
        )
        .increment(1);

        let goal_id = Self::extract_goal_id(&task);
        let events = vec![Self::make_event(
            EventSeverity::Warning,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskRetrying {
                task_id,
                attempt: task.retry_count,
                max_attempts: task.max_retries,
            },
        )];

        self.publish_events(&events).await;
        Ok((task, events))
    }

    /// Update workflow state stored in task.context.custom["workflow_state"].
    ///
    /// This is the **only** sanctioned path for writing workflow state. It
    /// performs a load-mutate-persist cycle with retry-on-conflict (up to 3
    /// attempts) so that concurrent TaskService status transitions and
    /// WorkflowEngine state writes don't permanently collide.
    pub async fn update_workflow_state(
        &self,
        task_id: Uuid,
        state: &WorkflowState,
    ) -> DomainResult<()> {
        let value = serde_json::to_value(state)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        for attempt in 0..3u32 {
            let mut task = self
                .task_repo
                .get(task_id)
                .await?
                .ok_or(DomainError::TaskNotFound(task_id))?;
            task.set_workflow_state_value(value.clone());
            task.updated_at = chrono::Utc::now();
            match self.task_repo.update(&task).await {
                Ok(()) => return Ok(()),
                Err(DomainError::ConcurrencyConflict { .. }) if attempt < 2 => {
                    tracing::debug!(%task_id, attempt, "update_workflow_state: conflict, retrying");
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(DomainError::ConcurrencyConflict {
            entity: "Task".to_string(),
            id: task_id.to_string(),
        })
    }

    /// Update arbitrary keys in task.context.custom with retry-on-conflict.
    ///
    /// Used by the WorkflowEngine for non-workflow-state context mutations
    /// (verification feedback, retry counters, etc.).
    pub async fn update_task_context(
        &self,
        task_id: Uuid,
        updates: Vec<(String, serde_json::Value)>,
    ) -> DomainResult<()> {
        for attempt in 0..3u32 {
            let mut task = self
                .task_repo
                .get(task_id)
                .await?
                .ok_or(DomainError::TaskNotFound(task_id))?;
            for (key, val) in &updates {
                task.context.custom.insert(key.clone(), val.clone());
            }
            task.updated_at = chrono::Utc::now();
            match self.task_repo.update(&task).await {
                Ok(()) => return Ok(()),
                Err(DomainError::ConcurrencyConflict { .. }) if attempt < 2 => {
                    tracing::debug!(%task_id, attempt, "update_task_context: conflict, retrying");
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(DomainError::ConcurrencyConflict {
            entity: "Task".to_string(),
            id: task_id.to_string(),
        })
    }

    /// Transition a task to Validating status.
    ///
    /// Used by the WorkflowEngine when a phase enters verification.
    /// Idempotent: if already Validating, returns Ok with empty events.
    pub async fn transition_to_validating(
        &self,
        task_id: Uuid,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.status == TaskStatus::Validating {
            return Ok((task, Vec::new()));
        }

        // Guard: refuse to transition to Validating if the task has a WorkflowState
        // of PhaseReady or PhaseGate. These states mean the overmind should be driving
        // the workflow, and setting Validating here creates a deadlock.
        //
        // Note: we allow Aggregating, PhaseRunning, FanningOut because the normal
        // workflow flow calls transition_to_validating() BEFORE updating WorkflowState
        // to Verifying (the workflow engine does: transition_to_validating → write_state(Verifying)).
        if let Some(ws) = task.workflow_state()
            && matches!(
                ws,
                WorkflowState::PhaseReady { .. } | WorkflowState::PhaseGate { .. }
            )
        {
            tracing::warn!(
                %task_id,
                workflow_state = ?ws,
                "Refusing to transition task to Validating — workflow state is {:?}, which would cause a deadlock",
                ws
            );
            return Err(DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "validating".to_string(),
                reason: format!(
                    "task has workflow state {:?} which is not compatible with Validating — transitioning would cause a deadlock",
                    ws
                ),
            });
        }

        task.transition_to(TaskStatus::Validating).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "validating".to_string(),
                reason: e,
            }
        })?;

        self.task_repo.update(&task).await?;
        tracing::debug!(%task_id, "task transitioned to validating");

        let goal_id = Self::extract_goal_id(&task);
        let events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskValidating { task_id },
        )];

        Ok((task, events))
    }

    /// Transition a task back to Running from Validating.
    ///
    /// Used by the WorkflowEngine when verification passes and the workflow
    /// needs the parent task back in Running state to continue.
    /// Idempotent: if already Running, returns Ok with empty events.
    pub async fn transition_to_running(
        &self,
        task_id: Uuid,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.status == TaskStatus::Running {
            return Ok((task, Vec::new()));
        }

        task.transition_to(TaskStatus::Running).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "running".to_string(),
                reason: e,
            }
        })?;

        self.task_repo.update(&task).await?;
        tracing::debug!(%task_id, "task transitioned to running");

        Ok((task, Vec::new()))
    }

    /// Cancel a task.
    pub async fn cancel_task(
        &self,
        task_id: Uuid,
        reason: &str,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        tracing::info!(%task_id, reason, "cancelling task");
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.is_terminal() {
            return Err(DomainError::ValidationFailed(
                "Cannot cancel a terminal task".to_string(),
            ));
        }

        task.transition_to(TaskStatus::Canceled).map_err(|e| {
            DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "canceled".to_string(),
                reason: e,
            }
        })?;

        self.task_repo.update(&task).await?;
        tracing::info!(%task_id, reason, "task cancelled successfully");

        // Metrics: terminal cancellation.
        let template_lbl = task.task_type.as_str();
        metrics::counter!(
            "abathur_tasks_completed_total",
            "template" => template_lbl,
            "outcome" => "cancelled"
        )
        .increment(1);
        if let Some(started) = task.started_at {
            let secs = (chrono::Utc::now() - started).num_milliseconds().max(0) as f64 / 1000.0;
            metrics::histogram!(
                "abathur_task_duration_seconds",
                "template" => template_lbl,
                "outcome" => "cancelled"
            )
            .record(secs);
        }

        let goal_id = Self::extract_goal_id(&task);
        let events = vec![Self::make_event(
            EventSeverity::Warning,
            EventCategory::Task,
            goal_id,
            Some(task_id),
            EventPayload::TaskCanceled {
                task_id,
                reason: reason.to_string(),
            },
        )];

        self.publish_events(&events).await;
        Ok((task, events))
    }

    /// Force-transition a task to a new status, bypassing valid_transitions checks.
    ///
    /// This is an administrative escape hatch for unsticking tasks that are
    /// deadlocked (e.g. stuck in Validating with no verifier running).
    /// Updates the workflow_state in context.custom to match the new status
    /// when applicable.
    pub async fn force_transition(
        &self,
        task_id: Uuid,
        new_status: TaskStatus,
        reason: &str,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let old_status = task.status;

        // Bypass valid_transitions — set status directly
        task.status = new_status;
        task.updated_at = chrono::Utc::now();
        task.version += 1;

        // Update timestamps
        match new_status {
            TaskStatus::Running => task.started_at = Some(chrono::Utc::now()),
            TaskStatus::Complete | TaskStatus::Failed | TaskStatus::Canceled => {
                task.completed_at = Some(chrono::Utc::now());
            }
            _ => {}
        }

        // Update workflow_state in context.custom if present
        if let Some(ws) = task.workflow_state() {
            let workflow_name = ws.workflow_name().to_string();
            let new_ws = match new_status {
                TaskStatus::Complete => Some(WorkflowState::Completed { workflow_name }),
                TaskStatus::Failed => Some(WorkflowState::Failed {
                    workflow_name,
                    error: reason.to_string(),
                }),
                TaskStatus::Canceled => Some(WorkflowState::Failed {
                    workflow_name,
                    error: "canceled".to_string(),
                }),
                // For Running (retry), leave workflow_state as-is
                _ => None,
            };
            if let Some(new_ws) = new_ws {
                let _ = task.set_workflow_state(&new_ws);
            }
        }

        self.task_repo.update(&task).await?;

        tracing::warn!(
            %task_id,
            old_status = old_status.as_str(),
            new_status = new_status.as_str(),
            reason,
            "Force-transitioned task {} from {} to {}: {}",
            task_id, old_status.as_str(), new_status.as_str(), reason
        );

        let goal_id = Self::extract_goal_id(&task);
        let mut events = Vec::new();

        let payload = match new_status {
            TaskStatus::Complete => Some(EventPayload::TaskCompleted {
                task_id,
                tokens_used: 0,
            }),
            TaskStatus::Failed => Some(EventPayload::TaskFailed {
                task_id,
                error: reason.to_string(),
                retry_count: task.retry_count,
            }),
            TaskStatus::Canceled => Some(EventPayload::TaskCanceled {
                task_id,
                reason: reason.to_string(),
            }),
            TaskStatus::Ready => Some(EventPayload::TaskReady {
                task_id,
                task_title: task.title.clone(),
            }),
            _ => None,
        };

        if let Some(payload) = payload {
            events.push(Self::make_event(
                EventSeverity::Warning,
                EventCategory::Task,
                goal_id,
                Some(task_id),
                payload,
            ));
        }

        self.publish_events(&events).await;
        Ok((task, events))
    }
}

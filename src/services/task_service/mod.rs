//! Task service implementing business logic.

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::errors::DomainError;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{ExecutionMode, Task, TaskSource, TaskStatus, TaskType};
use crate::domain::ports::TaskRepository;
use crate::services::command_bus::{
    CommandError, CommandOutcome, CommandResult, TaskCommand, TaskCommandHandler,
};
use crate::services::event_bus::EventBus;
use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity, UnifiedEvent};
use crate::services::event_factory;

mod lifecycle;
mod queries;
mod spawn_limits;
mod submit;

#[cfg(test)]
mod tests;

pub use queries::{PruneResult, PruneSkipped};
pub use spawn_limits::{SpawnLimitConfig, SpawnLimitResult, SpawnLimitType};

#[derive(Clone)]
pub struct TaskService<T: TaskRepository> {
    task_repo: Arc<T>,
    spawn_limits: SpawnLimitConfig,
    /// Default execution mode override. When `Some`, all tasks without an
    /// explicit execution mode use this value and the classification heuristic
    /// is skipped. When `None`, the heuristic decides. This gives operators a
    /// kill switch (set to `Some(ExecutionMode::Direct)` to disable convergence
    /// inference globally).
    default_execution_mode: Option<ExecutionMode>,
    /// Optional EventBus for publishing events directly after persisting.
    /// When `Some`, events are published to the bus before being returned to
    /// the caller, closing the persist-then-publish gap (S7). When `None`,
    /// events are only returned (backward-compatible behavior for tests).
    event_bus: Option<Arc<EventBus>>,
}

impl<T: TaskRepository> TaskService<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self {
            task_repo,
            spawn_limits: SpawnLimitConfig::default(),
            default_execution_mode: None,
            event_bus: None,
        }
    }

    /// Attach an EventBus so that events are published immediately after
    /// persisting, before being returned to the caller (S7 fix).
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Create with custom spawn limits.
    pub fn with_spawn_limits(mut self, limits: SpawnLimitConfig) -> Self {
        self.spawn_limits = limits;
        self
    }

    /// Set the default execution mode override.
    ///
    /// When set to `Some(ExecutionMode::Direct)`, the classification heuristic
    /// is bypassed and all tasks default to direct execution unless they were
    /// explicitly submitted with a convergent mode. When `None`, the heuristic
    /// runs for tasks that don't have an explicit mode set.
    pub fn with_default_execution_mode(mut self, mode: Option<ExecutionMode>) -> Self {
        self.default_execution_mode = mode;
        self
    }

    /// Access the underlying task repository.
    pub fn repo(&self) -> &Arc<T> {
        &self.task_repo
    }

    /// Determine the workflow name for a task based on its characteristics.
    ///
    /// Returns `None` for tasks that should NOT be enrolled (verification tasks,
    /// review tasks, tasks already part of a workflow phase, non-root subtasks).
    ///
    /// `default_workflow` is the workflow name used for root tasks without an
    /// explicit routing hint (typically `config.default_workflow`).
    fn infer_workflow_name(task: &Task, default_workflow: &str) -> Option<String> {
        // Adapter-sourced tasks -> "external" (triage-first)
        if let TaskSource::Adapter(_) = &task.source {
            return Some("external".to_string());
        }

        // Verification and Review tasks are never enrolled
        if matches!(task.task_type, TaskType::Verification | TaskType::Review) {
            return None;
        }

        // Tasks that are already a workflow phase subtask are never enrolled
        if task.is_workflow_phase_subtask() {
            return None;
        }

        // Explicit workflow_name hint takes priority
        if let Some(ref name) = task.routing_hints.workflow_name {
            return Some(name.clone());
        }

        // Root tasks (no parent) use the configured default workflow
        if task.parent_id.is_none() {
            return Some(default_workflow.to_string());
        }

        // Other subtasks are not enrolled
        None
    }

    /// Extract the goal_id from a task's context custom data.
    ///
    /// Thin wrapper over `Task::goal_id()`; the goal_id is stored as a JSON
    /// string in `task.context.custom["goal_id"]`. Returns `None` if the key
    /// is missing or the value is not a valid UUID.
    fn extract_goal_id(task: &Task) -> Option<uuid::Uuid> {
        task.goal_id()
    }

    /// Helper to build a UnifiedEvent with standard fields.
    fn make_event(
        severity: EventSeverity,
        category: EventCategory,
        goal_id: Option<uuid::Uuid>,
        task_id: Option<uuid::Uuid>,
        payload: EventPayload,
    ) -> UnifiedEvent {
        event_factory::make_event(severity, category, goal_id, task_id, payload)
    }

    /// Publish events to the attached EventBus, if any.
    ///
    /// Called after persisting state changes so that events are delivered
    /// even if the caller never publishes them (S7 fix).
    ///
    /// When executing inside a CommandBus transaction scope, the outbox path
    /// already handles event delivery (events are inserted into the outbox
    /// table atomically and the OutboxPoller publishes them later). Publishing
    /// here as well would cause dual-delivery, so we skip in that case.
    async fn publish_events(&self, events: &[UnifiedEvent]) {
        // If a transaction scope is active, the CommandBus outbox owns delivery.
        if crate::adapters::sqlite::tx_context::try_get_tx().is_some() {
            return;
        }
        if let Some(ref bus) = self.event_bus {
            for evt in events {
                bus.publish(evt.clone()).await;
            }
        }
    }
}

#[async_trait]
impl<T: TaskRepository + 'static> TaskCommandHandler for TaskService<T> {
    async fn handle(&self, cmd: TaskCommand) -> Result<CommandOutcome, CommandError> {
        match cmd {
            TaskCommand::Submit {
                title,
                description,
                parent_id,
                priority,
                agent_type,
                depends_on,
                context,
                idempotency_key,
                source,
                deadline,
                task_type,
                execution_mode,
            } => {
                let (task, events) = self
                    .submit_task(
                        title,
                        description,
                        parent_id,
                        priority,
                        agent_type,
                        depends_on,
                        *context,
                        idempotency_key,
                        source,
                        deadline,
                        task_type,
                        execution_mode,
                    )
                    .await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::Claim {
                task_id,
                agent_type,
            } => {
                let (task, events) = self.claim_task(task_id, &agent_type).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::Complete { task_id, .. } => {
                let (task, events) = self.complete_task(task_id).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::Fail { task_id, error } => {
                let (task, events) = self.fail_task(task_id, error).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::Retry { task_id } => {
                let (task, events) = self.retry_task(task_id).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::Cancel { task_id, reason } => {
                let (task, events) = self.cancel_task(task_id, &reason).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::Assign {
                task_id,
                agent_type,
            } => {
                // Set agent_type on a Ready task without claiming it.
                // The scheduler will pick it up on the next cycle.
                let mut task = self
                    .task_repo
                    .get(task_id)
                    .await?
                    .ok_or(DomainError::TaskNotFound(task_id))?;
                if task.status != TaskStatus::Ready {
                    return Err(DomainError::InvalidStateTransition {
                        from: task.status.as_str().to_string(),
                        to: "ready (assign)".to_string(),
                        reason: "task must be in Ready state to assign an agent_type".to_string(),
                    }
                    .into());
                }
                task.agent_type = Some(agent_type);
                task.updated_at = chrono::Utc::now();
                self.task_repo.update(&task).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events: vec![],
                })
            }
            TaskCommand::Transition {
                task_id,
                new_status,
            } => {
                // Direct transition for reconciliation — load, transition, save.
                let mut task = self
                    .task_repo
                    .get(task_id)
                    .await?
                    .ok_or(DomainError::TaskNotFound(task_id))?;

                // Guard: refuse Validating transition if workflow state is not
                // Verifying (same check as transition_to_validating). This path
                // is used by goal_processing direct execution when
                // verify_on_completion is set.
                if new_status == TaskStatus::Validating
                    && let Some(ws) = task.workflow_state()
                    && matches!(
                        ws,
                        WorkflowState::PhaseReady { .. } | WorkflowState::PhaseGate { .. }
                    )
                {
                    tracing::warn!(
                        %task_id,
                        workflow_state = ?ws,
                        "Refusing Transition command to Validating — workflow state is {:?}, which would cause a deadlock",
                        ws
                    );
                    return Err(DomainError::InvalidStateTransition {
                            from: task.status.as_str().to_string(),
                            to: "validating".to_string(),
                            reason: format!(
                                "task has workflow state {:?} which is not compatible with Validating — transitioning would cause a deadlock",
                                ws
                            ),
                        }.into());
                }

                task.transition_to(new_status).map_err(|e| {
                    DomainError::InvalidStateTransition {
                        from: task.status.as_str().to_string(),
                        to: new_status.as_str().to_string(),
                        reason: e,
                    }
                })?;
                self.task_repo.update(&task).await?;

                // Collect event for the transition so handlers can react
                let mut events = Vec::new();
                let payload = match new_status {
                    TaskStatus::Ready => Some(EventPayload::TaskReady {
                        task_id,
                        task_title: task.title.clone(),
                    }),
                    TaskStatus::Complete => Some(EventPayload::TaskCompleted {
                        task_id,
                        tokens_used: 0,
                    }),
                    TaskStatus::Failed => Some(EventPayload::TaskFailed {
                        task_id,
                        error: "reconciliation-transition".into(),
                        retry_count: task.retry_count,
                    }),
                    TaskStatus::Canceled => Some(EventPayload::TaskCanceled {
                        task_id,
                        reason: "reconciliation-transition".into(),
                    }),
                    _ => None,
                };
                if let Some(payload) = payload {
                    let goal_id = Self::extract_goal_id(&task);
                    events.push(Self::make_event(
                        EventSeverity::Info,
                        EventCategory::Task,
                        goal_id,
                        Some(task_id),
                        payload,
                    ));
                }

                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
            TaskCommand::ForceTransition {
                task_id,
                new_status,
                reason,
            } => {
                let (task, events) = self.force_transition(task_id, new_status, &reason).await?;
                Ok(CommandOutcome {
                    result: CommandResult::Task(task),
                    events,
                })
            }
        }
    }
}

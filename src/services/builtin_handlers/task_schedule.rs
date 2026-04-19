//! Built-in reactive event handler.
//!
//! All handlers are **idempotent** — safe to run even if the poll loop already
//! handled the same state change. They check current state before acting.

#![allow(unused_imports)]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::domain::errors::DomainError;
use crate::domain::models::adapter::IngestionItemKind;
use crate::domain::models::convergence::{AmendmentSource, SpecificationAmendment};
use crate::domain::models::task_schedule::*;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{Goal, HumanEscalationEvent, Task, TaskSource, TaskStatus};
use crate::domain::ports::{
    GoalRepository, MemoryRepository, TaskRepository, TaskScheduleRepository, TrajectoryRepository,
    WorktreeRepository,
};
#[cfg(test)]
use crate::services::event_bus::ConvergenceTerminatedPayload;
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, HumanEscalationPayload,
    SequenceNumber, SwarmStatsPayload, TaskResultPayload, UnifiedEvent,
};
use crate::services::event_reactor::{
    ErrorStrategy, EventFilter, EventHandler, HandlerContext, HandlerId, HandlerMetadata,
    HandlerPriority, Reaction,
};
use crate::services::event_store::EventStore;
use crate::services::goal_context_service::GoalContextService;
use crate::services::memory_service::MemoryService;
use crate::services::swarm_orchestrator::SwarmStats;
use crate::services::task_service::TaskService;

use super::{try_update_task, update_with_retry};

// ============================================================================
// TaskScheduleHandler
// ============================================================================

/// Creates tasks when a task schedule fires.
///
/// Listens for `ScheduledEventFired` events with names matching
/// `"task-schedule:{uuid}"` and creates the corresponding task
/// via the CommandBus.
pub struct TaskScheduleHandler<S: TaskScheduleRepository, T: TaskRepository> {
    schedule_repo: Arc<S>,
    task_repo: Arc<T>,
    command_bus: Arc<crate::services::command_bus::CommandBus>,
}

impl<S: TaskScheduleRepository, T: TaskRepository> TaskScheduleHandler<S, T> {
    pub fn new(
        schedule_repo: Arc<S>,
        task_repo: Arc<T>,
        command_bus: Arc<crate::services::command_bus::CommandBus>,
    ) -> Self {
        Self {
            schedule_repo,
            task_repo,
            command_bus,
        }
    }

    async fn record_fire(
        &self,
        schedule_id: uuid::Uuid,
        task_id: uuid::Uuid,
    ) -> Result<(), String> {
        let mut schedule = self
            .schedule_repo
            .get(schedule_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Schedule not found".to_string())?;

        schedule.fire_count += 1;
        schedule.last_fired_at = Some(chrono::Utc::now());
        schedule.last_task_id = Some(task_id);
        schedule.updated_at = chrono::Utc::now();

        if matches!(schedule.schedule, TaskScheduleType::Once { .. }) {
            schedule.status = TaskScheduleStatus::Completed;
        }

        self.schedule_repo
            .update(&schedule)
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl<S: TaskScheduleRepository + 'static, T: TaskRepository + 'static> EventHandler
    for TaskScheduleHandler<S, T>
{
    fn metadata(&self) -> HandlerMetadata {
        HandlerMetadata {
            id: HandlerId::new(),
            name: "TaskScheduleHandler".to_string(),
            filter: EventFilter::new()
                .categories(vec![EventCategory::Scheduler])
                .payload_types(vec!["ScheduledEventFired".to_string()]),
            priority: HandlerPriority::NORMAL,
            error_strategy: ErrorStrategy::LogAndContinue,
            critical: false,
        }
    }

    async fn handle(
        &self,
        event: &UnifiedEvent,
        _ctx: &HandlerContext,
    ) -> Result<Reaction, String> {
        use crate::services::command_bus::{
            CommandEnvelope, CommandSource, DomainCommand, TaskCommand,
        };

        let (_schedule_id, name) = match &event.payload {
            EventPayload::ScheduledEventFired { schedule_id, name } => (*schedule_id, name.clone()),
            _ => return Ok(Reaction::None),
        };

        // Only handle task-schedule events
        if !name.starts_with("task-schedule:") {
            return Ok(Reaction::None);
        }

        // Extract schedule UUID from event name
        let sched_id_str = name
            .strip_prefix("task-schedule:")
            .ok_or_else(|| "Invalid task-schedule event name".to_string())?;
        let sched_id = uuid::Uuid::parse_str(sched_id_str)
            .map_err(|e| format!("Invalid schedule UUID in event name: {}", e))?;

        // Load the schedule
        let schedule = match self.schedule_repo.get(sched_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!("Task schedule {} not found, skipping", sched_id);
                return Ok(Reaction::None);
            }
            Err(e) => return Err(format!("Failed to load task schedule: {}", e)),
        };

        // Check if schedule is active
        if schedule.status != TaskScheduleStatus::Active {
            tracing::debug!("Task schedule {} is not active, skipping", schedule.name);
            return Ok(Reaction::None);
        }

        // Handle overlap policy
        match schedule.overlap_policy {
            OverlapPolicy::Skip => {
                if let Some(last_task_id) = schedule.last_task_id {
                    match self.task_repo.get(last_task_id).await {
                        Ok(Some(task)) if !task.status.is_terminal() => {
                            tracing::info!(
                                "Skipping task creation for schedule '{}': previous task {} is still {}",
                                schedule.name,
                                last_task_id,
                                task.status.as_str()
                            );
                            return Ok(Reaction::None);
                        }
                        _ => {} // Previous task done or not found, proceed
                    }
                }
            }
            OverlapPolicy::CancelPrevious => {
                if let Some(last_task_id) = schedule.last_task_id {
                    match self.task_repo.get(last_task_id).await {
                        Ok(Some(task)) if !task.status.is_terminal() => {
                            // Cancel the previous task
                            let cancel_cmd = DomainCommand::Task(TaskCommand::Cancel {
                                task_id: last_task_id,
                                reason: format!(
                                    "Superseded by new instance of schedule '{}'",
                                    schedule.name
                                ),
                            });
                            let envelope = CommandEnvelope::new(
                                CommandSource::Scheduler(schedule.name.clone()),
                                cancel_cmd,
                            );
                            if let Err(e) = self.command_bus.dispatch(envelope).await {
                                tracing::warn!(
                                    "Failed to cancel previous task {} for schedule '{}': {}",
                                    last_task_id,
                                    schedule.name,
                                    e
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            OverlapPolicy::Allow => {} // Always create
        }

        // Create the task via CommandBus
        let idempotency_key = schedule.next_idempotency_key();
        let submit_cmd = DomainCommand::Task(TaskCommand::Submit {
            title: Some(schedule.task_title.clone()),
            description: schedule.task_description.clone(),
            parent_id: None,
            priority: schedule.task_priority,
            agent_type: schedule.task_agent_type.clone(),
            depends_on: vec![],
            context: Box::new(None),
            idempotency_key: Some(idempotency_key),
            source: TaskSource::Schedule(schedule.id),
            deadline: None,
            task_type: None,
            execution_mode: None,
        });

        let envelope =
            CommandEnvelope::new(CommandSource::Scheduler(schedule.name.clone()), submit_cmd);

        match self.command_bus.dispatch(envelope).await {
            Ok(crate::services::command_bus::CommandResult::Task(task)) => {
                tracing::info!(
                    "Task schedule '{}' created task {} (fire #{})",
                    schedule.name,
                    task.id,
                    schedule.fire_count + 1
                );

                // Record the fire (best-effort update)
                if let Err(e) = self.record_fire(sched_id, task.id).await {
                    tracing::warn!(
                        "Failed to record fire for schedule '{}': {}",
                        schedule.name,
                        e
                    );
                }

                Ok(Reaction::None)
            }
            Ok(_) => {
                tracing::warn!(
                    "Unexpected command result for task schedule '{}'",
                    schedule.name
                );
                Ok(Reaction::None)
            }
            Err(crate::services::command_bus::CommandError::DuplicateCommand(_)) => {
                tracing::debug!("Duplicate fire for schedule '{}', skipping", schedule.name);
                Ok(Reaction::None)
            }
            Err(e) => {
                tracing::error!(
                    "Failed to create task for schedule '{}': {}",
                    schedule.name,
                    e
                );
                Err(format!("Task creation failed: {}", e))
            }
        }
    }
}

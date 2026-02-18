//! Service for managing periodic task schedules.
//!
//! Coordinates between TaskScheduleRepository (persistence),
//! EventScheduler (time-keeping), and CommandBus (task creation).

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::task_schedule::*;
use crate::domain::ports::task_schedule_repository::{TaskScheduleFilter, TaskScheduleRepository};
use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity};
use crate::services::event_scheduler::{EventScheduler, ScheduleType, ScheduledEvent};

pub struct TaskScheduleService<R: TaskScheduleRepository> {
    repo: Arc<R>,
}

impl<R: TaskScheduleRepository> TaskScheduleService<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }

    /// Create a new task schedule and persist it.
    /// Does NOT register with EventScheduler -- that happens in the handler/startup.
    pub async fn create_schedule(&self, schedule: TaskSchedule) -> DomainResult<TaskSchedule> {
        // Validate cron expression if applicable
        if let TaskScheduleType::Cron { ref expression } = schedule.schedule {
            use std::str::FromStr;
            cron::Schedule::from_str(expression)
                .map_err(|e| crate::domain::errors::DomainError::ValidationFailed(
                    format!("Invalid cron expression '{}': {}", expression, e)
                ))?;
        }

        // Validate interval
        if let TaskScheduleType::Interval { every_secs } = schedule.schedule {
            if every_secs < 10 {
                return Err(crate::domain::errors::DomainError::ValidationFailed(
                    "Interval must be at least 10 seconds".to_string()
                ));
            }
        }

        self.repo.create(&schedule).await?;
        Ok(schedule)
    }

    /// Get a schedule by ID.
    pub async fn get_schedule(&self, id: Uuid) -> DomainResult<Option<TaskSchedule>> {
        self.repo.get(id).await
    }

    /// Get a schedule by name.
    pub async fn get_by_name(&self, name: &str) -> DomainResult<Option<TaskSchedule>> {
        self.repo.get_by_name(name).await
    }

    /// List schedules with optional filter.
    pub async fn list_schedules(&self, filter: TaskScheduleFilter) -> DomainResult<Vec<TaskSchedule>> {
        self.repo.list(filter).await
    }

    /// Enable (unpause) a schedule.
    pub async fn enable_schedule(&self, id: Uuid) -> DomainResult<TaskSchedule> {
        let mut schedule = self.repo.get(id).await?
            .ok_or(crate::domain::errors::DomainError::TaskScheduleNotFound(id))?;
        schedule.status = TaskScheduleStatus::Active;
        schedule.updated_at = Utc::now();
        self.repo.update(&schedule).await?;
        Ok(schedule)
    }

    /// Disable (pause) a schedule.
    pub async fn disable_schedule(&self, id: Uuid) -> DomainResult<TaskSchedule> {
        let mut schedule = self.repo.get(id).await?
            .ok_or(crate::domain::errors::DomainError::TaskScheduleNotFound(id))?;
        schedule.status = TaskScheduleStatus::Paused;
        schedule.updated_at = Utc::now();
        self.repo.update(&schedule).await?;
        Ok(schedule)
    }

    /// Delete a schedule.
    pub async fn delete_schedule(&self, id: Uuid) -> DomainResult<()> {
        self.repo.delete(id).await
    }

    /// Record that a task was created by this schedule.
    pub async fn record_fire(&self, id: Uuid, task_id: Uuid) -> DomainResult<()> {
        let mut schedule = self.repo.get(id).await?
            .ok_or(crate::domain::errors::DomainError::TaskScheduleNotFound(id))?;
        schedule.fire_count += 1;
        schedule.last_fired_at = Some(Utc::now());
        schedule.last_task_id = Some(task_id);
        schedule.updated_at = Utc::now();

        // Mark one-shot schedules as completed
        if matches!(schedule.schedule, TaskScheduleType::Once { .. }) {
            schedule.status = TaskScheduleStatus::Completed;
        }

        self.repo.update(&schedule).await?;
        Ok(())
    }

    /// Convert a TaskScheduleType to an EventScheduler ScheduleType.
    pub fn to_event_schedule_type(schedule: &TaskScheduleType) -> ScheduleType {
        match schedule {
            TaskScheduleType::Once { at } => ScheduleType::Once { at: *at },
            TaskScheduleType::Interval { every_secs } => {
                ScheduleType::Interval { every: Duration::from_secs(*every_secs) }
            }
            TaskScheduleType::Cron { expression } => {
                ScheduleType::Cron { expression: expression.clone() }
            }
        }
    }

    /// Create a ScheduledEvent for registration with EventScheduler.
    pub fn to_scheduled_event(schedule: &TaskSchedule) -> ScheduledEvent {
        ScheduledEvent {
            id: Uuid::new_v4(),
            name: schedule.event_name(),
            schedule: Self::to_event_schedule_type(&schedule.schedule),
            payload: EventPayload::ScheduledEventFired {
                schedule_id: schedule.id,
                name: schedule.event_name(),
            },
            category: EventCategory::Scheduler,
            severity: EventSeverity::Info,
            goal_id: None,
            task_id: None,
            active: schedule.status == TaskScheduleStatus::Active,
            created_at: Utc::now(),
            last_fired: schedule.last_fired_at,
            fire_count: schedule.fire_count,
        }
    }

    /// Register all active schedules with the EventScheduler.
    /// Called on orchestrator startup.
    pub async fn register_active_schedules(
        &self,
        event_scheduler: &EventScheduler,
    ) -> DomainResult<usize> {
        let active = self.repo.list_active().await?;
        let mut registered = 0;

        for mut schedule in active {
            let sched_event = Self::to_scheduled_event(&schedule);
            if let Some(event_id) = event_scheduler.register(sched_event).await {
                schedule.scheduled_event_id = Some(event_id);
                self.repo.update(&schedule).await?;
                registered += 1;
            }
        }

        Ok(registered)
    }
}

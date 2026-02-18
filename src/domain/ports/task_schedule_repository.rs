//! Repository port for task schedule persistence.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::task_schedule::{TaskSchedule, TaskScheduleStatus};

/// Filter for listing task schedules.
#[derive(Debug, Default)]
pub struct TaskScheduleFilter {
    pub status: Option<TaskScheduleStatus>,
}

#[async_trait]
pub trait TaskScheduleRepository: Send + Sync {
    /// Create a new task schedule.
    async fn create(&self, schedule: &TaskSchedule) -> DomainResult<()>;

    /// Get a task schedule by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<TaskSchedule>>;

    /// Get a task schedule by name.
    async fn get_by_name(&self, name: &str) -> DomainResult<Option<TaskSchedule>>;

    /// Update an existing task schedule.
    async fn update(&self, schedule: &TaskSchedule) -> DomainResult<()>;

    /// Delete a task schedule by ID.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// List task schedules with optional filter.
    async fn list(&self, filter: TaskScheduleFilter) -> DomainResult<Vec<TaskSchedule>>;

    /// List all active task schedules.
    async fn list_active(&self) -> DomainResult<Vec<TaskSchedule>>;
}

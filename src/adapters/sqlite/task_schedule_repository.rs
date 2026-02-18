//! SQLite adapter for TaskScheduleRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::adapters::sqlite::{parse_datetime, parse_optional_datetime, parse_optional_uuid, parse_uuid};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::task_schedule::*;
use crate::domain::models::TaskPriority;
use crate::domain::ports::task_schedule_repository::{TaskScheduleFilter, TaskScheduleRepository};

#[derive(Clone)]
pub struct SqliteTaskScheduleRepository {
    pool: SqlitePool,
}

impl SqliteTaskScheduleRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct TaskScheduleRow {
    id: String,
    name: String,
    description: String,
    #[allow(dead_code)]
    schedule_type: String,
    schedule_data: String,
    task_title: String,
    task_description: String,
    task_priority: String,
    task_agent_type: Option<String>,
    overlap_policy: String,
    status: String,
    scheduled_event_id: Option<String>,
    fire_count: i64,
    last_fired_at: Option<String>,
    last_task_id: Option<String>,
    created_at: String,
    updated_at: String,
}

fn row_to_schedule(row: TaskScheduleRow) -> DomainResult<TaskSchedule> {
    let schedule: TaskScheduleType = serde_json::from_str(&row.schedule_data)
        .map_err(|e| DomainError::SerializationError(format!("schedule_data: {}", e)))?;

    Ok(TaskSchedule {
        id: parse_uuid(&row.id)?,
        name: row.name,
        description: row.description,
        schedule,
        task_title: row.task_title,
        task_description: row.task_description,
        task_priority: TaskPriority::from_str(&row.task_priority).unwrap_or(TaskPriority::Normal),
        task_agent_type: row.task_agent_type,
        overlap_policy: OverlapPolicy::from_str(&row.overlap_policy).unwrap_or_default(),
        status: TaskScheduleStatus::from_str(&row.status).unwrap_or(TaskScheduleStatus::Active),
        scheduled_event_id: parse_optional_uuid(row.scheduled_event_id)?,
        fire_count: row.fire_count as u64,
        last_fired_at: parse_optional_datetime(row.last_fired_at)?,
        last_task_id: parse_optional_uuid(row.last_task_id)?,
        created_at: parse_datetime(&row.created_at)?,
        updated_at: parse_datetime(&row.updated_at)?,
    })
}

#[async_trait]
impl TaskScheduleRepository for SqliteTaskScheduleRepository {
    async fn create(&self, schedule: &TaskSchedule) -> DomainResult<()> {
        let id = schedule.id.to_string();
        let schedule_data = serde_json::to_string(&schedule.schedule)?;
        let priority = schedule.task_priority.as_str();
        let overlap = schedule.overlap_policy.as_str();
        let status = schedule.status.as_str();
        let sched_event_id = schedule.scheduled_event_id.map(|u| u.to_string());
        let last_fired = schedule.last_fired_at.map(|dt| dt.to_rfc3339());
        let last_task = schedule.last_task_id.map(|u| u.to_string());
        let created = schedule.created_at.to_rfc3339();
        let updated = schedule.updated_at.to_rfc3339();

        sqlx::query(
            "INSERT INTO task_schedules
             (id, name, description, schedule_type, schedule_data,
              task_title, task_description, task_priority, task_agent_type,
              overlap_policy, status, scheduled_event_id,
              fire_count, last_fired_at, last_task_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
        )
        .bind(&id)
        .bind(&schedule.name)
        .bind(&schedule.description)
        .bind(schedule.schedule.as_str())
        .bind(&schedule_data)
        .bind(&schedule.task_title)
        .bind(&schedule.task_description)
        .bind(priority)
        .bind(&schedule.task_agent_type)
        .bind(overlap)
        .bind(status)
        .bind(&sched_event_id)
        .bind(schedule.fire_count as i64)
        .bind(&last_fired)
        .bind(&last_task)
        .bind(&created)
        .bind(&updated)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<TaskSchedule>> {
        let row: Option<TaskScheduleRow> =
            sqlx::query_as("SELECT * FROM task_schedules WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;

        row.map(row_to_schedule).transpose()
    }

    async fn get_by_name(&self, name: &str) -> DomainResult<Option<TaskSchedule>> {
        let row: Option<TaskScheduleRow> =
            sqlx::query_as("SELECT * FROM task_schedules WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;

        row.map(row_to_schedule).transpose()
    }

    async fn update(&self, schedule: &TaskSchedule) -> DomainResult<()> {
        let id = schedule.id.to_string();
        let schedule_data = serde_json::to_string(&schedule.schedule)?;
        let priority = schedule.task_priority.as_str();
        let overlap = schedule.overlap_policy.as_str();
        let status = schedule.status.as_str();
        let sched_event_id = schedule.scheduled_event_id.map(|u| u.to_string());
        let last_fired = schedule.last_fired_at.map(|dt| dt.to_rfc3339());
        let last_task = schedule.last_task_id.map(|u| u.to_string());
        let updated = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "UPDATE task_schedules SET
             name = ?2, description = ?3, schedule_type = ?4, schedule_data = ?5,
             task_title = ?6, task_description = ?7, task_priority = ?8, task_agent_type = ?9,
             overlap_policy = ?10, status = ?11, scheduled_event_id = ?12,
             fire_count = ?13, last_fired_at = ?14, last_task_id = ?15, updated_at = ?16
             WHERE id = ?1"
        )
        .bind(&id)
        .bind(&schedule.name)
        .bind(&schedule.description)
        .bind(schedule.schedule.as_str())
        .bind(&schedule_data)
        .bind(&schedule.task_title)
        .bind(&schedule.task_description)
        .bind(priority)
        .bind(&schedule.task_agent_type)
        .bind(overlap)
        .bind(status)
        .bind(&sched_event_id)
        .bind(schedule.fire_count as i64)
        .bind(&last_fired)
        .bind(&last_task)
        .bind(&updated)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        sqlx::query("DELETE FROM task_schedules WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list(&self, filter: TaskScheduleFilter) -> DomainResult<Vec<TaskSchedule>> {
        let rows = if let Some(status) = filter.status {
            sqlx::query_as::<_, TaskScheduleRow>(
                "SELECT * FROM task_schedules WHERE status = ? ORDER BY created_at DESC"
            )
            .bind(status.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, TaskScheduleRow>(
                "SELECT * FROM task_schedules ORDER BY created_at DESC"
            )
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(row_to_schedule).collect()
    }

    async fn list_active(&self) -> DomainResult<Vec<TaskSchedule>> {
        self.list(TaskScheduleFilter {
            status: Some(TaskScheduleStatus::Active),
        }).await
    }
}

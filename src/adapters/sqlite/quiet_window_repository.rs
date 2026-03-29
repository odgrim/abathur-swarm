//! SQLite implementation of the QuietWindowRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::quiet_window::{QuietWindow, QuietWindowStatus};
use crate::domain::ports::quiet_window_repository::{QuietWindowFilter, QuietWindowRepository};
use super::{parse_datetime, parse_uuid};

pub struct SqliteQuietWindowRepository {
    pool: SqlitePool,
}

impl SqliteQuietWindowRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct QuietWindowRow {
    id: String,
    name: String,
    description: String,
    start_cron: String,
    end_cron: String,
    timezone: String,
    status: String,
    created_at: String,
    updated_at: String,
}

impl QuietWindowRow {
    fn into_domain(self) -> DomainResult<QuietWindow> {
        Ok(QuietWindow {
            id: parse_uuid(&self.id)?,
            name: self.name,
            description: self.description,
            start_cron: self.start_cron,
            end_cron: self.end_cron,
            timezone: self.timezone,
            status: QuietWindowStatus::from_str(&self.status)
                .unwrap_or(QuietWindowStatus::Enabled),
            created_at: parse_datetime(&self.created_at)?,
            updated_at: parse_datetime(&self.updated_at)?,
        })
    }
}

#[async_trait]
impl QuietWindowRepository for SqliteQuietWindowRepository {
    async fn create(&self, window: &QuietWindow) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO quiet_windows (id, name, description, start_cron, end_cron, timezone, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(window.id.to_string())
        .bind(&window.name)
        .bind(&window.description)
        .bind(&window.start_cron)
        .bind(&window.end_cron)
        .bind(&window.timezone)
        .bind(window.status.as_str())
        .bind(window.created_at.to_rfc3339())
        .bind(window.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<QuietWindow>> {
        let row: Option<QuietWindowRow> = sqlx::query_as(
            "SELECT id, name, description, start_cron, end_cron, timezone, status, created_at, updated_at
             FROM quiet_windows WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;
        row.map(|r| r.into_domain()).transpose()
    }

    async fn get_by_name(&self, name: &str) -> DomainResult<Option<QuietWindow>> {
        let row: Option<QuietWindowRow> = sqlx::query_as(
            "SELECT id, name, description, start_cron, end_cron, timezone, status, created_at, updated_at
             FROM quiet_windows WHERE name = ?"
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;
        row.map(|r| r.into_domain()).transpose()
    }

    async fn update(&self, window: &QuietWindow) -> DomainResult<()> {
        sqlx::query(
            "UPDATE quiet_windows SET name = ?, description = ?, start_cron = ?, end_cron = ?,
             timezone = ?, status = ?, updated_at = ? WHERE id = ?"
        )
        .bind(&window.name)
        .bind(&window.description)
        .bind(&window.start_cron)
        .bind(&window.end_cron)
        .bind(&window.timezone)
        .bind(window.status.as_str())
        .bind(window.updated_at.to_rfc3339())
        .bind(window.id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        sqlx::query("DELETE FROM quiet_windows WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| DomainError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    async fn list(&self, filter: QuietWindowFilter) -> DomainResult<Vec<QuietWindow>> {
        let rows: Vec<QuietWindowRow> = if let Some(status) = filter.status {
            sqlx::query_as(
                "SELECT id, name, description, start_cron, end_cron, timezone, status, created_at, updated_at
                 FROM quiet_windows WHERE status = ? ORDER BY created_at DESC"
            )
            .bind(status.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DomainError::DatabaseError(e.to_string()))?
        } else {
            sqlx::query_as(
                "SELECT id, name, description, start_cron, end_cron, timezone, status, created_at, updated_at
                 FROM quiet_windows ORDER BY created_at DESC"
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DomainError::DatabaseError(e.to_string()))?
        };
        rows.into_iter().map(|r| r.into_domain()).collect()
    }

    async fn list_enabled(&self) -> DomainResult<Vec<QuietWindow>> {
        self.list(QuietWindowFilter { status: Some(QuietWindowStatus::Enabled) }).await
    }
}

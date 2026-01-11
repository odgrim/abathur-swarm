//! SQLite implementation of the WorktreeRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Worktree, WorktreeStatus};
use crate::domain::ports::WorktreeRepository;

pub struct SqliteWorktreeRepository {
    pool: SqlitePool,
}

impl SqliteWorktreeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorktreeRepository for SqliteWorktreeRepository {
    async fn create(&self, worktree: &Worktree) -> DomainResult<()> {
        sqlx::query(
            r#"INSERT INTO worktrees (id, task_id, path, branch, base_ref, status, merge_commit, error_message, created_at, updated_at, completed_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
        )
        .bind(worktree.id.to_string())
        .bind(worktree.task_id.to_string())
        .bind(&worktree.path)
        .bind(&worktree.branch)
        .bind(&worktree.base_ref)
        .bind(worktree.status.as_str())
        .bind(&worktree.merge_commit)
        .bind(&worktree.error_message)
        .bind(worktree.created_at.to_rfc3339())
        .bind(worktree.updated_at.to_rfc3339())
        .bind(worktree.completed_at.map(|dt| dt.to_rfc3339()))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<Worktree>> {
        let row: Option<WorktreeRow> = sqlx::query_as(
            "SELECT * FROM worktrees WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_by_task(&self, task_id: Uuid) -> DomainResult<Option<Worktree>> {
        let row: Option<WorktreeRow> = sqlx::query_as(
            "SELECT * FROM worktrees WHERE task_id = ?"
        )
        .bind(task_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_by_path(&self, path: &str) -> DomainResult<Option<Worktree>> {
        let row: Option<WorktreeRow> = sqlx::query_as(
            "SELECT * FROM worktrees WHERE path = ?"
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn update(&self, worktree: &Worktree) -> DomainResult<()> {
        let result = sqlx::query(
            r#"UPDATE worktrees SET status = ?, merge_commit = ?, error_message = ?, updated_at = ?, completed_at = ?
               WHERE id = ?"#
        )
        .bind(worktree.status.as_str())
        .bind(&worktree.merge_commit)
        .bind(&worktree.error_message)
        .bind(worktree.updated_at.to_rfc3339())
        .bind(worktree.completed_at.map(|dt| dt.to_rfc3339()))
        .bind(worktree.id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::ValidationFailed("Worktree not found".to_string()));
        }

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        let result = sqlx::query("DELETE FROM worktrees WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::ValidationFailed("Worktree not found".to_string()));
        }

        Ok(())
    }

    async fn list_by_status(&self, status: WorktreeStatus) -> DomainResult<Vec<Worktree>> {
        let rows: Vec<WorktreeRow> = sqlx::query_as(
            "SELECT * FROM worktrees WHERE status = ? ORDER BY created_at DESC"
        )
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_active(&self) -> DomainResult<Vec<Worktree>> {
        let rows: Vec<WorktreeRow> = sqlx::query_as(
            "SELECT * FROM worktrees WHERE status IN ('creating', 'active', 'completed', 'merging') ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_for_cleanup(&self) -> DomainResult<Vec<Worktree>> {
        let rows: Vec<WorktreeRow> = sqlx::query_as(
            "SELECT * FROM worktrees WHERE status IN ('merged', 'failed', 'completed') ORDER BY created_at"
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn count_by_status(&self) -> DomainResult<HashMap<WorktreeStatus, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) FROM worktrees GROUP BY status"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut counts = HashMap::new();
        for (status_str, count) in rows {
            if let Some(status) = WorktreeStatus::from_str(&status_str) {
                counts.insert(status, count as u64);
            }
        }
        Ok(counts)
    }
}

#[derive(sqlx::FromRow)]
struct WorktreeRow {
    id: String,
    task_id: String,
    path: String,
    branch: String,
    base_ref: String,
    status: String,
    merge_commit: Option<String>,
    error_message: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl TryFrom<WorktreeRow> for Worktree {
    type Error = DomainError;

    fn try_from(row: WorktreeRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let task_id = Uuid::parse_str(&row.task_id)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let status = WorktreeStatus::from_str(&row.status)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid status: {}", row.status)))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        let updated_at = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        let completed_at = row.completed_at
            .map(|s| chrono::DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        Ok(Worktree {
            id,
            task_id,
            path: row.path,
            branch: row.branch,
            base_ref: row.base_ref,
            status,
            merge_commit: row.merge_commit,
            error_message: row.error_message,
            created_at,
            updated_at,
            completed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, Migrator, all_embedded_migrations};

    async fn setup_test_repo() -> SqliteWorktreeRepository {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
        SqliteWorktreeRepository::new(pool)
    }

    #[tokio::test]
    async fn test_create_and_get_worktree() {
        let repo = setup_test_repo().await;
        let task_id = Uuid::new_v4();

        let worktree = Worktree::new(task_id, "/tmp/wt", "branch", "main");
        repo.create(&worktree).await.unwrap();

        let retrieved = repo.get(worktree.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().task_id, task_id);
    }

    #[tokio::test]
    async fn test_get_by_task() {
        let repo = setup_test_repo().await;
        let task_id = Uuid::new_v4();

        let worktree = Worktree::new(task_id, "/tmp/wt2", "branch2", "main");
        repo.create(&worktree).await.unwrap();

        let found = repo.get_by_task(task_id).await.unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn test_list_active() {
        let repo = setup_test_repo().await;

        let mut wt1 = Worktree::new(Uuid::new_v4(), "/tmp/wt3", "branch3", "main");
        wt1.activate();
        repo.create(&wt1).await.unwrap();

        let active = repo.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
    }
}

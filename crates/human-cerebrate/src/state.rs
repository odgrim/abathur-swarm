//! SQLite state management for task mappings.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// A mapping between a federation task and its corresponding ClickUp task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMapping {
    pub federation_task_id: String,
    pub correlation_id: String,
    pub clickup_task_id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub parent_goal_id: Option<String>,
    pub envelope_json: String,
    pub clickup_status: String,
    pub human_response: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deadline_at: String,
    pub result_sent: bool,
}

/// Run the migration SQL to create the task_mappings table.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::query(include_str!("../migrations/001_task_mappings.sql"))
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_mapping(pool: &SqlitePool, mapping: &TaskMapping) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO task_mappings
           (federation_task_id, correlation_id, clickup_task_id, title, status, priority,
            parent_goal_id, envelope_json, clickup_status, human_response, created_at,
            updated_at, deadline_at, result_sent)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&mapping.federation_task_id)
    .bind(&mapping.correlation_id)
    .bind(&mapping.clickup_task_id)
    .bind(&mapping.title)
    .bind(&mapping.status)
    .bind(&mapping.priority)
    .bind(&mapping.parent_goal_id)
    .bind(&mapping.envelope_json)
    .bind(&mapping.clickup_status)
    .bind(&mapping.human_response)
    .bind(&mapping.created_at)
    .bind(&mapping.updated_at)
    .bind(&mapping.deadline_at)
    .bind(mapping.result_sent)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_mapping(pool: &SqlitePool, federation_task_id: &str) -> Result<Option<TaskMapping>> {
    let row = sqlx::query_as::<_, TaskMappingRow>(
        "SELECT * FROM task_mappings WHERE federation_task_id = ?",
    )
    .bind(federation_task_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn get_active_mappings(pool: &SqlitePool) -> Result<Vec<TaskMapping>> {
    let rows = sqlx::query_as::<_, TaskMappingRow>(
        "SELECT * FROM task_mappings WHERE result_sent = 0 AND status IN ('pending', 'in_progress')",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Get all unsent task mappings regardless of status, used for reconciliation.
#[expect(dead_code, reason = "reserved for reconciliation workflow")]
pub async fn get_all_unsent(pool: &SqlitePool) -> Result<Vec<TaskMapping>> {
    let rows = sqlx::query_as::<_, TaskMappingRow>(
        "SELECT * FROM task_mappings WHERE result_sent = 0",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update_status(
    pool: &SqlitePool,
    federation_task_id: &str,
    status: &str,
    clickup_status: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE task_mappings SET status = ?, clickup_status = ?, updated_at = datetime('now') WHERE federation_task_id = ?",
    )
    .bind(status)
    .bind(clickup_status)
    .bind(federation_task_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_human_response(
    pool: &SqlitePool,
    federation_task_id: &str,
    response: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE task_mappings SET human_response = ?, updated_at = datetime('now') WHERE federation_task_id = ?",
    )
    .bind(response)
    .bind(federation_task_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_result_sent(pool: &SqlitePool, federation_task_id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE task_mappings SET result_sent = 1, updated_at = datetime('now') WHERE federation_task_id = ?",
    )
    .bind(federation_task_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_active_task_ids(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT federation_task_id FROM task_mappings WHERE result_sent = 0",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

pub async fn count_active(pool: &SqlitePool) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM task_mappings WHERE result_sent = 0 AND status IN ('pending', 'in_progress')",
    )
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Internal row type for sqlx deserialization.
#[derive(Debug, sqlx::FromRow)]
struct TaskMappingRow {
    federation_task_id: String,
    correlation_id: String,
    clickup_task_id: String,
    title: String,
    status: String,
    priority: String,
    parent_goal_id: Option<String>,
    envelope_json: String,
    clickup_status: String,
    human_response: Option<String>,
    created_at: String,
    updated_at: String,
    deadline_at: String,
    result_sent: bool,
}

impl From<TaskMappingRow> for TaskMapping {
    fn from(row: TaskMappingRow) -> Self {
        Self {
            federation_task_id: row.federation_task_id,
            correlation_id: row.correlation_id,
            clickup_task_id: row.clickup_task_id,
            title: row.title,
            status: row.status,
            priority: row.priority,
            parent_goal_id: row.parent_goal_id,
            envelope_json: row.envelope_json,
            clickup_status: row.clickup_status,
            human_response: row.human_response,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deadline_at: row.deadline_at,
            result_sent: row.result_sent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;
    use uuid::Uuid;

    async fn setup_db() -> SqlitePool {
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::from_str(":memory:")
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    fn test_mapping() -> TaskMapping {
        let now = Utc::now().to_rfc3339();
        let deadline = (Utc::now() + chrono::Duration::days(14)).to_rfc3339();
        TaskMapping {
            federation_task_id: Uuid::new_v4().to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            clickup_task_id: "abc123".to_string(),
            title: "Test task".to_string(),
            status: "pending".to_string(),
            priority: "normal".to_string(),
            parent_goal_id: None,
            envelope_json: "{}".to_string(),
            clickup_status: "to do".to_string(),
            human_response: None,
            created_at: now.clone(),
            updated_at: now,
            deadline_at: deadline,
            result_sent: false,
        }
    }

    #[tokio::test]
    async fn test_insert_and_get_mapping() {
        let pool = setup_db().await;
        let mapping = test_mapping();
        insert_mapping(&pool, &mapping).await.unwrap();

        let fetched = get_mapping(&pool, &mapping.federation_task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.clickup_task_id, "abc123");
        assert_eq!(fetched.status, "pending");
    }

    #[tokio::test]
    async fn test_active_mappings() {
        let pool = setup_db().await;
        let mapping = test_mapping();
        insert_mapping(&pool, &mapping).await.unwrap();

        let active = get_active_mappings(&pool).await.unwrap();
        assert_eq!(active.len(), 1);

        mark_result_sent(&pool, &mapping.federation_task_id)
            .await
            .unwrap();

        let active = get_active_mappings(&pool).await.unwrap();
        assert_eq!(active.len(), 0);
    }

    #[tokio::test]
    async fn test_count_active() {
        let pool = setup_db().await;
        assert_eq!(count_active(&pool).await.unwrap(), 0);

        let mapping = test_mapping();
        insert_mapping(&pool, &mapping).await.unwrap();
        assert_eq!(count_active(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_update_status() {
        let pool = setup_db().await;
        let mapping = test_mapping();
        insert_mapping(&pool, &mapping).await.unwrap();

        update_status(&pool, &mapping.federation_task_id, "completed", "complete")
            .await
            .unwrap();

        let fetched = get_mapping(&pool, &mapping.federation_task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.status, "completed");
        assert_eq!(fetched.clickup_status, "complete");
    }
}

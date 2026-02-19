//! SQLite implementation of the GoalRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Goal, GoalConstraint, GoalMetadata, GoalPriority, GoalStatus};
use crate::domain::ports::{GoalFilter, GoalRepository};

#[derive(Clone)]
pub struct SqliteGoalRepository {
    pool: SqlitePool,
}

impl SqliteGoalRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GoalRepository for SqliteGoalRepository {
    async fn create(&self, goal: &Goal) -> DomainResult<()> {
        let constraints_json = serde_json::to_string(&goal.constraints)?;
        let metadata_json = serde_json::to_string(&goal.metadata)?;
        let domains_json = serde_json::to_string(&goal.applicability_domains)?;

        sqlx::query(
            r#"INSERT INTO goals (id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', ?, ?)"#
        )
        .bind(goal.id.to_string())
        .bind(&goal.name)
        .bind(&goal.description)
        .bind(goal.status.as_str())
        .bind(goal.priority.as_str())
        .bind(goal.parent_id.map(|id| id.to_string()))
        .bind(&constraints_json)
        .bind(&metadata_json)
        .bind(&domains_json)
        .bind(goal.created_at.to_rfc3339())
        .bind(goal.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<Goal>> {
        let row: Option<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at, last_convergence_check_at FROM goals WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn update(&self, goal: &Goal) -> DomainResult<()> {
        let constraints_json = serde_json::to_string(&goal.constraints)?;
        let metadata_json = serde_json::to_string(&goal.metadata)?;
        let domains_json = serde_json::to_string(&goal.applicability_domains)?;

        let result = sqlx::query(
            r#"UPDATE goals SET name = ?, description = ?, status = ?, priority = ?,
               parent_id = ?, constraints = ?, metadata = ?, applicability_domains = ?,
               updated_at = ?
               WHERE id = ?"#
        )
        .bind(&goal.name)
        .bind(&goal.description)
        .bind(goal.status.as_str())
        .bind(goal.priority.as_str())
        .bind(goal.parent_id.map(|id| id.to_string()))
        .bind(&constraints_json)
        .bind(&metadata_json)
        .bind(&domains_json)
        .bind(goal.updated_at.to_rfc3339())
        .bind(goal.id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::GoalNotFound(goal.id));
        }

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        let result = sqlx::query("DELETE FROM goals WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::GoalNotFound(id));
        }

        Ok(())
    }

    async fn list(&self, filter: GoalFilter) -> DomainResult<Vec<Goal>> {
        let mut query = String::from(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at, last_convergence_check_at FROM goals WHERE 1=1"
        );
        let mut bindings: Vec<String> = Vec::new();

        if let Some(status) = &filter.status {
            query.push_str(" AND status = ?");
            bindings.push(status.as_str().to_string());
        }

        if let Some(priority) = &filter.priority {
            query.push_str(" AND priority = ?");
            bindings.push(priority.as_str().to_string());
        }

        if let Some(parent_id) = &filter.parent_id {
            query.push_str(" AND parent_id = ?");
            bindings.push(parent_id.to_string());
        }

        query.push_str(" ORDER BY created_at DESC");

        let mut q = sqlx::query_as::<_, GoalRow>(&query);
        for binding in &bindings {
            q = q.bind(binding);
        }

        let rows: Vec<GoalRow> = q.fetch_all(&self.pool).await?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_children(&self, parent_id: Uuid) -> DomainResult<Vec<Goal>> {
        let rows: Vec<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at, last_convergence_check_at FROM goals WHERE parent_id = ? ORDER BY created_at"
        )
        .bind(parent_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_active_with_constraints(&self) -> DomainResult<Vec<Goal>> {
        let rows: Vec<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at, last_convergence_check_at FROM goals WHERE status = 'active' ORDER BY priority DESC, created_at"
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn find_by_domains(&self, domains: &[String]) -> DomainResult<Vec<Goal>> {
        // Load all active goals, then filter in-code (SQLite JSON support is limited)
        let rows: Vec<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at, last_convergence_check_at FROM goals WHERE status = 'active'"
        )
        .fetch_all(&self.pool)
        .await?;

        let all_active: Vec<Goal> = rows.into_iter().map(|r| r.try_into()).collect::<Result<Vec<_>, _>>()?;

        Ok(all_active
            .into_iter()
            .filter(|goal| {
                goal.applicability_domains.iter().any(|d| domains.contains(d))
            })
            .collect())
    }

    async fn count_by_status(&self) -> DomainResult<HashMap<GoalStatus, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) as count FROM goals GROUP BY status"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut counts = HashMap::new();
        for (status_str, count) in rows {
            if let Some(status) = GoalStatus::from_str(&status_str) {
                counts.insert(status, count as u64);
            }
        }
        Ok(counts)
    }

    async fn update_last_check(&self, goal_id: Uuid, ts: DateTime<Utc>) -> DomainResult<()> {
        let result = sqlx::query(
            "UPDATE goals SET last_convergence_check_at = ? WHERE id = ?"
        )
        .bind(ts.to_rfc3339())
        .bind(goal_id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::GoalNotFound(goal_id));
        }

        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct GoalRow {
    id: String,
    name: String,
    description: Option<String>,
    status: String,
    priority: String,
    parent_id: Option<String>,
    constraints: Option<String>,
    metadata: Option<String>,
    applicability_domains: Option<String>,
    evaluation_criteria: Option<String>,
    created_at: String,
    updated_at: String,
    last_convergence_check_at: Option<String>,
}

impl TryFrom<GoalRow> for Goal {
    type Error = DomainError;

    fn try_from(row: GoalRow) -> Result<Self, Self::Error> {
        let id = super::parse_uuid(&row.id)?;
        let parent_id = super::parse_optional_uuid(row.parent_id)?;

        let status = GoalStatus::from_str(&row.status)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid status: {}", row.status)))?;

        let priority = GoalPriority::from_str(&row.priority)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid priority: {}", row.priority)))?;

        let constraints: Vec<GoalConstraint> = super::parse_json_or_default(row.constraints)?;
        let metadata: GoalMetadata = super::parse_json_or_default(row.metadata)?;
        let applicability_domains: Vec<String> = super::parse_json_or_default(row.applicability_domains)?;
        // evaluation_criteria column is kept for DB compatibility but no longer used
        let _evaluation_criteria: Vec<String> = super::parse_json_or_default(row.evaluation_criteria)?;

        let created_at = super::parse_datetime(&row.created_at)?;
        let updated_at = super::parse_datetime(&row.updated_at)?;
        let last_convergence_check_at = super::parse_optional_datetime(row.last_convergence_check_at)?;

        Ok(Goal {
            id,
            name: row.name,
            description: row.description.unwrap_or_default(),
            status,
            priority,
            parent_id,
            constraints,
            applicability_domains,
            metadata,
            created_at,
            updated_at,
            version: 1,
            last_convergence_check_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_migrated_test_pool;

    async fn setup_test_repo() -> SqliteGoalRepository {
        let pool = create_migrated_test_pool().await.unwrap();
        SqliteGoalRepository::new(pool)
    }

    #[tokio::test]
    async fn test_create_and_get_goal() {
        let repo = setup_test_repo().await;
        let goal = Goal::new("Test Goal", "Description");

        repo.create(&goal).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "Test Goal");
        assert_eq!(retrieved.status, GoalStatus::Active);
    }

    #[tokio::test]
    async fn test_update_goal() {
        let repo = setup_test_repo().await;
        let mut goal = Goal::new("Original", "Description");
        repo.create(&goal).await.unwrap();

        goal.name = "Updated".to_string();
        goal.updated_at = chrono::Utc::now();
        repo.update(&goal).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "Updated");
    }

    #[tokio::test]
    async fn test_delete_goal() {
        let repo = setup_test_repo().await;
        let goal = Goal::new("To Delete", "Description");
        repo.create(&goal).await.unwrap();

        repo.delete(goal.id).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_list_with_filter() {
        let repo = setup_test_repo().await;

        let goal1 = Goal::new("Active Goal", "Desc").with_priority(GoalPriority::High);
        let mut goal2 = Goal::new("Paused Goal", "Desc");
        goal2.status = GoalStatus::Paused;

        repo.create(&goal1).await.unwrap();
        repo.create(&goal2).await.unwrap();

        let active_goals = repo.list(GoalFilter { status: Some(GoalStatus::Active), ..Default::default() }).await.unwrap();
        assert_eq!(active_goals.len(), 1);
        assert_eq!(active_goals[0].name, "Active Goal");
    }

    #[tokio::test]
    async fn test_update_last_check_initial_none() {
        let repo = setup_test_repo().await;
        let goal = Goal::new("Test Goal", "Description");
        repo.create(&goal).await.unwrap();

        // Initial value should be None
        let retrieved = repo.get(goal.id).await.unwrap().unwrap();
        assert!(retrieved.last_convergence_check_at.is_none());
    }

    #[tokio::test]
    async fn test_update_last_check_persists() {
        let repo = setup_test_repo().await;
        let goal = Goal::new("Test Goal", "Description");
        repo.create(&goal).await.unwrap();

        let ts = chrono::Utc::now();
        repo.update_last_check(goal.id, ts).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap().unwrap();
        assert!(retrieved.last_convergence_check_at.is_some());

        // Timestamps are stored as RFC3339 strings, so compare with second precision
        let retrieved_ts = retrieved.last_convergence_check_at.unwrap();
        let diff = (retrieved_ts - ts).num_seconds().abs();
        assert!(diff < 2, "Timestamp should be within 2 seconds of the set value");
    }

    #[tokio::test]
    async fn test_update_last_check_not_found() {
        let repo = setup_test_repo().await;
        let nonexistent_id = uuid::Uuid::new_v4();
        let ts = chrono::Utc::now();

        let result = repo.update_last_check(nonexistent_id, ts).await;
        assert!(result.is_err(), "Expected error for nonexistent goal");
    }
}

//! SQLite implementation of the GoalRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

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
        let criteria_json = serde_json::to_string(&goal.evaluation_criteria)?;

        sqlx::query(
            r#"INSERT INTO goals (id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
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
        .bind(&criteria_json)
        .bind(goal.created_at.to_rfc3339())
        .bind(goal.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<Goal>> {
        let row: Option<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at FROM goals WHERE id = ?"
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
        let criteria_json = serde_json::to_string(&goal.evaluation_criteria)?;

        let result = sqlx::query(
            r#"UPDATE goals SET name = ?, description = ?, status = ?, priority = ?,
               parent_id = ?, constraints = ?, metadata = ?, applicability_domains = ?,
               evaluation_criteria = ?, updated_at = ?
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
        .bind(&criteria_json)
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
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at FROM goals WHERE 1=1"
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
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at FROM goals WHERE parent_id = ? ORDER BY created_at"
        )
        .bind(parent_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_active_with_constraints(&self) -> DomainResult<Vec<Goal>> {
        let rows: Vec<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at FROM goals WHERE status = 'active' ORDER BY priority DESC, created_at"
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn find_by_domains(&self, domains: &[String]) -> DomainResult<Vec<Goal>> {
        // Load all active goals, then filter in-code (SQLite JSON support is limited)
        let rows: Vec<GoalRow> = sqlx::query_as(
            "SELECT id, name, description, status, priority, parent_id, constraints, metadata, applicability_domains, evaluation_criteria, created_at, updated_at FROM goals WHERE status = 'active'"
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
}

impl TryFrom<GoalRow> for Goal {
    type Error = DomainError;

    fn try_from(row: GoalRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let parent_id = row.parent_id
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let status = GoalStatus::from_str(&row.status)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid status: {}", row.status)))?;

        let priority = GoalPriority::from_str(&row.priority)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid priority: {}", row.priority)))?;

        let constraints: Vec<GoalConstraint> = row.constraints
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let metadata: GoalMetadata = row.metadata
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let applicability_domains: Vec<String> = row.applicability_domains
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let evaluation_criteria: Vec<String> = row.evaluation_criteria
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        let updated_at = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        Ok(Goal {
            id,
            name: row.name,
            description: row.description.unwrap_or_default(),
            status,
            priority,
            parent_id,
            constraints,
            applicability_domains,
            evaluation_criteria,
            metadata,
            created_at,
            updated_at,
            version: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, Migrator, all_embedded_migrations};

    async fn setup_test_repo() -> SqliteGoalRepository {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
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
}

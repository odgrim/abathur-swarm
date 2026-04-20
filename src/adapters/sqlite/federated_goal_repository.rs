//! SQLite implementation of the FederatedGoalRepository.

use crate::exec_tx;
use async_trait::async_trait;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::goal_federation::*;
use crate::domain::ports::FederatedGoalRepository;

#[derive(Clone)]
pub struct SqliteFederatedGoalRepository {
    pool: SqlitePool,
}

impl SqliteFederatedGoalRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl FederatedGoalRepository for SqliteFederatedGoalRepository {
    async fn save(&self, goal: &FederatedGoal) -> DomainResult<()> {
        let data = serde_json::to_string(goal)?;

        let q = sqlx::query(
            r#"INSERT INTO federated_goals (id, local_goal_id, cerebrate_id, state, data, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   local_goal_id = excluded.local_goal_id,
                   cerebrate_id = excluded.cerebrate_id,
                   state = excluded.state,
                   data = excluded.data,
                   updated_at = excluded.updated_at"#,
        )
        .bind(goal.id.to_string())
        .bind(goal.local_goal_id.to_string())
        .bind(&goal.cerebrate_id)
        .bind(goal.state.as_str())
        .bind(&data)
        .bind(goal.created_at.to_rfc3339())
        .bind(goal.updated_at.to_rfc3339());

        exec_tx!(&self.pool, q, execute)?;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<FederatedGoal>> {
        let q = sqlx::query_as::<_, FederatedGoalRow>(
            "SELECT id, local_goal_id, cerebrate_id, state, data, created_at, updated_at FROM federated_goals WHERE id = ?",
        )
        .bind(id.to_string());

        let row: Option<FederatedGoalRow> = exec_tx!(&self.pool, q, fetch_optional)?;
        row.map(|r| r.try_into()).transpose()
    }

    async fn get_by_local_goal(&self, local_goal_id: Uuid) -> DomainResult<Vec<FederatedGoal>> {
        let q = sqlx::query_as::<_, FederatedGoalRow>(
            "SELECT id, local_goal_id, cerebrate_id, state, data, created_at, updated_at FROM federated_goals WHERE local_goal_id = ? ORDER BY created_at",
        )
        .bind(local_goal_id.to_string());

        let rows: Vec<FederatedGoalRow> = exec_tx!(&self.pool, q, fetch_all)?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_by_cerebrate(&self, cerebrate_id: &str) -> DomainResult<Vec<FederatedGoal>> {
        let q = sqlx::query_as::<_, FederatedGoalRow>(
            "SELECT id, local_goal_id, cerebrate_id, state, data, created_at, updated_at FROM federated_goals WHERE cerebrate_id = ? ORDER BY created_at",
        )
        .bind(cerebrate_id);

        let rows: Vec<FederatedGoalRow> = exec_tx!(&self.pool, q, fetch_all)?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_active(&self) -> DomainResult<Vec<FederatedGoal>> {
        let q = sqlx::query_as::<_, FederatedGoalRow>(
            "SELECT id, local_goal_id, cerebrate_id, state, data, created_at, updated_at FROM federated_goals WHERE state NOT IN ('converged', 'failed') ORDER BY created_at",
        );

        let rows: Vec<FederatedGoalRow> = exec_tx!(&self.pool, q, fetch_all)?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn update_state(&self, id: Uuid, state: FederatedGoalState) -> DomainResult<()> {
        // Atomic UPDATE: set both the state column and the JSON data's state/updated_at
        // fields in a single statement to avoid fetch-modify-save race conditions.
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();
        let state_str = state.as_str();

        let q = sqlx::query(
            r#"UPDATE federated_goals
               SET state = ?, data = json_set(data, '$.state', ?, '$.updated_at', ?), updated_at = ?
               WHERE id = ?"#,
        )
        .bind(state_str)
        .bind(state_str)
        .bind(&now_str)
        .bind(&now_str)
        .bind(id.to_string());

        let result = exec_tx!(&self.pool, q, execute)?;
        if result.rows_affected() == 0 {
            return Err(DomainError::ValidationFailed(format!(
                "Federated goal not found: {id}"
            )));
        }
        Ok(())
    }

    async fn update_signals(
        &self,
        id: Uuid,
        signals: ConvergenceSignalSnapshot,
    ) -> DomainResult<()> {
        // Atomic UPDATE: set the last_signals and updated_at fields in the JSON data
        // in a single statement to avoid fetch-modify-save race conditions.
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();
        let signals_json = serde_json::to_string(&signals)?;

        let q = sqlx::query(
            r#"UPDATE federated_goals
               SET data = json_set(data, '$.last_signals', json(?), '$.updated_at', ?), updated_at = ?
               WHERE id = ?"#,
        )
        .bind(&signals_json)
        .bind(&now_str)
        .bind(&now_str)
        .bind(id.to_string());

        let result = exec_tx!(&self.pool, q, execute)?;
        if result.rows_affected() == 0 {
            return Err(DomainError::ValidationFailed(format!(
                "Federated goal not found: {id}"
            )));
        }
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        let q = sqlx::query("DELETE FROM federated_goals WHERE id = ?").bind(id.to_string());

        let result = exec_tx!(&self.pool, q, execute)?;
        if result.rows_affected() == 0 {
            return Err(DomainError::ValidationFailed(format!(
                "Federated goal not found: {id}"
            )));
        }
        Ok(())
    }
}

/// reason: most columns are selected by sqlx::FromRow to match the table
/// shape but are not surfaced through `TryFrom<FederatedGoalRow>` because
/// the domain `FederatedGoal` is fully serialised in the `data` JSON column.
/// The other columns are projections used for indexed lookups (id,
/// local_goal_id, cerebrate_id, state) and audit (created_at, updated_at).
#[derive(sqlx::FromRow)]
struct FederatedGoalRow {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    local_goal_id: String,
    #[allow(dead_code)]
    cerebrate_id: String,
    #[allow(dead_code)]
    state: String,
    data: String,
    #[allow(dead_code)]
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

impl TryFrom<FederatedGoalRow> for FederatedGoal {
    type Error = DomainError;

    fn try_from(row: FederatedGoalRow) -> Result<Self, Self::Error> {
        serde_json::from_str(&row.data).map_err(|e| DomainError::SerializationError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_migrated_test_pool;
    use std::collections::HashMap;

    async fn setup_test_repo() -> SqliteFederatedGoalRepository {
        let pool = create_migrated_test_pool().await.unwrap();
        SqliteFederatedGoalRepository::new(pool)
    }

    #[tokio::test]
    async fn test_save_and_get() {
        let repo = setup_test_repo().await;
        let goal = FederatedGoal::new(Uuid::new_v4(), "cerebrate-1", "Implement feature X")
            .with_constraint("Must not break CI");

        repo.save(&goal).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, goal.id);
        assert_eq!(retrieved.cerebrate_id, "cerebrate-1");
        assert_eq!(retrieved.intent, "Implement feature X");
        assert_eq!(retrieved.constraints.len(), 1);
        assert_eq!(retrieved.state, FederatedGoalState::Pending);
    }

    #[tokio::test]
    async fn test_save_upsert() {
        let repo = setup_test_repo().await;
        let mut goal = FederatedGoal::new(Uuid::new_v4(), "cerebrate-1", "Do stuff");

        repo.save(&goal).await.unwrap();

        goal.state = FederatedGoalState::Delegated;
        goal.remote_task_id = Some("task-123".to_string());
        repo.save(&goal).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap().unwrap();
        assert_eq!(retrieved.state, FederatedGoalState::Delegated);
        assert_eq!(retrieved.remote_task_id.as_deref(), Some("task-123"));
    }

    #[tokio::test]
    async fn test_get_by_local_goal() {
        let repo = setup_test_repo().await;
        let local_id = Uuid::new_v4();

        let g1 = FederatedGoal::new(local_id, "cerebrate-1", "Part A");
        let g2 = FederatedGoal::new(local_id, "cerebrate-2", "Part B");
        let g3 = FederatedGoal::new(Uuid::new_v4(), "cerebrate-1", "Unrelated");

        repo.save(&g1).await.unwrap();
        repo.save(&g2).await.unwrap();
        repo.save(&g3).await.unwrap();

        let results = repo.get_by_local_goal(local_id).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_get_by_cerebrate() {
        let repo = setup_test_repo().await;

        let g1 = FederatedGoal::new(Uuid::new_v4(), "cerebrate-alpha", "Task 1");
        let g2 = FederatedGoal::new(Uuid::new_v4(), "cerebrate-alpha", "Task 2");
        let g3 = FederatedGoal::new(Uuid::new_v4(), "cerebrate-beta", "Task 3");

        repo.save(&g1).await.unwrap();
        repo.save(&g2).await.unwrap();
        repo.save(&g3).await.unwrap();

        let results = repo.get_by_cerebrate("cerebrate-alpha").await.unwrap();
        assert_eq!(results.len(), 2);

        let results = repo.get_by_cerebrate("cerebrate-beta").await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_get_active() {
        let repo = setup_test_repo().await;

        let mut g1 = FederatedGoal::new(Uuid::new_v4(), "c1", "Active goal");
        g1.state = FederatedGoalState::Active;

        let mut g2 = FederatedGoal::new(Uuid::new_v4(), "c2", "Converged goal");
        g2.state = FederatedGoalState::Converged;

        let mut g3 = FederatedGoal::new(Uuid::new_v4(), "c3", "Failed goal");
        g3.state = FederatedGoalState::Failed;

        let g4 = FederatedGoal::new(Uuid::new_v4(), "c4", "Pending goal");

        repo.save(&g1).await.unwrap();
        repo.save(&g2).await.unwrap();
        repo.save(&g3).await.unwrap();
        repo.save(&g4).await.unwrap();

        let active = repo.get_active().await.unwrap();
        assert_eq!(active.len(), 2); // Active + Pending, not Converged or Failed
    }

    #[tokio::test]
    async fn test_update_state() {
        let repo = setup_test_repo().await;
        let goal = FederatedGoal::new(Uuid::new_v4(), "c1", "Goal");
        repo.save(&goal).await.unwrap();

        repo.update_state(goal.id, FederatedGoalState::Delegated)
            .await
            .unwrap();

        let retrieved = repo.get(goal.id).await.unwrap().unwrap();
        assert_eq!(retrieved.state, FederatedGoalState::Delegated);
    }

    #[tokio::test]
    async fn test_update_signals() {
        let repo = setup_test_repo().await;
        let goal = FederatedGoal::new(Uuid::new_v4(), "c1", "Goal");
        repo.save(&goal).await.unwrap();

        let signals = ConvergenceSignalSnapshot {
            timestamp: chrono::Utc::now(),
            signals: HashMap::from([("build_passing".to_string(), 1.0)]),
            convergence_level: 0.75,
            task_summary: TaskStatusSummary {
                total: 5,
                completed: 3,
                failed: 0,
                running: 1,
                pending: 1,
            },
        };

        repo.update_signals(goal.id, signals).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap().unwrap();
        assert!(retrieved.last_signals.is_some());
        let snap = retrieved.last_signals.unwrap();
        assert!((snap.convergence_level - 0.75).abs() < f64::EPSILON);
        assert_eq!(snap.task_summary.completed, 3);
    }

    #[tokio::test]
    async fn test_delete() {
        let repo = setup_test_repo().await;
        let goal = FederatedGoal::new(Uuid::new_v4(), "c1", "To delete");
        repo.save(&goal).await.unwrap();

        repo.delete(goal.id).await.unwrap();

        let retrieved = repo.get(goal.id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let repo = setup_test_repo().await;
        let result = repo.delete(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_state_not_found() {
        let repo = setup_test_repo().await;
        let result = repo
            .update_state(Uuid::new_v4(), FederatedGoalState::Active)
            .await;
        assert!(result.is_err());
    }
}

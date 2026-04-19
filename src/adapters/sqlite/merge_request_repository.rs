//! SQLite implementation of the MergeRequestRepository port.

use async_trait::async_trait;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ports::MergeRequestRepository;
use crate::services::merge_queue::{MergeRequest, MergeStage, MergeStatus};

use super::{parse_datetime, parse_uuid};

/// SQLite-backed merge request repository.
#[derive(Clone)]
pub struct SqliteMergeRequestRepository {
    pool: SqlitePool,
}

impl SqliteMergeRequestRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_merge_request(&self, row: &MergeRequestRow) -> DomainResult<MergeRequest> {
        let id = parse_uuid(&row.id)?;
        let task_id = parse_uuid(&row.task_id)?;
        let stage = MergeStage::from_str(&row.stage).ok_or_else(|| {
            DomainError::SerializationError(format!("Invalid merge stage: {}", row.stage))
        })?;
        let status = MergeStatus::from_str(&row.status).ok_or_else(|| {
            DomainError::SerializationError(format!("Invalid merge status: {}", row.status))
        })?;
        let created_at = parse_datetime(&row.created_at)?;
        let updated_at = parse_datetime(&row.updated_at)?;

        let verification = row
            .verification_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let conflict_files: Vec<String> = serde_json::from_str(&row.conflict_files_json)
            .map_err(|e| {
                tracing::error!(
                    merge_request_id = %row.id,
                    error = %e,
                    "failed to deserialize conflict_files_json"
                );
                DomainError::SerializationError(format!(
                    "conflict_files_json for merge request {}: {}",
                    row.id, e
                ))
            })?;

        Ok(MergeRequest {
            id,
            stage,
            task_id,
            source_branch: row.source_branch.clone(),
            target_branch: row.target_branch.clone(),
            workdir: row.workdir.clone(),
            status,
            error: row.error.clone(),
            commit_sha: row.commit_sha.clone(),
            verification,
            conflict_files,
            attempts: row.attempts as u32,
            created_at,
            updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct MergeRequestRow {
    id: String,
    stage: String,
    task_id: String,
    source_branch: String,
    target_branch: String,
    workdir: String,
    status: String,
    error: Option<String>,
    commit_sha: Option<String>,
    verification_json: Option<String>,
    conflict_files_json: String,
    attempts: i64,
    created_at: String,
    updated_at: String,
}

#[async_trait]
impl MergeRequestRepository for SqliteMergeRequestRepository {
    async fn create(&self, request: &MergeRequest) -> DomainResult<()> {
        let id = request.id.to_string();
        let stage = request.stage.as_str();
        let task_id = request.task_id.to_string();
        let status = request.status.as_str();
        let verification_json = request
            .verification
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        let conflict_files_json = serde_json::to_string(&request.conflict_files)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        let created_at = request.created_at.to_rfc3339();
        let updated_at = request.updated_at.to_rfc3339();

        sqlx::query(
            "INSERT INTO merge_requests (id, stage, task_id, source_branch, target_branch, workdir, \
             status, error, commit_sha, verification_json, conflict_files_json, attempts, \
             created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(stage)
        .bind(&task_id)
        .bind(&request.source_branch)
        .bind(&request.target_branch)
        .bind(&request.workdir)
        .bind(status)
        .bind(&request.error)
        .bind(&request.commit_sha)
        .bind(&verification_json)
        .bind(&conflict_files_json)
        .bind(request.attempts as i64)
        .bind(&created_at)
        .bind(&updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<MergeRequest>> {
        let row: Option<MergeRequestRow> =
            sqlx::query_as("SELECT * FROM merge_requests WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        row.as_ref()
            .map(|r| self.row_to_merge_request(r))
            .transpose()
    }

    async fn update(&self, request: &MergeRequest) -> DomainResult<()> {
        let id = request.id.to_string();
        let status = request.status.as_str();
        let verification_json = request
            .verification
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        let conflict_files_json = serde_json::to_string(&request.conflict_files)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        let updated_at = request.updated_at.to_rfc3339();

        sqlx::query(
            "UPDATE merge_requests SET status = ?, error = ?, commit_sha = ?, \
             verification_json = ?, conflict_files_json = ?, attempts = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(status)
        .bind(&request.error)
        .bind(&request.commit_sha)
        .bind(&verification_json)
        .bind(&conflict_files_json)
        .bind(request.attempts as i64)
        .bind(&updated_at)
        .bind(&id)
        .execute(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn list_by_status(&self, status: MergeStatus) -> DomainResult<Vec<MergeRequest>> {
        let rows: Vec<MergeRequestRow> =
            sqlx::query_as("SELECT * FROM merge_requests WHERE status = ? ORDER BY created_at ASC")
                .bind(status.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        rows.iter().map(|r| self.row_to_merge_request(r)).collect()
    }

    async fn list_by_task(&self, task_id: Uuid) -> DomainResult<Vec<MergeRequest>> {
        let rows: Vec<MergeRequestRow> = sqlx::query_as(
            "SELECT * FROM merge_requests WHERE task_id = ? ORDER BY created_at ASC",
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        rows.iter().map(|r| self.row_to_merge_request(r)).collect()
    }

    async fn list_unresolved_conflicts(
        &self,
        max_attempts: u32,
    ) -> DomainResult<Vec<MergeRequest>> {
        let rows: Vec<MergeRequestRow> = sqlx::query_as(
            "SELECT * FROM merge_requests WHERE status = 'Conflict' AND attempts < ? \
             ORDER BY created_at ASC",
        )
        .bind(max_attempts as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        rows.iter().map(|r| self.row_to_merge_request(r)).collect()
    }

    async fn prune_terminal(&self, older_than: chrono::Duration) -> DomainResult<u64> {
        let cutoff = (chrono::Utc::now() - older_than).to_rfc3339();

        let result = sqlx::query(
            "DELETE FROM merge_requests WHERE status IN ('Completed', 'Failed', 'VerificationFailed') \
             AND updated_at < ?"
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_migrated_test_pool, insert_test_task};
    use crate::services::merge_queue::MergeStage;

    async fn make_test_request(pool: &SqlitePool) -> MergeRequest {
        let task_id = Uuid::new_v4();
        insert_test_task(pool, task_id).await;
        MergeRequest::new_stage1(
            task_id,
            "feature/test".to_string(),
            "main".to_string(),
            "/tmp/test".to_string(),
        )
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool.clone());

        let req = make_test_request(&pool).await;
        repo.create(&req).await.unwrap();

        let fetched = repo.get(req.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, req.id);
        assert_eq!(fetched.stage, MergeStage::AgentToTask);
        assert_eq!(fetched.status, MergeStatus::Queued);
        assert_eq!(fetched.source_branch, "feature/test");
    }

    #[tokio::test]
    async fn test_update_status() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool.clone());

        let mut req = make_test_request(&pool).await;
        repo.create(&req).await.unwrap();

        req.status = MergeStatus::Conflict;
        req.conflict_files = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
        req.error = Some("Merge conflicts in: src/main.rs, src/lib.rs".to_string());
        req.updated_at = chrono::Utc::now();
        repo.update(&req).await.unwrap();

        let fetched = repo.get(req.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, MergeStatus::Conflict);
        assert_eq!(fetched.conflict_files.len(), 2);
        assert!(fetched.error.is_some());
    }

    #[tokio::test]
    async fn test_list_by_status() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool.clone());

        let req1 = make_test_request(&pool).await;
        let req2 = make_test_request(&pool).await;
        repo.create(&req1).await.unwrap();
        repo.create(&req2).await.unwrap();

        let queued = repo.list_by_status(MergeStatus::Queued).await.unwrap();
        assert_eq!(queued.len(), 2);
    }

    #[tokio::test]
    async fn test_list_unresolved_conflicts() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool.clone());

        let mut req = make_test_request(&pool).await;
        repo.create(&req).await.unwrap();

        // Set to conflict
        req.status = MergeStatus::Conflict;
        req.attempts = 1;
        req.updated_at = chrono::Utc::now();
        repo.update(&req).await.unwrap();

        // Should find it with max_attempts=3
        let conflicts = repo.list_unresolved_conflicts(3).await.unwrap();
        assert_eq!(conflicts.len(), 1);

        // Should NOT find it with max_attempts=1
        let conflicts = repo.list_unresolved_conflicts(1).await.unwrap();
        assert_eq!(conflicts.len(), 0);
    }

    #[tokio::test]
    async fn test_list_by_task() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool.clone());

        let task_id = Uuid::new_v4();
        insert_test_task(&pool, task_id).await;
        let req = MergeRequest::new_stage1(
            task_id,
            "branch".to_string(),
            "main".to_string(),
            "/tmp".to_string(),
        );
        repo.create(&req).await.unwrap();

        let found = repo.list_by_task(task_id).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].task_id, task_id);
    }

    #[tokio::test]
    async fn test_prune_terminal() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool.clone());

        let mut req = make_test_request(&pool).await;
        repo.create(&req).await.unwrap();

        req.status = MergeStatus::Completed;
        req.updated_at = chrono::Utc::now();
        repo.update(&req).await.unwrap();

        // Prune with zero duration should remove it
        let pruned = repo.prune_terminal(chrono::Duration::zero()).await.unwrap();
        assert_eq!(pruned, 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteMergeRequestRepository::new(pool);

        let result = repo.get(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }
}

use crate::domain::models::{Task, TaskStatus};
use crate::domain::ports::task_repository::{DecompositionResult, IdempotentInsertResult, TaskFilters, TaskRepository};
use crate::infrastructure::database::{utils::parse_datetime, DatabaseError};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// SQLite implementation of TaskRepository using sqlx
pub struct TaskRepositoryImpl {
    pool: SqlitePool,
}

impl TaskRepositoryImpl {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Helper to convert database row to Task
    fn row_to_task(&self, row: &sqlx::sqlite::SqliteRow) -> Result<Task, DatabaseError> {
        use sqlx::Row;

        Ok(Task {
            id: Uuid::parse_str(row.get::<String, _>("id").as_str())?,
            summary: row.get("summary"),
            description: row.get("description"),
            agent_type: row.get("agent_type"),
            priority: row.get::<i64, _>("priority") as u8,
            calculated_priority: row.get("calculated_priority"),
            status: row.get::<String, _>("status").parse()?,
            dependencies: row
                .get::<Option<String>, _>("dependencies")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            dependency_type: row.get::<String, _>("dependency_type").parse()?,
            dependency_depth: row.get::<i64, _>("dependency_depth") as u32,
            input_data: row
                .get::<Option<String>, _>("input_data")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            result_data: row
                .get::<Option<String>, _>("result_data")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            error_message: row.get("error_message"),
            retry_count: row.get::<i64, _>("retry_count") as u32,
            max_retries: row.get::<i64, _>("max_retries") as u32,
            max_execution_timeout_seconds: row.get::<i64, _>("max_execution_timeout_seconds")
                as u32,
            submitted_at: parse_datetime(
                row.get::<String, _>("submitted_at").as_str(),
            )?,
            started_at: row
                .get::<Option<String>, _>("started_at")
                .as_ref()
                .and_then(|s| parse_datetime(s).ok()),
            completed_at: row
                .get::<Option<String>, _>("completed_at")
                .as_ref()
                .and_then(|s| parse_datetime(s).ok()),
            last_updated_at: parse_datetime(
                row.get::<String, _>("last_updated_at").as_str(),
            )?,
            created_by: row.get("created_by"),
            parent_task_id: row
                .get::<Option<String>, _>("parent_task_id")
                .as_ref()
                .and_then(|s| Uuid::parse_str(s).ok()),
            session_id: row
                .get::<Option<String>, _>("session_id")
                .as_ref()
                .and_then(|s| Uuid::parse_str(s).ok()),
            source: row.get::<String, _>("source").parse()?,
            deadline: row
                .get::<Option<String>, _>("deadline")
                .as_ref()
                .and_then(|s| parse_datetime(s).ok()),
            estimated_duration_seconds: row
                .get::<Option<i64>, _>("estimated_duration_seconds")
                .map(|v| v as u32),
            branch: row.get("branch"),
            feature_branch: row.get("feature_branch"),
            worktree_path: row.get("worktree_path"),

            // Validation and workflow tracking fields
            validation_requirement: row
                .get::<Option<String>, _>("validation_requirement")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default(),
            validation_task_id: row
                .get::<Option<String>, _>("validation_task_id")
                .as_ref()
                .and_then(|s| Uuid::parse_str(s).ok()),
            validating_task_id: row
                .get::<Option<String>, _>("validating_task_id")
                .as_ref()
                .and_then(|s| Uuid::parse_str(s).ok()),
            remediation_count: row
                .get::<Option<i64>, _>("remediation_count")
                .unwrap_or(0) as u32,
            is_remediation: row
                .get::<Option<i64>, _>("is_remediation")
                .unwrap_or(0) != 0,
            workflow_state: row
                .get::<Option<String>, _>("workflow_state")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            workflow_expectations: row
                .get::<Option<String>, _>("workflow_expectations")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            chain_id: row.get("chain_id"),
            chain_step_index: row
                .get::<Option<i64>, _>("chain_step_index")
                .unwrap_or(0) as usize,
            awaiting_children: row
                .get::<Option<String>, _>("awaiting_children")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            spawned_by_task_id: row
                .get::<Option<String>, _>("spawned_by_task_id")
                .as_ref()
                .and_then(|s| Uuid::parse_str(s).ok()),
            chain_handoff_state: row
                .get::<Option<String>, _>("chain_handoff_state")
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok()),
            idempotency_key: row.get("idempotency_key"),
            version: row
                .get::<Option<i64>, _>("version")
                .unwrap_or(1) as u32,
        })
    }
}

#[async_trait]
impl TaskRepository for TaskRepositoryImpl {
    async fn insert(&self, task: &Task) -> Result<(), DatabaseError> {
        // Create let bindings for all temporary values to avoid lifetime issues
        let id = task.id.to_string();
        let status = task.status.to_string();
        let dependency_type = task.dependency_type.to_string();
        let dependencies = task
            .dependencies
            .as_ref()
            .and_then(|d| serde_json::to_string(d).ok());
        let input_data = task
            .input_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let result_data = task
            .result_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let submitted_at = task.submitted_at.to_rfc3339();
        let started_at = task.started_at.as_ref().map(|dt| dt.to_rfc3339());
        let completed_at = task.completed_at.as_ref().map(|dt| dt.to_rfc3339());
        let last_updated_at = task.last_updated_at.to_rfc3339();
        let parent_task_id = task.parent_task_id.as_ref().map(|id| id.to_string());
        let session_id = task.session_id.as_ref().map(|id| id.to_string());
        let source = task.source.to_string();
        let deadline = task.deadline.as_ref().map(|dt| dt.to_rfc3339());

        // Validation and workflow tracking fields
        let validation_requirement = serde_json::to_string(&task.validation_requirement).ok();
        let validation_task_id = task.validation_task_id.as_ref().map(|id| id.to_string());
        let validating_task_id = task.validating_task_id.as_ref().map(|id| id.to_string());
        let workflow_state = task.workflow_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());
        let workflow_expectations = task.workflow_expectations
            .as_ref()
            .and_then(|e| serde_json::to_string(e).ok());
        let chain_step_index = task.chain_step_index as i64;
        let awaiting_children = task.awaiting_children
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        let spawned_by_task_id = task.spawned_by_task_id.as_ref().map(|id| id.to_string());
        let chain_handoff_state = task.chain_handoff_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());

        sqlx::query!(
            r#"
            INSERT INTO tasks (
                id, summary, description, agent_type, priority, calculated_priority,
                status, dependencies, dependency_type, dependency_depth,
                input_data, result_data, error_message, retry_count, max_retries,
                max_execution_timeout_seconds, submitted_at, started_at, completed_at,
                last_updated_at, created_by, parent_task_id, session_id, source,
                deadline, estimated_duration_seconds, branch, feature_branch,
                worktree_path, validation_requirement, validation_task_id,
                validating_task_id, remediation_count, is_remediation,
                workflow_state, workflow_expectations, chain_id, chain_step_index,
                awaiting_children, spawned_by_task_id, chain_handoff_state, idempotency_key, version
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            id,
            task.summary,
            task.description,
            task.agent_type,
            task.priority,
            task.calculated_priority,
            status,
            dependencies,
            dependency_type,
            task.dependency_depth,
            input_data,
            result_data,
            task.error_message,
            task.retry_count,
            task.max_retries,
            task.max_execution_timeout_seconds,
            submitted_at,
            started_at,
            completed_at,
            last_updated_at,
            task.created_by,
            parent_task_id,
            session_id,
            source,
            deadline,
            task.estimated_duration_seconds,
            task.branch,
            task.feature_branch,
            task.worktree_path,
            validation_requirement,
            validation_task_id,
            validating_task_id,
            task.remediation_count,
            task.is_remediation,
            workflow_state,
            workflow_expectations,
            task.chain_id,
            chain_step_index,
            awaiting_children,
            spawned_by_task_id,
            chain_handoff_state,
            task.idempotency_key,
            task.version
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Task>, DatabaseError> {
        let row = sqlx::query(
            r#"
            SELECT * FROM tasks WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(self.row_to_task(&r)?)),
            None => Ok(None),
        }
    }

    async fn update(&self, task: &Task) -> Result<(), DatabaseError> {
        // Create let bindings for all temporary values
        let id = task.id.to_string();
        let status = task.status.to_string();
        let dependency_type = task.dependency_type.to_string();
        let dependencies = task
            .dependencies
            .as_ref()
            .and_then(|d| serde_json::to_string(d).ok());
        let input_data = task
            .input_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let result_data = task
            .result_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let started_at = task.started_at.as_ref().map(|dt| dt.to_rfc3339());
        let completed_at = task.completed_at.as_ref().map(|dt| dt.to_rfc3339());
        let last_updated_at = Utc::now().to_rfc3339(); // Always update timestamp
        let parent_task_id = task.parent_task_id.as_ref().map(|id| id.to_string());
        let session_id = task.session_id.as_ref().map(|id| id.to_string());
        let source = task.source.to_string();
        let deadline = task.deadline.as_ref().map(|dt| dt.to_rfc3339());

        // Validation and workflow tracking fields
        let validation_requirement = serde_json::to_string(&task.validation_requirement).ok();
        let validation_task_id = task.validation_task_id.as_ref().map(|id| id.to_string());
        let validating_task_id = task.validating_task_id.as_ref().map(|id| id.to_string());
        let workflow_state = task.workflow_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());
        let workflow_expectations = task.workflow_expectations
            .as_ref()
            .and_then(|e| serde_json::to_string(e).ok());
        let awaiting_children = task.awaiting_children
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        let spawned_by_task_id = task.spawned_by_task_id.as_ref().map(|id| id.to_string());
        let chain_handoff_state = task.chain_handoff_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());

        // New version is current version + 1
        let new_version = task.version + 1;

        // Optimistic locking: only update if version matches
        // This prevents lost updates when multiple processes modify the same task
        let result = sqlx::query!(
            r#"
            UPDATE tasks SET
                summary = ?,
                description = ?,
                agent_type = ?,
                priority = ?,
                calculated_priority = ?,
                status = ?,
                dependencies = ?,
                dependency_type = ?,
                dependency_depth = ?,
                input_data = ?,
                result_data = ?,
                error_message = ?,
                retry_count = ?,
                max_retries = ?,
                max_execution_timeout_seconds = ?,
                started_at = ?,
                completed_at = ?,
                last_updated_at = ?,
                created_by = ?,
                parent_task_id = ?,
                session_id = ?,
                source = ?,
                deadline = ?,
                estimated_duration_seconds = ?,
                branch = ?,
                feature_branch = ?,
                worktree_path = ?,
                validation_requirement = ?,
                validation_task_id = ?,
                validating_task_id = ?,
                remediation_count = ?,
                is_remediation = ?,
                workflow_state = ?,
                workflow_expectations = ?,
                chain_id = ?,
                awaiting_children = ?,
                spawned_by_task_id = ?,
                chain_handoff_state = ?,
                idempotency_key = ?,
                version = ?
            WHERE id = ? AND version = ?
            "#,
            task.summary,
            task.description,
            task.agent_type,
            task.priority,
            task.calculated_priority,
            status,
            dependencies,
            dependency_type,
            task.dependency_depth,
            input_data,
            result_data,
            task.error_message,
            task.retry_count,
            task.max_retries,
            task.max_execution_timeout_seconds,
            started_at,
            completed_at,
            last_updated_at,
            task.created_by,
            parent_task_id,
            session_id,
            source,
            deadline,
            task.estimated_duration_seconds,
            task.branch,
            task.feature_branch,
            task.worktree_path,
            validation_requirement,
            validation_task_id,
            validating_task_id,
            task.remediation_count,
            task.is_remediation,
            workflow_state,
            workflow_expectations,
            task.chain_id,
            awaiting_children,
            spawned_by_task_id,
            chain_handoff_state,
            task.idempotency_key,
            new_version,
            id,
            task.version
        )
        .execute(&self.pool)
        .await?;

        // Check if any row was updated
        if result.rows_affected() == 0 {
            return Err(DatabaseError::OptimisticLockConflict {
                task_id: task.id,
                expected_version: task.version,
            });
        }

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), DatabaseError> {
        let id_str = id.to_string();
        sqlx::query!(r#"DELETE FROM tasks WHERE id = ?"#, id_str)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list(&self, filters: &TaskFilters) -> Result<Vec<Task>, DatabaseError> {
        // Build dynamic query based on filters
        let mut query = String::from("SELECT * FROM tasks WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(status) = &filters.status {
            query.push_str(" AND status = ?");
            bindings.push(status.to_string());
        }

        if let Some(agent_type) = &filters.agent_type {
            query.push_str(" AND agent_type = ?");
            bindings.push(agent_type.clone());
        }

        if let Some(feature_branch) = &filters.feature_branch {
            query.push_str(" AND feature_branch = ?");
            bindings.push(feature_branch.clone());
        }

        if let Some(session_id) = &filters.session_id {
            query.push_str(" AND session_id = ?");
            bindings.push(session_id.to_string());
        }

        if let Some(source) = &filters.source {
            query.push_str(" AND source = ?");
            bindings.push(source.to_string());
        }

        if let Some(exclude_status) = &filters.exclude_status {
            query.push_str(" AND status != ?");
            bindings.push(exclude_status.to_string());
        }

        query.push_str(" ORDER BY calculated_priority DESC, submitted_at ASC");

        if let Some(limit) = filters.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        // Build and execute query
        let mut query_builder = sqlx::query(&query);
        for binding in bindings {
            query_builder = query_builder.bind(binding);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;

        let tasks: Result<Vec<Task>, DatabaseError> =
            rows.iter().map(|row| self.row_to_task(row)).collect();

        tasks
    }

    async fn count(&self, filters: &TaskFilters) -> Result<i64, DatabaseError> {
        let mut query = String::from("SELECT COUNT(*) as count FROM tasks WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(status) = &filters.status {
            query.push_str(" AND status = ?");
            bindings.push(status.to_string());
        }

        if let Some(agent_type) = &filters.agent_type {
            query.push_str(" AND agent_type = ?");
            bindings.push(agent_type.clone());
        }

        if let Some(feature_branch) = &filters.feature_branch {
            query.push_str(" AND feature_branch = ?");
            bindings.push(feature_branch.clone());
        }

        if let Some(session_id) = &filters.session_id {
            query.push_str(" AND session_id = ?");
            bindings.push(session_id.to_string());
        }

        if let Some(source) = &filters.source {
            query.push_str(" AND source = ?");
            bindings.push(source.to_string());
        }

        if let Some(exclude_status) = &filters.exclude_status {
            query.push_str(" AND status != ?");
            bindings.push(exclude_status.to_string());
        }

        let mut query_builder = sqlx::query_scalar(&query);
        for binding in bindings {
            query_builder = query_builder.bind(binding);
        }

        let count: i64 = query_builder.fetch_one(&self.pool).await?;

        Ok(count)
    }

    async fn get_ready_tasks(&self, limit: usize) -> Result<Vec<Task>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM tasks
            WHERE status = ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            LIMIT ?
            "#,
        )
        .bind(TaskStatus::Ready.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let tasks: Result<Vec<Task>, DatabaseError> =
            rows.iter().map(|row| self.row_to_task(row)).collect();

        tasks
    }

    async fn update_status(&self, id: Uuid, status: TaskStatus) -> Result<(), DatabaseError> {
        let now = Utc::now();
        let status_str = status.to_string();
        let now_str = now.to_rfc3339();
        let id_str = id.to_string();

        sqlx::query!(
            r#"
            UPDATE tasks SET
                status = ?,
                last_updated_at = ?
            WHERE id = ?
            "#,
            status_str,
            now_str,
            id_str
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_by_feature_branch(
        &self,
        feature_branch: &str,
    ) -> Result<Vec<Task>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM tasks
            WHERE feature_branch = ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            "#,
        )
        .bind(feature_branch)
        .fetch_all(&self.pool)
        .await?;

        let tasks: Result<Vec<Task>, DatabaseError> =
            rows.iter().map(|row| self.row_to_task(row)).collect();

        tasks
    }

    async fn get_by_parent(&self, parent_id: Uuid) -> Result<Vec<Task>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM tasks
            WHERE parent_task_id = ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            "#,
        )
        .bind(parent_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let tasks: Result<Vec<Task>, DatabaseError> =
            rows.iter().map(|row| self.row_to_task(row)).collect();

        tasks
    }

    async fn get_dependents(&self, dependency_id: Uuid) -> Result<Vec<Task>, DatabaseError> {
        // Query tasks where the dependencies JSON array contains the dependency_id
        let dependency_str = dependency_id.to_string();
        let rows = sqlx::query(
            r#"
            SELECT * FROM tasks
            WHERE dependencies IS NOT NULL
            AND dependencies LIKE ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            "#,
        )
        .bind(format!("%{}%", dependency_str))
        .fetch_all(&self.pool)
        .await?;

        // Filter to ensure we have exact matches (not substring matches)
        let all_tasks: Vec<Task> = rows
            .iter()
            .map(|row| self.row_to_task(row))
            .collect::<Result<Vec<_>, _>>()?;

        let tasks: Vec<Task> = all_tasks
            .into_iter()
            .filter(|task| {
                task.dependencies
                    .as_ref()
                    .map(|deps| deps.contains(&dependency_id))
                    .unwrap_or(false)
            })
            .collect();

        Ok(tasks)
    }

    async fn get_by_session(&self, session_id: Uuid) -> Result<Vec<Task>, DatabaseError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM tasks
            WHERE session_id = ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            "#,
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let tasks: Result<Vec<Task>, DatabaseError> =
            rows.iter().map(|row| self.row_to_task(row)).collect();

        tasks
    }

    /// Atomically claim the next ready task using SQLite transaction.
    ///
    /// This ensures exclusive access during the SELECT + UPDATE operation, preventing
    /// race conditions where multiple workers pick up the same task.
    ///
    /// IMPORTANT: We verify that the UPDATE actually affected a row. If another worker
    /// already claimed the task between our SELECT and UPDATE, the UPDATE will affect
    /// 0 rows (because status is no longer 'Ready'), and we return None instead of
    /// returning a task we didn't actually claim.
    async fn claim_next_ready_task(&self) -> Result<Option<Task>, DatabaseError> {
        use tracing::{debug, info, warn};

        // Start a transaction for atomic SELECT + UPDATE
        // The key protection is the rows_affected check below, which ensures
        // we only return a task if we actually claimed it
        let mut tx = self.pool.begin().await?;

        // Find the highest priority ready task
        let row = sqlx::query(
            r#"
            SELECT id FROM tasks
            WHERE status = ?
            ORDER BY calculated_priority DESC, submitted_at ASC
            LIMIT 1
            "#,
        )
        .bind(TaskStatus::Ready.to_string())
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            debug!("No ready tasks to claim");
            tx.rollback().await?;
            return Ok(None);
        };

        use sqlx::Row;
        let task_id_str: String = row.get("id");
        let task_id = Uuid::parse_str(&task_id_str)?;

        // Atomically update status to Running
        let now = Utc::now().to_rfc3339();
        let running_status = TaskStatus::Running.to_string();
        let started_at = Utc::now().to_rfc3339();

        let update_result = sqlx::query(
            r#"
            UPDATE tasks SET
                status = ?,
                started_at = ?,
                last_updated_at = ?
            WHERE id = ? AND status = ?
            "#,
        )
        .bind(&running_status)
        .bind(&started_at)
        .bind(&now)
        .bind(&task_id_str)
        .bind(TaskStatus::Ready.to_string())
        .execute(&mut *tx)
        .await?;

        // CRITICAL: Check if we actually claimed the task
        // If another worker already claimed it, rows_affected will be 0
        // because the WHERE clause (status = 'Ready') won't match
        if update_result.rows_affected() == 0 {
            warn!(
                task_id = %task_id,
                "Task was claimed by another worker between SELECT and UPDATE, retrying"
            );
            tx.rollback().await?;
            // Return None to let the caller retry on next poll cycle
            // This is safe because the task is already being processed
            return Ok(None);
        }

        // Fetch the updated task
        let task_row = sqlx::query(
            r#"SELECT * FROM tasks WHERE id = ?"#,
        )
        .bind(&task_id_str)
        .fetch_one(&mut *tx)
        .await?;

        let task = self.row_to_task(&task_row)?;

        // Commit the transaction
        tx.commit().await?;

        info!(
            task_id = %task_id,
            agent_type = %task.agent_type,
            summary = %task.summary,
            "Atomically claimed task for execution"
        );

        Ok(Some(task))
    }

    async fn get_stale_running_tasks(&self, stale_threshold_secs: u64) -> Result<Vec<Task>, DatabaseError> {
        use chrono::{Duration, Utc};

        let threshold = Utc::now() - Duration::seconds(stale_threshold_secs as i64);
        let threshold_str = threshold.to_rfc3339();
        let running_status = TaskStatus::Running.to_string();

        let rows = sqlx::query(
            r#"
            SELECT * FROM tasks
            WHERE status = ?
            AND started_at IS NOT NULL
            AND started_at < ?
            ORDER BY started_at ASC
            "#,
        )
        .bind(&running_status)
        .bind(&threshold_str)
        .fetch_all(&self.pool)
        .await?;

        let tasks: Result<Vec<Task>, DatabaseError> =
            rows.iter().map(|row| self.row_to_task(row)).collect();

        tasks
    }

    async fn task_exists_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<bool, DatabaseError> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM tasks WHERE idempotency_key = ?
            "#,
        )
        .bind(idempotency_key)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    async fn get_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<Task>, DatabaseError> {
        let row = sqlx::query(
            r#"
            SELECT * FROM tasks WHERE idempotency_key = ?
            "#,
        )
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(self.row_to_task(&r)?)),
            None => Ok(None),
        }
    }

    async fn insert_task_idempotent(
        &self,
        task: &Task,
    ) -> Result<IdempotentInsertResult, DatabaseError> {
        use crate::domain::ports::task_repository::generate_auto_idempotency_key;

        // Auto-generate idempotency key if not provided using the unified function
        // This ensures ALL tasks go through the idempotent path with consistent keys
        let idempotency_key = match &task.idempotency_key {
            Some(key) => key.clone(),
            None => {
                let generated_key = generate_auto_idempotency_key(task);

                info!(
                    task_id = %task.id,
                    generated_key = %generated_key,
                    "Auto-generated idempotency key for task without explicit key"
                );

                generated_key
            }
        };

        debug!(
            task_id = %task.id,
            idempotency_key = %idempotency_key,
            "Attempting idempotent task insert"
        );

        // Use INSERT OR IGNORE with the UNIQUE constraint on idempotency_key
        // This is atomic and prevents race conditions
        let id = task.id.to_string();
        let status = task.status.to_string();
        let dependency_type = task.dependency_type.to_string();
        let dependencies = task
            .dependencies
            .as_ref()
            .and_then(|d| serde_json::to_string(d).ok());
        let input_data = task
            .input_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let result_data = task
            .result_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let submitted_at = task.submitted_at.to_rfc3339();
        let started_at = task.started_at.as_ref().map(|dt| dt.to_rfc3339());
        let completed_at = task.completed_at.as_ref().map(|dt| dt.to_rfc3339());
        let last_updated_at = task.last_updated_at.to_rfc3339();
        let parent_task_id = task.parent_task_id.as_ref().map(|id| id.to_string());
        let session_id = task.session_id.as_ref().map(|id| id.to_string());
        let source = task.source.to_string();
        let deadline = task.deadline.as_ref().map(|dt| dt.to_rfc3339());
        let validation_requirement = serde_json::to_string(&task.validation_requirement).ok();
        let validation_task_id = task.validation_task_id.as_ref().map(|id| id.to_string());
        let validating_task_id = task.validating_task_id.as_ref().map(|id| id.to_string());
        let workflow_state = task.workflow_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());
        let workflow_expectations = task.workflow_expectations
            .as_ref()
            .and_then(|e| serde_json::to_string(e).ok());
        let chain_step_index = task.chain_step_index as i64;
        let chain_handoff_state = task.chain_handoff_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());

        // Use INSERT OR IGNORE - if idempotency_key already exists, this returns 0 rows affected
        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO tasks (
                id, summary, description, agent_type, priority, calculated_priority,
                status, dependencies, dependency_type, dependency_depth,
                input_data, result_data, error_message, retry_count, max_retries,
                max_execution_timeout_seconds, submitted_at, started_at, completed_at,
                last_updated_at, created_by, parent_task_id, session_id, source,
                deadline, estimated_duration_seconds, branch, feature_branch,
                worktree_path, validation_requirement, validation_task_id,
                validating_task_id, remediation_count, is_remediation,
                workflow_state, workflow_expectations, chain_id, chain_step_index,
                chain_handoff_state, idempotency_key
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&task.summary)
        .bind(&task.description)
        .bind(&task.agent_type)
        .bind(task.priority)
        .bind(task.calculated_priority)
        .bind(&status)
        .bind(&dependencies)
        .bind(&dependency_type)
        .bind(task.dependency_depth)
        .bind(&input_data)
        .bind(&result_data)
        .bind(&task.error_message)
        .bind(task.retry_count)
        .bind(task.max_retries)
        .bind(task.max_execution_timeout_seconds)
        .bind(&submitted_at)
        .bind(&started_at)
        .bind(&completed_at)
        .bind(&last_updated_at)
        .bind(&task.created_by)
        .bind(&parent_task_id)
        .bind(&session_id)
        .bind(&source)
        .bind(&deadline)
        .bind(task.estimated_duration_seconds)
        .bind(&task.branch)
        .bind(&task.feature_branch)
        .bind(&task.worktree_path)
        .bind(&validation_requirement)
        .bind(&validation_task_id)
        .bind(&validating_task_id)
        .bind(task.remediation_count)
        .bind(task.is_remediation)
        .bind(&workflow_state)
        .bind(&workflow_expectations)
        .bind(&task.chain_id)
        .bind(chain_step_index)
        .bind(&chain_handoff_state)
        .bind(&idempotency_key)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            // Row was not inserted because idempotency_key already exists
            // Query to get the existing task's ID
            let existing_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM tasks WHERE idempotency_key = ?"
            )
            .bind(&idempotency_key)
            .fetch_optional(&self.pool)
            .await?;

            let existing_uuid = existing_id
                .and_then(|id| Uuid::parse_str(&id).ok())
                .unwrap_or_else(|| {
                    warn!(
                        idempotency_key = %idempotency_key,
                        "Could not find existing task with idempotency key, using submitted task ID"
                    );
                    task.id
                });

            info!(
                idempotency_key = %idempotency_key,
                existing_task_id = %existing_uuid,
                "Task already exists with idempotency key, skipping duplicate"
            );
            Ok(IdempotentInsertResult::AlreadyExists(existing_uuid))
        } else {
            info!(
                task_id = %task.id,
                idempotency_key = %idempotency_key,
                "Task inserted successfully with idempotency key"
            );
            Ok(IdempotentInsertResult::Inserted(task.id))
        }
    }

    /// Transactional batch insert of multiple tasks.
    ///
    /// Uses SQLite transaction to ensure all tasks are inserted atomically.
    /// If any insert fails (not due to idempotency), the entire transaction
    /// is rolled back.
    async fn insert_tasks_transactional(
        &self,
        tasks: &[Task],
    ) -> Result<crate::domain::ports::task_repository::BatchInsertResult, DatabaseError> {
        use crate::domain::ports::task_repository::{BatchInsertResult, generate_auto_idempotency_key};

        if tasks.is_empty() {
            return Ok(BatchInsertResult::new());
        }

        info!(
            task_count = tasks.len(),
            "Starting transactional batch insert of tasks"
        );

        // Start transaction
        let mut tx = self.pool.begin().await?;
        let mut result = BatchInsertResult::new();

        for task in tasks {
            // Prepare all the fields for this task
            let id = task.id.to_string();
            let status = task.status.to_string();
            let dependency_type = task.dependency_type.to_string();
            let dependencies = task
                .dependencies
                .as_ref()
                .and_then(|d| serde_json::to_string(d).ok());
            let input_data = task
                .input_data
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            let result_data = task
                .result_data
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            let submitted_at = task.submitted_at.to_rfc3339();
            let started_at = task.started_at.as_ref().map(|dt| dt.to_rfc3339());
            let completed_at = task.completed_at.as_ref().map(|dt| dt.to_rfc3339());
            let last_updated_at = task.last_updated_at.to_rfc3339();
            let parent_task_id = task.parent_task_id.as_ref().map(|id| id.to_string());
            let session_id = task.session_id.as_ref().map(|id| id.to_string());
            let source = task.source.to_string();
            let deadline = task.deadline.as_ref().map(|dt| dt.to_rfc3339());
            let validation_requirement = serde_json::to_string(&task.validation_requirement).ok();
            let validation_task_id = task.validation_task_id.as_ref().map(|id| id.to_string());
            let validating_task_id = task.validating_task_id.as_ref().map(|id| id.to_string());
            let workflow_state = task.workflow_state
                .as_ref()
                .and_then(|s| serde_json::to_string(s).ok());
            let workflow_expectations = task.workflow_expectations
                .as_ref()
                .and_then(|e| serde_json::to_string(e).ok());
            let chain_step_index = task.chain_step_index as i64;
            let chain_handoff_state = task.chain_handoff_state
                .as_ref()
                .and_then(|s| serde_json::to_string(s).ok());
            // Use the unified idempotency key generation function
            let idempotency_key = task.idempotency_key.clone().unwrap_or_else(|| {
                generate_auto_idempotency_key(task)
            });

            // Use INSERT OR IGNORE for idempotency within the transaction
            let insert_result = sqlx::query(
                r#"
                INSERT OR IGNORE INTO tasks (
                    id, summary, description, agent_type, priority, calculated_priority,
                    status, dependencies, dependency_type, dependency_depth,
                    input_data, result_data, error_message, retry_count, max_retries,
                    max_execution_timeout_seconds, submitted_at, started_at, completed_at,
                    last_updated_at, created_by, parent_task_id, session_id, source,
                    deadline, estimated_duration_seconds, branch, feature_branch,
                    worktree_path, validation_requirement, validation_task_id,
                    validating_task_id, remediation_count, is_remediation,
                    workflow_state, workflow_expectations, chain_id, chain_step_index,
                    chain_handoff_state, idempotency_key, version
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(&task.summary)
            .bind(&task.description)
            .bind(&task.agent_type)
            .bind(task.priority)
            .bind(task.calculated_priority)
            .bind(&status)
            .bind(&dependencies)
            .bind(&dependency_type)
            .bind(task.dependency_depth)
            .bind(&input_data)
            .bind(&result_data)
            .bind(&task.error_message)
            .bind(task.retry_count)
            .bind(task.max_retries)
            .bind(task.max_execution_timeout_seconds)
            .bind(&submitted_at)
            .bind(&started_at)
            .bind(&completed_at)
            .bind(&last_updated_at)
            .bind(&task.created_by)
            .bind(&parent_task_id)
            .bind(&session_id)
            .bind(&source)
            .bind(&deadline)
            .bind(task.estimated_duration_seconds)
            .bind(&task.branch)
            .bind(&task.feature_branch)
            .bind(&task.worktree_path)
            .bind(&validation_requirement)
            .bind(&validation_task_id)
            .bind(&validating_task_id)
            .bind(task.remediation_count)
            .bind(task.is_remediation)
            .bind(&workflow_state)
            .bind(&workflow_expectations)
            .bind(&task.chain_id)
            .bind(chain_step_index)
            .bind(&chain_handoff_state)
            .bind(&idempotency_key)
            .bind(task.version)
            .execute(&mut *tx)
            .await?;

            if insert_result.rows_affected() == 0 {
                // Task already exists
                debug!(
                    idempotency_key = %idempotency_key,
                    "Task already exists in transaction, skipping"
                );
                result.already_existed.push(idempotency_key);
            } else {
                debug!(
                    task_id = %task.id,
                    idempotency_key = %idempotency_key,
                    "Task inserted in transaction"
                );
                result.inserted.push(task.id);
            }
        }

        // Commit the transaction
        tx.commit().await?;

        info!(
            inserted = result.inserted.len(),
            already_existed = result.already_existed.len(),
            "Transactional batch insert completed successfully"
        );

        Ok(result)
    }

    async fn update_parent_and_insert_children_atomic(
        &self,
        parent_task: &Task,
        child_tasks: &[Task],
    ) -> Result<DecompositionResult, DatabaseError> {
        use crate::domain::ports::task_repository::generate_auto_idempotency_key;

        info!(
            parent_task_id = %parent_task.id,
            parent_version = parent_task.version,
            child_count = child_tasks.len(),
            "Starting atomic decomposition transaction (parent update + children insert)"
        );

        // Start transaction
        let mut tx = self.pool.begin().await?;

        // ========== STEP 1: Update parent task with optimistic locking ==========
        let parent_id = parent_task.id.to_string();
        let status = parent_task.status.to_string();
        let dependency_type = parent_task.dependency_type.to_string();
        let dependencies = parent_task
            .dependencies
            .as_ref()
            .and_then(|d| serde_json::to_string(d).ok());
        let input_data = parent_task
            .input_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let result_data = parent_task
            .result_data
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let started_at = parent_task.started_at.as_ref().map(|dt| dt.to_rfc3339());
        let completed_at = parent_task.completed_at.as_ref().map(|dt| dt.to_rfc3339());
        let last_updated_at = Utc::now().to_rfc3339();
        let parent_task_id_fk = parent_task.parent_task_id.as_ref().map(|id| id.to_string());
        let session_id = parent_task.session_id.as_ref().map(|id| id.to_string());
        let source = parent_task.source.to_string();
        let deadline = parent_task.deadline.as_ref().map(|dt| dt.to_rfc3339());
        let validation_requirement = serde_json::to_string(&parent_task.validation_requirement).ok();
        let validation_task_id = parent_task.validation_task_id.as_ref().map(|id| id.to_string());
        let validating_task_id = parent_task.validating_task_id.as_ref().map(|id| id.to_string());
        let workflow_state = parent_task.workflow_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());
        let workflow_expectations = parent_task.workflow_expectations
            .as_ref()
            .and_then(|e| serde_json::to_string(e).ok());
        let awaiting_children = parent_task.awaiting_children
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        let spawned_by_task_id = parent_task.spawned_by_task_id.as_ref().map(|id| id.to_string());
        let chain_handoff_state = parent_task.chain_handoff_state
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok());

        let new_version = parent_task.version + 1;

        // Update parent with optimistic locking
        let update_result = sqlx::query(
            r#"
            UPDATE tasks SET
                summary = ?,
                description = ?,
                agent_type = ?,
                priority = ?,
                calculated_priority = ?,
                status = ?,
                dependencies = ?,
                dependency_type = ?,
                dependency_depth = ?,
                input_data = ?,
                result_data = ?,
                error_message = ?,
                retry_count = ?,
                max_retries = ?,
                max_execution_timeout_seconds = ?,
                started_at = ?,
                completed_at = ?,
                last_updated_at = ?,
                created_by = ?,
                parent_task_id = ?,
                session_id = ?,
                source = ?,
                deadline = ?,
                estimated_duration_seconds = ?,
                branch = ?,
                feature_branch = ?,
                worktree_path = ?,
                validation_requirement = ?,
                validation_task_id = ?,
                validating_task_id = ?,
                remediation_count = ?,
                is_remediation = ?,
                workflow_state = ?,
                workflow_expectations = ?,
                chain_id = ?,
                awaiting_children = ?,
                spawned_by_task_id = ?,
                chain_handoff_state = ?,
                idempotency_key = ?,
                version = ?
            WHERE id = ? AND version = ?
            "#,
        )
        .bind(&parent_task.summary)
        .bind(&parent_task.description)
        .bind(&parent_task.agent_type)
        .bind(parent_task.priority)
        .bind(parent_task.calculated_priority)
        .bind(&status)
        .bind(&dependencies)
        .bind(&dependency_type)
        .bind(parent_task.dependency_depth)
        .bind(&input_data)
        .bind(&result_data)
        .bind(&parent_task.error_message)
        .bind(parent_task.retry_count)
        .bind(parent_task.max_retries)
        .bind(parent_task.max_execution_timeout_seconds)
        .bind(&started_at)
        .bind(&completed_at)
        .bind(&last_updated_at)
        .bind(&parent_task.created_by)
        .bind(&parent_task_id_fk)
        .bind(&session_id)
        .bind(&source)
        .bind(&deadline)
        .bind(parent_task.estimated_duration_seconds)
        .bind(&parent_task.branch)
        .bind(&parent_task.feature_branch)
        .bind(&parent_task.worktree_path)
        .bind(&validation_requirement)
        .bind(&validation_task_id)
        .bind(&validating_task_id)
        .bind(parent_task.remediation_count)
        .bind(parent_task.is_remediation)
        .bind(&workflow_state)
        .bind(&workflow_expectations)
        .bind(&parent_task.chain_id)
        .bind(&awaiting_children)
        .bind(&spawned_by_task_id)
        .bind(&chain_handoff_state)
        .bind(&parent_task.idempotency_key)
        .bind(new_version)
        .bind(&parent_id)
        .bind(parent_task.version)
        .execute(&mut *tx)
        .await?;

        // Check if parent was updated (optimistic lock check)
        if update_result.rows_affected() == 0 {
            // Rollback is automatic when tx is dropped
            warn!(
                parent_task_id = %parent_task.id,
                expected_version = parent_task.version,
                "Optimistic lock conflict on parent task, rolling back transaction"
            );
            return Err(DatabaseError::OptimisticLockConflict {
                task_id: parent_task.id,
                expected_version: parent_task.version,
            });
        }

        info!(
            parent_task_id = %parent_task.id,
            new_version = new_version,
            "Parent task updated successfully in transaction"
        );

        // ========== STEP 2: Insert all child tasks ==========
        let mut result = DecompositionResult::new(parent_task.id, new_version);

        for task in child_tasks {
            let id = task.id.to_string();
            let status = task.status.to_string();
            let dependency_type = task.dependency_type.to_string();
            let dependencies = task
                .dependencies
                .as_ref()
                .and_then(|d| serde_json::to_string(d).ok());
            let input_data = task
                .input_data
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            let result_data = task
                .result_data
                .as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            let submitted_at = task.submitted_at.to_rfc3339();
            let started_at = task.started_at.as_ref().map(|dt| dt.to_rfc3339());
            let completed_at = task.completed_at.as_ref().map(|dt| dt.to_rfc3339());
            let last_updated_at = task.last_updated_at.to_rfc3339();
            let parent_task_id = task.parent_task_id.as_ref().map(|id| id.to_string());
            let session_id = task.session_id.as_ref().map(|id| id.to_string());
            let source = task.source.to_string();
            let deadline = task.deadline.as_ref().map(|dt| dt.to_rfc3339());
            let validation_requirement = serde_json::to_string(&task.validation_requirement).ok();
            let validation_task_id = task.validation_task_id.as_ref().map(|id| id.to_string());
            let validating_task_id = task.validating_task_id.as_ref().map(|id| id.to_string());
            let workflow_state = task.workflow_state
                .as_ref()
                .and_then(|s| serde_json::to_string(s).ok());
            let workflow_expectations = task.workflow_expectations
                .as_ref()
                .and_then(|e| serde_json::to_string(e).ok());
            let chain_step_index = task.chain_step_index as i64;
            let chain_handoff_state = task.chain_handoff_state
                .as_ref()
                .and_then(|s| serde_json::to_string(s).ok());
            let idempotency_key = task.idempotency_key.clone().unwrap_or_else(|| {
                generate_auto_idempotency_key(task)
            });

            // Use INSERT OR IGNORE for idempotency within the transaction
            let insert_result = sqlx::query(
                r#"
                INSERT OR IGNORE INTO tasks (
                    id, summary, description, agent_type, priority, calculated_priority,
                    status, dependencies, dependency_type, dependency_depth,
                    input_data, result_data, error_message, retry_count, max_retries,
                    max_execution_timeout_seconds, submitted_at, started_at, completed_at,
                    last_updated_at, created_by, parent_task_id, session_id, source,
                    deadline, estimated_duration_seconds, branch, feature_branch,
                    worktree_path, validation_requirement, validation_task_id,
                    validating_task_id, remediation_count, is_remediation,
                    workflow_state, workflow_expectations, chain_id, chain_step_index,
                    chain_handoff_state, idempotency_key, version
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(&task.summary)
            .bind(&task.description)
            .bind(&task.agent_type)
            .bind(task.priority)
            .bind(task.calculated_priority)
            .bind(&status)
            .bind(&dependencies)
            .bind(&dependency_type)
            .bind(task.dependency_depth)
            .bind(&input_data)
            .bind(&result_data)
            .bind(&task.error_message)
            .bind(task.retry_count)
            .bind(task.max_retries)
            .bind(task.max_execution_timeout_seconds)
            .bind(&submitted_at)
            .bind(&started_at)
            .bind(&completed_at)
            .bind(&last_updated_at)
            .bind(&task.created_by)
            .bind(&parent_task_id)
            .bind(&session_id)
            .bind(&source)
            .bind(&deadline)
            .bind(task.estimated_duration_seconds)
            .bind(&task.branch)
            .bind(&task.feature_branch)
            .bind(&task.worktree_path)
            .bind(&validation_requirement)
            .bind(&validation_task_id)
            .bind(&validating_task_id)
            .bind(task.remediation_count)
            .bind(task.is_remediation)
            .bind(&workflow_state)
            .bind(&workflow_expectations)
            .bind(&task.chain_id)
            .bind(chain_step_index)
            .bind(&chain_handoff_state)
            .bind(&idempotency_key)
            .bind(task.version)
            .execute(&mut *tx)
            .await?;

            if insert_result.rows_affected() == 0 {
                debug!(
                    idempotency_key = %idempotency_key,
                    "Child task already exists in transaction, skipping"
                );
                result.children_already_existed.push(idempotency_key);
            } else {
                debug!(
                    child_task_id = %task.id,
                    idempotency_key = %idempotency_key,
                    "Child task inserted in transaction"
                );
                result.children_inserted.push(task.id);
            }
        }

        // ========== STEP 3: Commit the transaction ==========
        tx.commit().await?;

        info!(
            parent_task_id = %parent_task.id,
            parent_new_version = new_version,
            children_inserted = result.children_inserted.len(),
            children_already_existed = result.children_already_existed.len(),
            "Atomic decomposition transaction committed successfully"
        );

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::database::connection::DatabaseConnection;

    async fn setup_test_db() -> SqlitePool {
        let db = DatabaseConnection::new("sqlite::memory:")
            .await
            .expect("Failed to create test database");
        db.migrate().await.expect("Failed to run migrations");
        db.pool().clone()
    }

    #[tokio::test]
    async fn test_insert_and_get_task() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        let task = Task::new("Test task".to_string(), "Test description".to_string());

        repo.insert(&task).await.expect("Failed to insert task");

        let retrieved = repo.get(task.id).await.expect("Failed to get task");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, task.id);
        assert_eq!(retrieved.summary, task.summary);
        assert_eq!(retrieved.status, TaskStatus::Pending);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_update_task() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        let mut task = Task::new("Test task".to_string(), "Test description".to_string());
        repo.insert(&task).await.expect("Failed to insert");

        task.summary = "Updated summary".to_string();
        task.status = TaskStatus::Running;
        repo.update(&task).await.expect("Failed to update");

        let updated = repo.get(task.id).await.expect("Failed to get").unwrap();
        assert_eq!(updated.summary, "Updated summary");
        assert_eq!(updated.status, TaskStatus::Running);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_delete_task() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        let task = Task::new("Test task".to_string(), "Test description".to_string());
        repo.insert(&task).await.expect("Failed to insert");

        repo.delete(task.id).await.expect("Failed to delete");

        let deleted = repo.get(task.id).await.expect("Failed to get");
        assert!(deleted.is_none());

        pool.close().await;
    }

    #[tokio::test]
    async fn test_list_tasks_with_filters() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        // Insert multiple tasks
        for i in 0..5 {
            let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
            task.priority = i as u8;
            task.calculated_priority = i as f64;
            task.status = if i % 2 == 0 {
                TaskStatus::Ready
            } else {
                TaskStatus::Pending
            };
            repo.insert(&task).await.expect("Failed to insert");
        }

        let filters = TaskFilters {
            status: Some(TaskStatus::Ready),
            ..Default::default()
        };

        let tasks = repo.list(&filters).await.expect("Failed to list tasks");
        assert_eq!(tasks.len(), 3); // Tasks 0, 2, 4

        pool.close().await;
    }

    #[tokio::test]
    async fn test_get_ready_tasks() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        for i in 0..5 {
            let mut task = Task::new(format!("Task {}", i), format!("Description {}", i));
            task.priority = i as u8;
            task.calculated_priority = i as f64;
            task.status = if i >= 2 {
                TaskStatus::Ready
            } else {
                TaskStatus::Pending
            };
            repo.insert(&task).await.expect("Failed to insert");
        }

        let ready_tasks = repo
            .get_ready_tasks(10)
            .await
            .expect("Failed to get ready tasks");
        assert_eq!(ready_tasks.len(), 3); // Tasks 2, 3, 4
                                          // Verify they're ordered by priority descending
        assert!(ready_tasks[0].calculated_priority >= ready_tasks[1].calculated_priority);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_count_tasks() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        for i in 0..5 {
            let task = Task::new(format!("Task {}", i), format!("Description {}", i));
            repo.insert(&task).await.expect("Failed to insert");
        }

        let count = repo
            .count(&TaskFilters::default())
            .await
            .expect("Failed to count tasks");
        assert_eq!(count, 5);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_update_status() {
        let pool = setup_test_db().await;
        let repo = TaskRepositoryImpl::new(pool.clone());

        let task = Task::new("Test task".to_string(), "Test description".to_string());
        repo.insert(&task).await.expect("Failed to insert");

        repo.update_status(task.id, TaskStatus::Running)
            .await
            .expect("Failed to update status");

        let updated = repo.get(task.id).await.expect("Failed to get").unwrap();
        assert_eq!(updated.status, TaskStatus::Running);

        pool.close().await;
    }
}

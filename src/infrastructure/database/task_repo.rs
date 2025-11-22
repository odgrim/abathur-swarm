use crate::domain::models::{Task, TaskStatus};
use crate::domain::ports::task_repository::{TaskFilters, TaskRepository};
use crate::infrastructure::database::{utils::parse_datetime, DatabaseError};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
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
                workflow_state, workflow_expectations, chain_id, chain_step_index
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            chain_step_index
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

        sqlx::query!(
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
                chain_id = ?
            WHERE id = ?
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
            id
        )
        .execute(&self.pool)
        .await?;

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

//! SQLite implementation of the TaskRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    ArtifactRef, RoutingHints, Task, TaskContext, TaskPriority, TaskSource, TaskStatus,
};
use crate::domain::ports::{TaskFilter, TaskRepository};

#[derive(Clone)]
pub struct SqliteTaskRepository {
    pool: SqlitePool,
}

impl SqliteTaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TaskRepository for SqliteTaskRepository {
    async fn create(&self, task: &Task) -> DomainResult<()> {
        let routing_json = serde_json::to_string(&task.routing_hints)?;
        let artifacts_json = serde_json::to_string(&task.artifacts)?;
        let context_json = serde_json::to_string(&task.context)?;
        let (source_type, source_ref) = serialize_task_source(&task.source);

        sqlx::query(
            r#"INSERT INTO tasks (id, parent_id, title, description, status, priority,
               agent_type, routing, artifacts, context, retry_count, max_retries, worktree_path,
               idempotency_key, source_type, source_ref, version, created_at, updated_at, started_at, completed_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
        )
        .bind(task.id.to_string())
        .bind(task.parent_id.map(|id| id.to_string()))
        .bind(&task.title)
        .bind(&task.description)
        .bind(task.status.as_str())
        .bind(task.priority.as_str())
        .bind(&task.agent_type)
        .bind(&routing_json)
        .bind(&artifacts_json)
        .bind(&context_json)
        .bind(task.retry_count as i32)
        .bind(task.max_retries as i32)
        .bind(&task.worktree_path)
        .bind(&task.idempotency_key)
        .bind(&source_type)
        .bind(&source_ref)
        .bind(task.version as i64)
        .bind(task.created_at.to_rfc3339())
        .bind(task.updated_at.to_rfc3339())
        .bind(task.started_at.map(|t| t.to_rfc3339()))
        .bind(task.completed_at.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await?;

        // Add dependencies
        for dep_id in &task.depends_on {
            self.add_dependency(task.id, *dep_id).await?;
        }

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<Task>> {
        let row: Option<TaskRow> = sqlx::query_as(
            "SELECT * FROM tasks WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => {
                let mut task = r.try_into()?;
                self.load_dependencies(&mut task).await?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    async fn update(&self, task: &Task) -> DomainResult<()> {
        let routing_json = serde_json::to_string(&task.routing_hints)?;
        let artifacts_json = serde_json::to_string(&task.artifacts)?;
        let context_json = serde_json::to_string(&task.context)?;
        let (source_type, source_ref) = serialize_task_source(&task.source);

        let result = sqlx::query(
            r#"UPDATE tasks SET parent_id = ?, title = ?, description = ?,
               status = ?, priority = ?, agent_type = ?, routing = ?, artifacts = ?,
               context = ?, retry_count = ?, max_retries = ?, worktree_path = ?,
               source_type = ?, source_ref = ?,
               version = ?, updated_at = ?, started_at = ?, completed_at = ?
               WHERE id = ?"#
        )
        .bind(task.parent_id.map(|id| id.to_string()))
        .bind(&task.title)
        .bind(&task.description)
        .bind(task.status.as_str())
        .bind(task.priority.as_str())
        .bind(&task.agent_type)
        .bind(&routing_json)
        .bind(&artifacts_json)
        .bind(&context_json)
        .bind(task.retry_count as i32)
        .bind(task.max_retries as i32)
        .bind(&task.worktree_path)
        .bind(&source_type)
        .bind(&source_ref)
        .bind(task.version as i64)
        .bind(task.updated_at.to_rfc3339())
        .bind(task.started_at.map(|t| t.to_rfc3339()))
        .bind(task.completed_at.map(|t| t.to_rfc3339()))
        .bind(task.id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::TaskNotFound(task.id));
        }

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        let result = sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::TaskNotFound(id));
        }

        Ok(())
    }

    async fn list(&self, filter: TaskFilter) -> DomainResult<Vec<Task>> {
        let mut query = String::from("SELECT * FROM tasks WHERE 1=1");
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
        if let Some(agent_type) = &filter.agent_type {
            query.push_str(" AND agent_type = ?");
            bindings.push(agent_type.clone());
        }

        query.push_str(" ORDER BY created_at DESC");

        let mut q = sqlx::query_as::<_, TaskRow>(&query);
        for binding in &bindings {
            q = q.bind(binding);
        }

        let rows: Vec<TaskRow> = q.fetch_all(&self.pool).await?;
        let mut tasks = Vec::new();
        for row in rows {
            let mut task: Task = row.try_into()?;
            self.load_dependencies(&mut task).await?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    async fn list_by_status(&self, status: TaskStatus) -> DomainResult<Vec<Task>> {
        self.list(TaskFilter { status: Some(status), ..Default::default() }).await
    }

    async fn get_subtasks(&self, parent_id: Uuid) -> DomainResult<Vec<Task>> {
        self.list(TaskFilter { parent_id: Some(parent_id), ..Default::default() }).await
    }

    async fn get_ready_tasks(&self, limit: usize) -> DomainResult<Vec<Task>> {
        let rows: Vec<TaskRow> = sqlx::query_as(
            r#"SELECT * FROM tasks WHERE status = 'ready'
               ORDER BY CASE priority
                   WHEN 'critical' THEN 1
                   WHEN 'high' THEN 2
                   WHEN 'normal' THEN 3
                   WHEN 'low' THEN 4
               END, created_at
               LIMIT ?"#
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut tasks = Vec::new();
        for row in rows {
            let mut task: Task = row.try_into()?;
            self.load_dependencies(&mut task).await?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    async fn get_by_agent(&self, agent_type: &str) -> DomainResult<Vec<Task>> {
        self.list(TaskFilter { agent_type: Some(agent_type.to_string()), ..Default::default() }).await
    }

    async fn get_dependencies(&self, task_id: Uuid) -> DomainResult<Vec<Task>> {
        let rows: Vec<TaskRow> = sqlx::query_as(
            r#"SELECT t.* FROM tasks t
               INNER JOIN task_dependencies d ON t.id = d.depends_on_id
               WHERE d.task_id = ?"#
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_dependents(&self, task_id: Uuid) -> DomainResult<Vec<Task>> {
        let rows: Vec<TaskRow> = sqlx::query_as(
            r#"SELECT t.* FROM tasks t
               INNER JOIN task_dependencies d ON t.id = d.task_id
               WHERE d.depends_on_id = ?"#
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn add_dependency(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?, ?)"
        )
        .bind(task_id.to_string())
        .bind(depends_on.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_dependency(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<()> {
        sqlx::query(
            "DELETE FROM task_dependencies WHERE task_id = ? AND depends_on_id = ?"
        )
        .bind(task_id.to_string())
        .bind(depends_on.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn count_descendants(&self, task_id: Uuid) -> DomainResult<u64> {
        // Simple recursive CTE to count all descendants
        let result: (i64,) = sqlx::query_as(
            r#"WITH RECURSIVE descendants AS (
                SELECT id FROM tasks WHERE parent_id = ?
                UNION ALL
                SELECT t.id FROM tasks t INNER JOIN descendants d ON t.parent_id = d.id
            )
            SELECT COUNT(*) FROM descendants"#
        )
        .bind(task_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0 as u64)
    }

    async fn get_by_idempotency_key(&self, key: &str) -> DomainResult<Option<Task>> {
        let row: Option<TaskRow> = sqlx::query_as(
            "SELECT * FROM tasks WHERE idempotency_key = ?"
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => {
                let mut task = r.try_into()?;
                self.load_dependencies(&mut task).await?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    async fn list_by_source(&self, source_type: &str) -> DomainResult<Vec<Task>> {
        let rows: Vec<TaskRow> = sqlx::query_as(
            "SELECT * FROM tasks WHERE source_type = ? ORDER BY created_at DESC"
        )
        .bind(source_type)
        .fetch_all(&self.pool)
        .await?;

        let mut tasks = Vec::new();
        for row in rows {
            let mut task: Task = row.try_into()?;
            self.load_dependencies(&mut task).await?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    async fn count_by_status(&self) -> DomainResult<HashMap<TaskStatus, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) FROM tasks GROUP BY status"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut counts = HashMap::new();
        for (status_str, count) in rows {
            if let Some(status) = TaskStatus::from_str(&status_str) {
                counts.insert(status, count as u64);
            }
        }
        Ok(counts)
    }
}

impl SqliteTaskRepository {
    async fn load_dependencies(&self, task: &mut Task) -> DomainResult<()> {
        let deps: Vec<(String,)> = sqlx::query_as(
            "SELECT depends_on_id FROM task_dependencies WHERE task_id = ?"
        )
        .bind(task.id.to_string())
        .fetch_all(&self.pool)
        .await?;

        task.depends_on = deps
            .into_iter()
            .filter_map(|(id,)| Uuid::parse_str(&id).ok())
            .collect();

        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct TaskRow {
    id: String,
    parent_id: Option<String>,
    title: String,
    description: Option<String>,
    status: String,
    priority: String,
    agent_type: Option<String>,
    routing: Option<String>,
    artifacts: Option<String>,
    context: Option<String>,
    retry_count: i32,
    max_retries: i32,
    worktree_path: Option<String>,
    idempotency_key: Option<String>,
    source_type: Option<String>,
    source_ref: Option<String>,
    version: i64,
    created_at: String,
    updated_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl TryFrom<TaskRow> for Task {
    type Error = DomainError;

    fn try_from(row: TaskRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let parent_id = row.parent_id
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let status = TaskStatus::from_str(&row.status)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid status: {}", row.status)))?;

        let priority = TaskPriority::from_str(&row.priority)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid priority: {}", row.priority)))?;

        let routing_hints: RoutingHints = row.routing
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let artifacts: Vec<ArtifactRef> = row.artifacts
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let context: TaskContext = row.context
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

        let started_at = row.started_at
            .map(|s| chrono::DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let completed_at = row.completed_at
            .map(|s| chrono::DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let source = deserialize_task_source(row.source_type.as_deref(), row.source_ref.as_deref())?;

        Ok(Task {
            id,
            parent_id,
            title: row.title,
            description: row.description.unwrap_or_default(),
            agent_type: row.agent_type,
            routing_hints,
            depends_on: Vec::new(), // Loaded separately
            status,
            priority,
            retry_count: row.retry_count as u32,
            max_retries: row.max_retries as u32,
            artifacts,
            worktree_path: row.worktree_path,
            context,
            source,
            created_at,
            updated_at,
            started_at,
            completed_at,
            version: row.version as u64,
            idempotency_key: row.idempotency_key,
        })
    }
}

/// Serialize a TaskSource into (source_type, source_ref) for DB storage.
fn serialize_task_source(source: &TaskSource) -> (String, Option<String>) {
    match source {
        TaskSource::Human => ("human".to_string(), None),
        TaskSource::System => ("system".to_string(), None),
        TaskSource::SubtaskOf(uuid) => ("subtask".to_string(), Some(uuid.to_string())),
    }
}

/// Deserialize (source_type, source_ref) from DB into a TaskSource.
fn deserialize_task_source(
    source_type: Option<&str>,
    source_ref: Option<&str>,
) -> Result<TaskSource, DomainError> {
    match source_type {
        Some("human") | None => Ok(TaskSource::Human),
        Some("system") => Ok(TaskSource::System),
        Some("subtask") => {
            let uuid_str = source_ref.ok_or_else(|| {
                DomainError::SerializationError("subtask source requires source_ref".to_string())
            })?;
            let uuid = Uuid::parse_str(uuid_str)
                .map_err(|e| DomainError::SerializationError(e.to_string()))?;
            Ok(TaskSource::SubtaskOf(uuid))
        }
        // Legacy: goal_evaluation rows in DB are treated as Human
        Some("goal_evaluation") => Ok(TaskSource::Human),
        Some(other) => Err(DomainError::SerializationError(format!(
            "Unknown source_type: {}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, Migrator, all_embedded_migrations};

    async fn setup_test_repo() -> SqliteTaskRepository {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
        SqliteTaskRepository::new(pool)
    }

    #[tokio::test]
    async fn test_create_and_get_task() {
        let repo = setup_test_repo().await;
        let task = Task::with_title("Test Task", "Description");

        repo.create(&task).await.unwrap();

        let retrieved = repo.get(task.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().title, "Test Task");
    }

    #[tokio::test]
    async fn test_task_dependencies() {
        let repo = setup_test_repo().await;

        let dep_task = Task::with_title("Dependency", "Desc");
        let main_task = Task::with_title("Main", "Desc").with_dependency(dep_task.id);

        repo.create(&dep_task).await.unwrap();
        repo.create(&main_task).await.unwrap();

        let retrieved = repo.get(main_task.id).await.unwrap().unwrap();
        assert!(retrieved.depends_on.contains(&dep_task.id));

        let deps = repo.get_dependencies(main_task.id).await.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].id, dep_task.id);
    }

    #[tokio::test]
    async fn test_ready_tasks() {
        let repo = setup_test_repo().await;

        let mut task1 = Task::with_title("Ready High", "Desc").with_priority(TaskPriority::High);
        task1.status = TaskStatus::Ready;

        let mut task2 = Task::with_title("Ready Low", "Desc").with_priority(TaskPriority::Low);
        task2.status = TaskStatus::Ready;

        repo.create(&task1).await.unwrap();
        repo.create(&task2).await.unwrap();

        let ready = repo.get_ready_tasks(10).await.unwrap();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].title, "Ready High"); // Higher priority first
    }
}

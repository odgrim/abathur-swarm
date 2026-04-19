//! SQLite implementation of the TaskRepository.

use crate::exec_tx;
use async_trait::async_trait;

/// Emit a warning when a serialized context JSON blob exceeds this size.
/// This is a signal that the hints cap may not be functioning or that
/// `custom` data is growing unexpectedly large.
const JSON_SIZE_WARN_BYTES: usize = 64 * 1024;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    ArtifactRef, ExecutionMode, RoutingHints, Task, TaskContext, TaskPriority, TaskSource,
    TaskStatus, TaskType,
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
        if context_json.len() > JSON_SIZE_WARN_BYTES {
            tracing::warn!(
                task_id = %task.id,
                size_bytes = context_json.len(),
                "context_json in create() exceeds size threshold; hints or custom data may be growing unboundedly"
            );
        }
        let (source_type, source_ref) = serialize_task_source(&task.source);
        let execution_mode_json = serde_json::to_string(&task.execution_mode)?;

        let create_q = sqlx::query(
            r#"INSERT INTO tasks (id, parent_id, title, description, status, priority,
               agent_type, routing, artifacts, context, retry_count, max_retries, worktree_path,
               idempotency_key, source_type, source_ref, version, created_at, updated_at, started_at, completed_at, deadline,
               execution_mode, trajectory_id, task_type)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
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
        .bind(task.deadline.map(|t| t.to_rfc3339()))
        .bind(&execution_mode_json)
        .bind(task.trajectory_id.map(|id| id.to_string()))
        .bind(task.task_type.as_str());
        exec_tx!(&self.pool, create_q, execute)?;

        // Add dependencies
        for dep_id in &task.depends_on {
            self.add_dependency(task.id, *dep_id).await?;
        }

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<Task>> {
        let get_q = sqlx::query_as("SELECT * FROM tasks WHERE id = ?").bind(id.to_string());
        let row: Option<TaskRow> = exec_tx!(&self.pool, get_q, fetch_optional)?;

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
        if context_json.len() > JSON_SIZE_WARN_BYTES {
            tracing::warn!(
                task_id = %task.id,
                size_bytes = context_json.len(),
                "context_json in update() exceeds size threshold; hints or custom data may be growing unboundedly"
            );
        }
        let (source_type, source_ref) = serialize_task_source(&task.source);
        let execution_mode_json = serde_json::to_string(&task.execution_mode)?;

        let update_q = sqlx::query(
            r#"UPDATE tasks SET parent_id = ?, title = ?, description = ?,
               status = ?, priority = ?, agent_type = ?, routing = ?, artifacts = ?,
               context = ?, retry_count = ?, max_retries = ?, worktree_path = ?,
               source_type = ?, source_ref = ?,
               version = ?, updated_at = ?, started_at = ?, completed_at = ?, deadline = ?,
               execution_mode = ?, trajectory_id = ?, task_type = ?
               WHERE id = ? AND version = ?"#,
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
        .bind(task.deadline.map(|t| t.to_rfc3339()))
        .bind(&execution_mode_json)
        .bind(task.trajectory_id.map(|id| id.to_string()))
        .bind(task.task_type.as_str())
        .bind(task.id.to_string())
        .bind(task.loaded_version.get() as i64);
        let result = exec_tx!(&self.pool, update_q, execute)?;

        if result.rows_affected() == 0 {
            // Distinguish between "task doesn't exist" and "version mismatch"
            let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM tasks WHERE id = ?")
                .bind(task.id.to_string())
                .fetch_optional(&self.pool)
                .await?;

            if exists.is_some() {
                return Err(DomainError::ConcurrencyConflict {
                    entity: "Task".to_string(),
                    id: task.id.to_string(),
                });
            } else {
                return Err(DomainError::TaskNotFound(task.id));
            }
        }

        // Sync loaded_version so subsequent updates on the same object succeed
        task.loaded_version.set(task.version);

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
        if let Some(task_type) = &filter.task_type {
            query.push_str(" AND task_type = ?");
            bindings.push(task_type.as_str().to_string());
        }
        if let Some(created_before) = &filter.created_before {
            query.push_str(" AND created_at < ?");
            bindings.push(created_before.to_rfc3339());
        }

        query.push_str(" ORDER BY created_at DESC");

        if let Some(n) = filter.limit {
            query.push_str(&format!(" LIMIT {}", n));
        }

        let mut q = sqlx::query_as::<_, TaskRow>(&query);
        for binding in &bindings {
            q = q.bind(binding);
        }

        let rows: Vec<TaskRow> = q.fetch_all(&self.pool).await?;
        let mut tasks: Vec<Task> = super::rows_into_lossy(rows, "tasks.list");
        for task in tasks.iter_mut() {
            self.load_dependencies(task).await?;
        }
        Ok(tasks)
    }

    async fn list_by_status(&self, status: TaskStatus) -> DomainResult<Vec<Task>> {
        self.list(TaskFilter {
            status: Some(status),
            ..Default::default()
        })
        .await
    }

    async fn get_subtasks(&self, parent_id: Uuid) -> DomainResult<Vec<Task>> {
        self.list(TaskFilter {
            parent_id: Some(parent_id),
            ..Default::default()
        })
        .await
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
               LIMIT ?"#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut tasks: Vec<Task> = super::rows_into_lossy(rows, "tasks.get_ready_tasks");
        for task in tasks.iter_mut() {
            self.load_dependencies(task).await?;
        }
        Ok(tasks)
    }

    async fn get_by_agent(&self, agent_type: &str) -> DomainResult<Vec<Task>> {
        self.list(TaskFilter {
            agent_type: Some(agent_type.to_string()),
            ..Default::default()
        })
        .await
    }

    async fn get_dependencies(&self, task_id: Uuid) -> DomainResult<Vec<Task>> {
        let deps_q = sqlx::query_as(
            r#"SELECT t.* FROM tasks t
               INNER JOIN task_dependencies d ON t.id = d.depends_on_id
               WHERE d.task_id = ?"#,
        )
        .bind(task_id.to_string());
        let rows: Vec<TaskRow> = exec_tx!(&self.pool, deps_q, fetch_all)?;

        Ok(super::rows_into_lossy(rows, "tasks.get_dependencies"))
    }

    async fn get_dependents(&self, task_id: Uuid) -> DomainResult<Vec<Task>> {
        let rows: Vec<TaskRow> = sqlx::query_as(
            r#"SELECT t.* FROM tasks t
               INNER JOIN task_dependencies d ON t.id = d.task_id
               WHERE d.depends_on_id = ?"#,
        )
        .bind(task_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        Ok(super::rows_into_lossy(rows, "tasks.get_dependents"))
    }

    async fn add_dependency(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<()> {
        // Self-dependency is always a cycle
        if task_id == depends_on {
            return Err(DomainError::DependencyCycle(task_id));
        }

        // Check if adding this edge would create a cycle.
        // A cycle exists if `depends_on` already transitively depends on `task_id`,
        // because adding "task_id depends on depends_on" would close the loop.
        if self.would_create_cycle(task_id, depends_on).await? {
            return Err(DomainError::DependencyCycle(task_id));
        }

        let dep_q = sqlx::query(
            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?, ?)",
        )
        .bind(task_id.to_string())
        .bind(depends_on.to_string());
        exec_tx!(&self.pool, dep_q, execute)?;
        Ok(())
    }

    async fn remove_dependency(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<()> {
        sqlx::query("DELETE FROM task_dependencies WHERE task_id = ? AND depends_on_id = ?")
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
            SELECT COUNT(*) FROM descendants"#,
        )
        .bind(task_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0 as u64)
    }

    async fn get_by_idempotency_key(&self, key: &str) -> DomainResult<Option<Task>> {
        let idem_q = sqlx::query_as("SELECT * FROM tasks WHERE idempotency_key = ?").bind(key);
        let row: Option<TaskRow> = exec_tx!(&self.pool, idem_q, fetch_optional)?;

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
        let rows: Vec<TaskRow> =
            sqlx::query_as("SELECT * FROM tasks WHERE source_type = ? ORDER BY created_at DESC")
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
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT status, COUNT(*) FROM tasks GROUP BY status")
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

    async fn claim_task_atomic(
        &self,
        task_id: Uuid,
        agent_type: &str,
    ) -> DomainResult<Option<Task>> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = sqlx::query(
            r#"UPDATE tasks
               SET status = 'running', agent_type = ?, version = version + 1,
                   updated_at = ?, started_at = ?
               WHERE id = ? AND status = 'ready'"#,
        )
        .bind(agent_type)
        .bind(&now)
        .bind(&now)
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get(task_id).await
    }

    async fn get_parent_id(&self, task_id: Uuid) -> DomainResult<Option<Uuid>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT parent_id FROM tasks WHERE id = ?")
                .bind(task_id.to_string())
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some((Some(parent_str),)) => {
                let uuid = Uuid::parse_str(&parent_str)
                    .map_err(|e| DomainError::SerializationError(e.to_string()))?;
                Ok(Some(uuid))
            }
            _ => Ok(None),
        }
    }

    async fn calculate_depth(&self, task_id: Uuid) -> DomainResult<u32> {
        let result: (i64,) = sqlx::query_as(
            r#"WITH RECURSIVE ancestors AS (
                SELECT parent_id, 0 AS depth FROM tasks WHERE id = ?
                UNION ALL
                SELECT t.parent_id, a.depth + 1
                FROM tasks t INNER JOIN ancestors a ON t.id = a.parent_id
                WHERE a.parent_id IS NOT NULL
            )
            SELECT COALESCE(MAX(depth), 0) FROM ancestors"#,
        )
        .bind(task_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0 as u32)
    }

    async fn find_root_task_id(&self, task_id: Uuid) -> DomainResult<Uuid> {
        let result: (String,) = sqlx::query_as(
            r#"WITH RECURSIVE ancestors AS (
                SELECT id, parent_id FROM tasks WHERE id = ?
                UNION ALL
                SELECT t.id, t.parent_id
                FROM tasks t INNER JOIN ancestors a ON t.id = a.parent_id
                WHERE a.parent_id IS NOT NULL
            )
            SELECT id FROM ancestors WHERE parent_id IS NULL LIMIT 1"#,
        )
        .bind(task_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        Uuid::parse_str(&result.0).map_err(|e| DomainError::SerializationError(e.to_string()))
    }

    async fn count_children(&self, task_id: Uuid) -> DomainResult<u32> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE parent_id = ?")
            .bind(task_id.to_string())
            .fetch_one(&self.pool)
            .await?;

        Ok(result.0 as u32)
    }
}

impl SqliteTaskRepository {
    /// Check whether adding edge `task_id -> depends_on` would create a cycle.
    ///
    /// Uses a recursive CTE to walk the transitive dependencies of `depends_on`.
    /// If `task_id` is reachable from `depends_on`, then the proposed edge would
    /// close a loop.
    async fn would_create_cycle(&self, task_id: Uuid, depends_on: Uuid) -> DomainResult<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"WITH RECURSIVE reachable(id) AS (
                SELECT ?1
                UNION
                SELECT td.depends_on_id
                FROM task_dependencies td
                INNER JOIN reachable r ON td.task_id = r.id
            )
            SELECT 1 FROM reachable WHERE id = ?2 LIMIT 1"#,
        )
        .bind(depends_on.to_string())
        .bind(task_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.is_some())
    }

    async fn load_dependencies(&self, task: &mut Task) -> DomainResult<()> {
        let load_deps_q =
            sqlx::query_as("SELECT depends_on_id FROM task_dependencies WHERE task_id = ?")
                .bind(task.id.to_string());
        let deps: Vec<(String,)> = exec_tx!(&self.pool, load_deps_q, fetch_all)?;

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
    deadline: Option<String>,
    execution_mode: Option<String>,
    trajectory_id: Option<String>,
    task_type: Option<String>,
}

impl TryFrom<TaskRow> for Task {
    type Error = DomainError;

    fn try_from(row: TaskRow) -> Result<Self, Self::Error> {
        let id = super::parse_uuid(&row.id)?;
        let parent_id = super::parse_optional_uuid(row.parent_id)?;

        let status = TaskStatus::from_str(&row.status).ok_or_else(|| {
            DomainError::SerializationError(format!("Invalid status: {}", row.status))
        })?;

        let priority = TaskPriority::from_str(&row.priority).ok_or_else(|| {
            DomainError::SerializationError(format!("Invalid priority: {}", row.priority))
        })?;

        let routing_hints: RoutingHints = super::parse_json_or_default(row.routing)?;
        let artifacts: Vec<ArtifactRef> = super::parse_json_or_default(row.artifacts)?;
        let context: TaskContext = super::parse_json_or_default(row.context)?;

        let created_at = super::parse_datetime(&row.created_at)?;
        let updated_at = super::parse_datetime(&row.updated_at)?;
        let started_at = super::parse_optional_datetime(row.started_at)?;
        let completed_at = super::parse_optional_datetime(row.completed_at)?;
        let deadline = super::parse_optional_datetime(row.deadline)?;

        let source =
            deserialize_task_source(row.source_type.as_deref(), row.source_ref.as_deref())?;

        let execution_mode: ExecutionMode = match row.execution_mode {
            Some(ref json) => serde_json::from_str(json).map_err(|e| {
                DomainError::SerializationError(format!("Invalid execution_mode: {}", e))
            })?,
            None => ExecutionMode::default(),
        };
        let trajectory_id = super::parse_optional_uuid(row.trajectory_id)?;
        let task_type = row
            .task_type
            .as_deref()
            .and_then(TaskType::from_str)
            .unwrap_or_default();

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
            deadline,
            version: row.version as u64,
            idempotency_key: row.idempotency_key,
            execution_mode,
            trajectory_id,
            task_type,
            loaded_version: crate::domain::models::VersionTag::new(row.version as u64),
        })
    }
}

/// Serialize a TaskSource into (source_type, source_ref) for DB storage.
fn serialize_task_source(source: &TaskSource) -> (String, Option<String>) {
    match source {
        TaskSource::Human => ("human".to_string(), None),
        TaskSource::System => ("system".to_string(), None),
        TaskSource::SubtaskOf(uuid) => ("subtask".to_string(), Some(uuid.to_string())),
        TaskSource::Schedule(uuid) => ("schedule".to_string(), Some(uuid.to_string())),
        TaskSource::Adapter(name) => ("adapter".to_string(), Some(name.clone())),
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
        Some("schedule") => {
            let uuid_str = source_ref.ok_or_else(|| {
                DomainError::SerializationError("schedule source requires source_ref".to_string())
            })?;
            let uuid = Uuid::parse_str(uuid_str)
                .map_err(|e| DomainError::SerializationError(e.to_string()))?;
            Ok(TaskSource::Schedule(uuid))
        }
        Some("adapter") => {
            let name = source_ref.unwrap_or("unknown").to_string();
            Ok(TaskSource::Adapter(name))
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
    use crate::adapters::sqlite::create_migrated_test_pool;

    async fn setup_test_repo() -> SqliteTaskRepository {
        let pool = create_migrated_test_pool().await.unwrap();
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

    #[tokio::test]
    async fn test_claim_task_atomic_success() {
        let repo = setup_test_repo().await;

        let mut task = Task::with_title("Claimable", "Desc");
        task.status = TaskStatus::Ready;
        repo.create(&task).await.unwrap();

        let claimed = repo.claim_task_atomic(task.id, "overmind").await.unwrap();
        assert!(claimed.is_some());

        let claimed = claimed.unwrap();
        assert_eq!(claimed.status, TaskStatus::Running);
        assert_eq!(claimed.agent_type.as_deref(), Some("overmind"));
        assert!(claimed.started_at.is_some());
    }

    #[tokio::test]
    async fn test_claim_task_atomic_double_claim() {
        let repo = setup_test_repo().await;

        let mut task = Task::with_title("Race me", "Desc");
        task.status = TaskStatus::Ready;
        repo.create(&task).await.unwrap();

        let first = repo.claim_task_atomic(task.id, "overmind").await.unwrap();
        assert!(first.is_some());

        // Second claim should return None (already Running)
        let second = repo.claim_task_atomic(task.id, "overmind").await.unwrap();
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_claim_task_atomic_non_ready() {
        let repo = setup_test_repo().await;

        // Default status is Pending, not Ready
        let task = Task::with_title("Pending task", "Desc");
        repo.create(&task).await.unwrap();

        let result = repo.claim_task_atomic(task.id, "overmind").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_update_with_correct_version_succeeds() {
        let repo = setup_test_repo().await;
        let task = Task::with_title("Version OK", "Desc");
        repo.create(&task).await.unwrap();

        // Fetch the task (loaded_version will match DB version)
        let mut fetched = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.loaded_version.get(), 1);

        // Transition bumps version; loaded_version stays at 1
        fetched.transition_to(TaskStatus::Ready).unwrap();
        assert_eq!(fetched.version, 2);
        assert_eq!(fetched.loaded_version.get(), 1);

        // Update should succeed because loaded_version matches DB
        repo.update(&fetched).await.unwrap();

        // Verify the DB was updated
        let refreshed = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(refreshed.version, 2);
        assert_eq!(refreshed.loaded_version.get(), 2);
        assert_eq!(refreshed.status, TaskStatus::Ready);
    }

    #[tokio::test]
    async fn test_update_with_stale_version_returns_concurrency_conflict() {
        let repo = setup_test_repo().await;
        let task = Task::with_title("Stale version", "Desc");
        repo.create(&task).await.unwrap();

        // Two readers fetch the same task
        let mut reader_a = repo.get(task.id).await.unwrap().unwrap();
        let mut reader_b = repo.get(task.id).await.unwrap().unwrap();

        // Reader A transitions and writes first
        reader_a.transition_to(TaskStatus::Ready).unwrap();
        repo.update(&reader_a).await.unwrap();

        // Reader B tries to write with stale loaded_version
        reader_b.transition_to(TaskStatus::Ready).unwrap();
        let result = repo.update(&reader_b).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, DomainError::ConcurrencyConflict { .. }),
            "Expected ConcurrencyConflict, got: {:?}",
            err,
        );
    }

    #[tokio::test]
    async fn test_update_nonexistent_task_returns_not_found() {
        let repo = setup_test_repo().await;
        let mut ghost = Task::with_title("Ghost", "Desc");
        ghost.id = Uuid::new_v4(); // never created
        ghost.transition_to(TaskStatus::Ready).unwrap();

        let result = repo.update(&ghost).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), DomainError::TaskNotFound(_)),
            "Expected TaskNotFound for nonexistent task",
        );
    }

    #[tokio::test]
    async fn test_metadata_update_without_transition_succeeds() {
        let repo = setup_test_repo().await;
        let task = Task::with_title("Metadata update", "Desc");
        repo.create(&task).await.unwrap();

        let mut fetched = repo.get(task.id).await.unwrap().unwrap();
        // Update metadata without transition (version stays the same)
        fetched.agent_type = Some("specialist".to_string());
        fetched.updated_at = chrono::Utc::now();

        repo.update(&fetched).await.unwrap();

        let refreshed = repo.get(task.id).await.unwrap().unwrap();
        assert_eq!(refreshed.agent_type.as_deref(), Some("specialist"));
    }

    #[tokio::test]
    async fn test_cycle_detection_self_dependency() {
        let repo = setup_test_repo().await;
        let task = Task::with_title("Self", "Desc");
        repo.create(&task).await.unwrap();

        let result = repo.add_dependency(task.id, task.id).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), DomainError::DependencyCycle(_)),
            "Expected DependencyCycle for self-dependency"
        );
    }

    #[tokio::test]
    async fn test_cycle_detection_direct_ab_ba() {
        let repo = setup_test_repo().await;

        let a = Task::with_title("A", "Desc");
        let b = Task::with_title("B", "Desc");
        repo.create(&a).await.unwrap();
        repo.create(&b).await.unwrap();

        // A depends on B — should succeed
        repo.add_dependency(a.id, b.id).await.unwrap();

        // B depends on A — would create cycle A->B->A
        let result = repo.add_dependency(b.id, a.id).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), DomainError::DependencyCycle(_)),
            "Expected DependencyCycle for direct A->B->A cycle"
        );
    }

    #[tokio::test]
    async fn test_cycle_detection_transitive_abc() {
        let repo = setup_test_repo().await;

        let a = Task::with_title("A", "Desc");
        let b = Task::with_title("B", "Desc");
        let c = Task::with_title("C", "Desc");
        repo.create(&a).await.unwrap();
        repo.create(&b).await.unwrap();
        repo.create(&c).await.unwrap();

        // A depends on B, B depends on C — both fine
        repo.add_dependency(a.id, b.id).await.unwrap();
        repo.add_dependency(b.id, c.id).await.unwrap();

        // C depends on A — would create cycle A->B->C->A
        let result = repo.add_dependency(c.id, a.id).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), DomainError::DependencyCycle(_)),
            "Expected DependencyCycle for transitive A->B->C->A cycle"
        );
    }

    #[tokio::test]
    async fn test_cycle_detection_allows_valid_dag() {
        let repo = setup_test_repo().await;

        let a = Task::with_title("A", "Desc");
        let b = Task::with_title("B", "Desc");
        let c = Task::with_title("C", "Desc");
        repo.create(&a).await.unwrap();
        repo.create(&b).await.unwrap();
        repo.create(&c).await.unwrap();

        // Diamond: A depends on B, A depends on C, B depends on C — valid DAG
        repo.add_dependency(a.id, b.id).await.unwrap();
        repo.add_dependency(a.id, c.id).await.unwrap();
        repo.add_dependency(b.id, c.id).await.unwrap();
    }

    #[tokio::test]
    async fn test_cycle_detection_at_create_time() {
        let repo = setup_test_repo().await;

        let a = Task::with_title("A", "Desc");
        repo.create(&a).await.unwrap();

        // Create B with a dependency on A
        let b = Task::with_title("B", "Desc").with_dependency(a.id);
        repo.create(&b).await.unwrap();

        // Now try to add A depends on B — would create cycle
        let result = repo.add_dependency(a.id, b.id).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), DomainError::DependencyCycle(_)),
            "Expected DependencyCycle when adding reverse dependency"
        );
    }

    #[tokio::test]
    async fn test_claim_task_atomic_increments_version() {
        let repo = setup_test_repo().await;

        let mut task = Task::with_title("Version check", "Desc");
        task.status = TaskStatus::Ready;
        let original_version = task.version;
        repo.create(&task).await.unwrap();

        let claimed = repo
            .claim_task_atomic(task.id, "overmind")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.version, original_version + 1);
    }

    #[tokio::test]
    async fn test_calculate_depth_root_task() {
        let repo = setup_test_repo().await;
        let root = Task::with_title("Root", "Desc");
        repo.create(&root).await.unwrap();

        let depth = repo.calculate_depth(root.id).await.unwrap();
        assert_eq!(depth, 0);
    }

    #[tokio::test]
    async fn test_calculate_depth_nested() {
        let repo = setup_test_repo().await;

        let root = Task::with_title("Root", "Desc");
        let child = Task::with_title("Child", "Desc").with_parent(root.id);
        let grandchild = Task::with_title("Grandchild", "Desc").with_parent(child.id);

        repo.create(&root).await.unwrap();
        repo.create(&child).await.unwrap();
        repo.create(&grandchild).await.unwrap();

        assert_eq!(repo.calculate_depth(root.id).await.unwrap(), 0);
        assert_eq!(repo.calculate_depth(child.id).await.unwrap(), 1);
        assert_eq!(repo.calculate_depth(grandchild.id).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_find_root_task_id_root() {
        let repo = setup_test_repo().await;
        let root = Task::with_title("Root", "Desc");
        repo.create(&root).await.unwrap();

        let found = repo.find_root_task_id(root.id).await.unwrap();
        assert_eq!(found, root.id);
    }

    #[tokio::test]
    async fn test_find_root_task_id_nested() {
        let repo = setup_test_repo().await;

        let root = Task::with_title("Root", "Desc");
        let child = Task::with_title("Child", "Desc").with_parent(root.id);
        let grandchild = Task::with_title("Grandchild", "Desc").with_parent(child.id);

        repo.create(&root).await.unwrap();
        repo.create(&child).await.unwrap();
        repo.create(&grandchild).await.unwrap();

        assert_eq!(
            repo.find_root_task_id(grandchild.id).await.unwrap(),
            root.id
        );
        assert_eq!(repo.find_root_task_id(child.id).await.unwrap(), root.id);
    }

    #[tokio::test]
    async fn test_count_children() {
        let repo = setup_test_repo().await;

        let parent = Task::with_title("Parent", "Desc");
        let child1 = Task::with_title("Child 1", "Desc").with_parent(parent.id);
        let child2 = Task::with_title("Child 2", "Desc").with_parent(parent.id);
        let grandchild = Task::with_title("Grandchild", "Desc").with_parent(child1.id);

        repo.create(&parent).await.unwrap();
        repo.create(&child1).await.unwrap();
        repo.create(&child2).await.unwrap();
        repo.create(&grandchild).await.unwrap();

        // Direct children only, not grandchildren
        assert_eq!(repo.count_children(parent.id).await.unwrap(), 2);
        assert_eq!(repo.count_children(child1.id).await.unwrap(), 1);
        assert_eq!(repo.count_children(child2.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_count_descendants_recursive() {
        let repo = setup_test_repo().await;

        let root = Task::with_title("Root", "Desc");
        let child1 = Task::with_title("Child 1", "Desc").with_parent(root.id);
        let child2 = Task::with_title("Child 2", "Desc").with_parent(root.id);
        let grandchild = Task::with_title("Grandchild", "Desc").with_parent(child1.id);

        repo.create(&root).await.unwrap();
        repo.create(&child1).await.unwrap();
        repo.create(&child2).await.unwrap();
        repo.create(&grandchild).await.unwrap();

        // All descendants: child1, child2, grandchild = 3
        assert_eq!(repo.count_descendants(root.id).await.unwrap(), 3);
        // child1 has one descendant: grandchild
        assert_eq!(repo.count_descendants(child1.id).await.unwrap(), 1);
        // Leaf has no descendants
        assert_eq!(repo.count_descendants(grandchild.id).await.unwrap(), 0);
    }
}

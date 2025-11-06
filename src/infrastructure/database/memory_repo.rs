use crate::domain::models::{Memory, MemoryType};
use crate::domain::ports::MemoryRepository;
use crate::infrastructure::database::utils::parse_datetime;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;

/// SQLite implementation of MemoryRepository
///
/// Provides async database operations for memory storage with:
/// - Compile-time checked queries using sqlx::query! macro
/// - JSON serialization for value and metadata fields
/// - Soft deletes (is_deleted flag)
/// - Namespace prefix searching with LIKE queries
pub struct MemoryRepositoryImpl {
    pool: SqlitePool,
}

impl MemoryRepositoryImpl {
    /// Create a new MemoryRepositoryImpl
    ///
    /// # Arguments
    /// * `pool` - SQLite connection pool
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MemoryRepository for MemoryRepositoryImpl {
    async fn insert(&self, memory: Memory) -> Result<i64> {
        let value_json = serde_json::to_string(&memory.value).context("failed to serialize value")?;
        let memory_type_str = memory.memory_type.to_string();
        let metadata_json = memory
            .metadata
            .as_ref()
            .and_then(|m| serde_json::to_string(m).ok());
        let created_at_str = memory.created_at.to_rfc3339();
        let updated_at_str = memory.updated_at.to_rfc3339();
        let is_deleted_i64 = memory.is_deleted as i64;

        let result = sqlx::query!(
            r#"
            INSERT INTO memories (
                namespace, key, value, memory_type, is_deleted,
                metadata, created_by, updated_by, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            memory.namespace,
            memory.key,
            value_json,
            memory_type_str,
            is_deleted_i64,
            metadata_json,
            memory.created_by,
            memory.updated_by,
            created_at_str,
            updated_at_str
        )
        .execute(&self.pool)
        .await
        .context("failed to insert memory")?;

        Ok(result.last_insert_rowid())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<Memory>> {
        let row = sqlx::query!(
            r#"
            SELECT id, namespace, key, value, memory_type, is_deleted,
                   metadata, created_by, updated_by, created_at, updated_at
            FROM memories
            WHERE namespace = ? AND key = ? AND is_deleted = 0
            "#,
            namespace,
            key
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to query memory")?;

        match row {
            Some(r) => {
                let memory = Memory {
                    id: r.id.context("missing id from database")?,
                    namespace: r.namespace,
                    key: r.key,
                    value: serde_json::from_str(&r.value)
                        .context("failed to deserialize value")?,
                    memory_type: r
                        .memory_type
                        .parse()
                        .context("failed to parse memory_type")?,
                    is_deleted: r.is_deleted != 0,
                    metadata: r
                        .metadata
                        .as_ref()
                        .and_then(|m| serde_json::from_str(m).ok()),
                    created_by: r.created_by,
                    updated_by: r.updated_by,
                    created_at: parse_datetime(&r.created_at)
                        .context("failed to parse created_at")?,
                    updated_at: parse_datetime(&r.updated_at)
                        .context("failed to parse updated_at")?,
                };
                Ok(Some(memory))
            }
            None => Ok(None),
        }
    }

    async fn search(
        &self,
        namespace_prefix: &str,
        memory_type: Option<MemoryType>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        // Build LIKE pattern for namespace prefix matching
        let namespace_pattern = format!("{}%", namespace_prefix);
        let limit_i64 = limit as i64;

        // Use dynamic query since we have conditional parameters
        let rows: Vec<_> = if let Some(mem_type) = memory_type {
            let memory_type_str = mem_type.to_string();
            sqlx::query_as::<_, (i64, String, String, String, String, i64, Option<String>, String, String, String, String)>(
                r#"
                SELECT id, namespace, key, value, memory_type, is_deleted,
                       metadata, created_by, updated_by, created_at, updated_at
                FROM memories
                WHERE namespace LIKE ? AND memory_type = ? AND is_deleted = 0
                ORDER BY namespace, key
                LIMIT ?
                "#
            )
            .bind(&namespace_pattern)
            .bind(&memory_type_str)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await
            .context("failed to search memories with type filter")?
        } else {
            sqlx::query_as::<_, (i64, String, String, String, String, i64, Option<String>, String, String, String, String)>(
                r#"
                SELECT id, namespace, key, value, memory_type, is_deleted,
                       metadata, created_by, updated_by, created_at, updated_at
                FROM memories
                WHERE namespace LIKE ? AND is_deleted = 0
                ORDER BY namespace, key
                LIMIT ?
                "#
            )
            .bind(&namespace_pattern)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await
            .context("failed to search memories")?
        };

        let memories: Result<Vec<Memory>> = rows
            .into_iter()
            .map(|(id, namespace, key, value, memory_type, is_deleted, metadata, created_by, updated_by, created_at, updated_at)| {
                Ok(Memory {
                    id,
                    namespace,
                    key,
                    value: serde_json::from_str(&value)
                        .context("failed to deserialize value")?,
                    memory_type: memory_type
                        .parse()
                        .context("failed to parse memory_type")?,
                    is_deleted: is_deleted != 0,
                    metadata: metadata
                        .as_ref()
                        .and_then(|m| serde_json::from_str(m).ok()),
                    created_by,
                    updated_by,
                    created_at: parse_datetime(&created_at)
                        .context("failed to parse created_at")?,
                    updated_at: parse_datetime(&updated_at)
                        .context("failed to parse updated_at")?,
                })
            })
            .collect();

        memories
    }

    async fn update(
        &self,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
        updated_by: &str,
    ) -> Result<()> {
        let now = Utc::now();
        let value_str =
            serde_json::to_string(&value).context("failed to serialize value")?;
        let updated_at_str = now.to_rfc3339();

        let result = sqlx::query!(
            r#"
            UPDATE memories
            SET value = ?,
                updated_by = ?,
                updated_at = ?
            WHERE namespace = ? AND key = ? AND is_deleted = 0
            "#,
            value_str,
            updated_by,
            updated_at_str,
            namespace,
            key
        )
        .execute(&self.pool)
        .await
        .context("failed to update memory")?;

        if result.rows_affected() == 0 {
            anyhow::bail!(
                "memory not found or already deleted: namespace={}, key={}",
                namespace,
                key
            );
        }

        Ok(())
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<()> {
        let result = sqlx::query!(
            r#"
            UPDATE memories
            SET is_deleted = 1
            WHERE namespace = ? AND key = ?
            "#,
            namespace,
            key
        )
        .execute(&self.pool)
        .await
        .context("failed to soft-delete memory")?;

        if result.rows_affected() == 0 {
            anyhow::bail!("memory not found: namespace={}, key={}", namespace, key);
        }

        Ok(())
    }


    async fn count(&self, namespace_prefix: &str, memory_type: Option<MemoryType>) -> Result<usize> {
        let namespace_pattern = format!("{}%", namespace_prefix);

        let count: i64 = if let Some(mem_type) = memory_type {
            let memory_type_str = mem_type.to_string();
            sqlx::query_scalar(
                r#"
                SELECT COUNT(*) FROM memories
                WHERE namespace LIKE ? AND memory_type = ? AND is_deleted = 0
                "#
            )
            .bind(&namespace_pattern)
            .bind(&memory_type_str)
            .fetch_one(&self.pool)
            .await
            .context("failed to count memories with type filter")?
        } else {
            sqlx::query_scalar(
                r#"
                SELECT COUNT(*) FROM memories
                WHERE namespace LIKE ? AND is_deleted = 0
                "#
            )
            .bind(&namespace_pattern)
            .fetch_one(&self.pool)
            .await
            .context("failed to count memories")?
        };

        Ok(count as usize)
    }

}

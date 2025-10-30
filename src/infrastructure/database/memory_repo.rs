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


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("failed to create test database");

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    #[tokio::test]
    async fn test_insert_and_get_memory() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        let memory = Memory::new(
            "user:alice:preferences".to_string(),
            "theme".to_string(),
            json!({"color": "dark"}),
            MemoryType::Semantic,
            "alice".to_string(),
        );

        repo.insert(memory.clone())
            .await
            .expect("failed to insert memory");

        let retrieved = repo
            .get("user:alice:preferences", "theme")
            .await
            .expect("failed to get memory");

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.namespace, "user:alice:preferences");
        assert_eq!(retrieved.key, "theme");
        assert_eq!(retrieved.value, json!({"color": "dark"}));
        assert_eq!(retrieved.memory_type, MemoryType::Semantic);
        assert!(!retrieved.is_deleted);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_update_memory() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        let memory = Memory::new(
            "user:bob:settings".to_string(),
            "language".to_string(),
            json!("en"),
            MemoryType::Semantic,
            "bob".to_string(),
        );

        repo.insert(memory).await.expect("failed to insert memory");

        repo.update(
            "user:bob:settings",
            "language",
            json!("fr"),
            "bob",
        )
        .await
        .expect("failed to update memory");

        let updated = repo
            .get("user:bob:settings", "language")
            .await
            .expect("failed to get memory")
            .unwrap();

        assert_eq!(updated.value, json!("fr"));
        assert_eq!(updated.updated_by, "bob");

        pool.close().await;
    }

    #[tokio::test]
    async fn test_search_by_namespace_prefix() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        // Add multiple memories under same namespace prefix
        let memories = vec![
            Memory::new(
                "user:alice:prefs:theme".to_string(),
                "color".to_string(),
                json!("dark"),
                MemoryType::Semantic,
                "alice".to_string(),
            ),
            Memory::new(
                "user:alice:prefs:font".to_string(),
                "size".to_string(),
                json!(14),
                MemoryType::Semantic,
                "alice".to_string(),
            ),
            Memory::new(
                "user:bob:prefs:theme".to_string(),
                "color".to_string(),
                json!("light"),
                MemoryType::Semantic,
                "bob".to_string(),
            ),
        ];

        for memory in memories {
            repo.insert(memory).await.expect("failed to insert memory");
        }

        let results = repo
            .search("user:alice", None, 100)
            .await
            .expect("failed to search memories");

        assert_eq!(results.len(), 2); // Only Alice's memories

        pool.close().await;
    }

    #[tokio::test]
    async fn test_search_with_memory_type_filter() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        // Add memories with different types
        let semantic = Memory::new(
            "test:data".to_string(),
            "fact".to_string(),
            json!("value"),
            MemoryType::Semantic,
            "test".to_string(),
        );

        let episodic = Memory::new(
            "test:data".to_string(),
            "event".to_string(),
            json!("value"),
            MemoryType::Episodic,
            "test".to_string(),
        );

        repo.insert(semantic).await.expect("failed to insert semantic");
        repo.insert(episodic).await.expect("failed to insert episodic");

        let results = repo
            .search("test:", Some(MemoryType::Semantic), 100)
            .await
            .expect("failed to search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].memory_type, MemoryType::Semantic);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_soft_delete() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        let memory = Memory::new(
            "test:ns".to_string(),
            "key".to_string(),
            json!({}),
            MemoryType::Semantic,
            "test".to_string(),
        );

        repo.insert(memory).await.expect("failed to insert memory");

        // Soft delete
        repo.delete("test:ns", "key")
            .await
            .expect("failed to delete memory");

        // Should not be returned by get()
        let result = repo
            .get("test:ns", "key")
            .await
            .expect("failed to get memory");
        assert!(result.is_none());

        // Verify row still exists in database
        let row = sqlx::query!(
            "SELECT is_deleted FROM memories WHERE namespace = ? AND key = ?",
            "test:ns",
            "key"
        )
        .fetch_optional(&pool)
        .await
        .expect("failed to query");

        assert!(row.is_some());
        assert_eq!(row.unwrap().is_deleted, 1);

        pool.close().await;
    }

    #[tokio::test]
    async fn test_update_deleted_memory_fails() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        let memory = Memory::new(
            "test:ns".to_string(),
            "key".to_string(),
            json!({}),
            MemoryType::Semantic,
            "test".to_string(),
        );

        repo.insert(memory).await.expect("failed to insert memory");
        repo.delete("test:ns", "key")
            .await
            .expect("failed to delete");

        // Try to update deleted memory
        let result = repo
            .update("test:ns", "key", json!({"new": "value"}), "test")
            .await;

        assert!(result.is_err());

        pool.close().await;
    }

    #[tokio::test]
    async fn test_unique_namespace_key_constraint() {
        let pool = setup_test_db().await;
        let repo = MemoryRepositoryImpl::new(pool.clone());

        let memory1 = Memory::new(
            "test:ns".to_string(),
            "key".to_string(),
            json!({}),
            MemoryType::Semantic,
            "test".to_string(),
        );

        let memory2 = Memory::new(
            "test:ns".to_string(),
            "key".to_string(),
            json!({}),
            MemoryType::Semantic,
            "test".to_string(),
        );

        repo.insert(memory1).await.expect("failed to insert first memory");

        // Second insert with same namespace+key should fail
        let result = repo.insert(memory2).await;
        assert!(result.is_err());

        pool.close().await;
    }
}

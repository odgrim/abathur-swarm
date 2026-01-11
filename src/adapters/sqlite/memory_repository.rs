//! SQLite implementation of the MemoryRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Memory, MemoryMetadata, MemoryQuery, MemoryTier, MemoryType};
use crate::domain::ports::MemoryRepository;

pub struct SqliteMemoryRepository {
    pool: SqlitePool,
}

impl SqliteMemoryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MemoryRepository for SqliteMemoryRepository {
    async fn store(&self, memory: &Memory) -> DomainResult<()> {
        let metadata_json = serde_json::to_string(&memory.metadata)?;

        sqlx::query(
            r#"INSERT INTO memories (id, namespace, key, content, value, memory_type, tier, metadata,
               access_count, version, created_at, updated_at, last_accessed_at, expires_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
        )
        .bind(memory.id.to_string())
        .bind(&memory.namespace)
        .bind(&memory.key)
        .bind(&memory.content)
        .bind(&memory.content) // Also set 'value' for backwards compat
        .bind(memory.memory_type.as_str())
        .bind(memory.tier.as_str())
        .bind(&metadata_json)
        .bind(memory.access_count as i32)
        .bind(memory.version as i64)
        .bind(memory.created_at.to_rfc3339())
        .bind(memory.updated_at.to_rfc3339())
        .bind(memory.last_accessed.to_rfc3339())
        .bind(memory.expires_at.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await?;

        // Update FTS index
        sqlx::query(
            "INSERT INTO memories_fts (memory_id, key, value, namespace) VALUES (?, ?, ?, ?)"
        )
        .bind(memory.id.to_string())
        .bind(&memory.key)
        .bind(&memory.content)
        .bind(&memory.namespace)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get(&self, id: Uuid) -> DomainResult<Option<Memory>> {
        let row: Option<MemoryRow> = sqlx::query_as(
            "SELECT * FROM memories WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_by_key(&self, key: &str, namespace: &str) -> DomainResult<Option<Memory>> {
        let row: Option<MemoryRow> = sqlx::query_as(
            "SELECT * FROM memories WHERE key = ? AND namespace = ? ORDER BY version DESC LIMIT 1"
        )
        .bind(key)
        .bind(namespace)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn update(&self, memory: &Memory) -> DomainResult<()> {
        let metadata_json = serde_json::to_string(&memory.metadata)?;

        let result = sqlx::query(
            r#"UPDATE memories SET namespace = ?, key = ?, content = ?, value = ?,
               memory_type = ?, tier = ?, metadata = ?, access_count = ?,
               version = ?, updated_at = ?, last_accessed_at = ?, expires_at = ?
               WHERE id = ?"#
        )
        .bind(&memory.namespace)
        .bind(&memory.key)
        .bind(&memory.content)
        .bind(&memory.content)
        .bind(memory.memory_type.as_str())
        .bind(memory.tier.as_str())
        .bind(&metadata_json)
        .bind(memory.access_count as i32)
        .bind(memory.version as i64)
        .bind(memory.updated_at.to_rfc3339())
        .bind(memory.last_accessed.to_rfc3339())
        .bind(memory.expires_at.map(|t| t.to_rfc3339()))
        .bind(memory.id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::MemoryNotFound(memory.id));
        }

        // Update FTS index
        sqlx::query("DELETE FROM memories_fts WHERE memory_id = ?")
            .bind(memory.id.to_string())
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "INSERT INTO memories_fts (memory_id, key, value, namespace) VALUES (?, ?, ?, ?)"
        )
        .bind(memory.id.to_string())
        .bind(&memory.key)
        .bind(&memory.content)
        .bind(&memory.namespace)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> DomainResult<()> {
        // Delete from FTS first
        sqlx::query("DELETE FROM memories_fts WHERE memory_id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        let result = sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::MemoryNotFound(id));
        }

        Ok(())
    }

    async fn query(&self, query: MemoryQuery) -> DomainResult<Vec<Memory>> {
        let mut sql = String::from("SELECT * FROM memories WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(ns) = &query.namespace {
            sql.push_str(" AND namespace = ?");
            bindings.push(ns.clone());
        }
        if let Some(tier) = &query.tier {
            sql.push_str(" AND tier = ?");
            bindings.push(tier.as_str().to_string());
        }
        if let Some(mtype) = &query.memory_type {
            sql.push_str(" AND memory_type = ?");
            bindings.push(mtype.as_str().to_string());
        }
        if let Some(pattern) = &query.key_pattern {
            sql.push_str(" AND key LIKE ?");
            bindings.push(pattern.replace('*', "%"));
        }

        sql.push_str(" ORDER BY last_accessed_at DESC");

        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        let mut q = sqlx::query_as::<_, MemoryRow>(&sql);
        for binding in &bindings {
            q = q.bind(binding);
        }

        let rows: Vec<MemoryRow> = q.fetch_all(&self.pool).await?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn search(&self, query: &str, namespace: Option<&str>, limit: usize) -> DomainResult<Vec<Memory>> {
        let sql = if namespace.is_some() {
            r#"SELECT m.* FROM memories m
               INNER JOIN memories_fts f ON m.id = f.memory_id
               WHERE memories_fts MATCH ? AND m.namespace = ?
               ORDER BY rank
               LIMIT ?"#
        } else {
            r#"SELECT m.* FROM memories m
               INNER JOIN memories_fts f ON m.id = f.memory_id
               WHERE memories_fts MATCH ?
               ORDER BY rank
               LIMIT ?"#
        };

        let rows: Vec<MemoryRow> = if let Some(ns) = namespace {
            sqlx::query_as(sql)
                .bind(query)
                .bind(ns)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query_as(&sql.replace(" AND m.namespace = ?", ""))
                .bind(query)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_by_tier(&self, tier: MemoryTier) -> DomainResult<Vec<Memory>> {
        let rows: Vec<MemoryRow> = sqlx::query_as(
            "SELECT * FROM memories WHERE tier = ? ORDER BY last_accessed_at DESC"
        )
        .bind(tier.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_by_namespace(&self, namespace: &str) -> DomainResult<Vec<Memory>> {
        let rows: Vec<MemoryRow> = sqlx::query_as(
            "SELECT * FROM memories WHERE namespace = ? ORDER BY last_accessed_at DESC"
        )
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_expired(&self) -> DomainResult<Vec<Memory>> {
        let now = chrono::Utc::now().to_rfc3339();
        let rows: Vec<MemoryRow> = sqlx::query_as(
            "SELECT * FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?"
        )
        .bind(&now)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn prune_expired(&self) -> DomainResult<u64> {
        let now = chrono::Utc::now().to_rfc3339();

        // Delete from FTS first
        sqlx::query(
            r#"DELETE FROM memories_fts WHERE memory_id IN
               (SELECT id FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?)"#
        )
        .bind(&now)
        .execute(&self.pool)
        .await?;

        // Delete from main table
        let result = sqlx::query(
            "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?"
        )
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    async fn get_decayed(&self, threshold: f32) -> DomainResult<Vec<Memory>> {
        // We can't compute decay in SQL easily, so fetch all and filter
        let rows: Vec<MemoryRow> = sqlx::query_as(
            "SELECT * FROM memories WHERE tier != 'semantic'"
        )
        .fetch_all(&self.pool)
        .await?;

        let memories: Vec<Memory> = rows.into_iter()
            .filter_map(|r| r.try_into().ok())
            .filter(|m: &Memory| m.decay_factor() < threshold)
            .collect();

        Ok(memories)
    }

    async fn get_for_task(&self, task_id: Uuid) -> DomainResult<Vec<Memory>> {
        let task_id_str = task_id.to_string();
        let rows: Vec<MemoryRow> = sqlx::query_as(
            r#"SELECT * FROM memories
               WHERE metadata LIKE ?
               ORDER BY last_accessed_at DESC"#
        )
        .bind(format!("%{}%", task_id_str))
        .fetch_all(&self.pool)
        .await?;

        // Filter to only those actually matching task_id in metadata
        let memories: Vec<Memory> = rows.into_iter()
            .filter_map(|r| r.try_into().ok())
            .filter(|m: &Memory| m.metadata.task_id == Some(task_id))
            .collect();

        Ok(memories)
    }

    async fn get_for_goal(&self, goal_id: Uuid) -> DomainResult<Vec<Memory>> {
        let goal_id_str = goal_id.to_string();
        let rows: Vec<MemoryRow> = sqlx::query_as(
            r#"SELECT * FROM memories
               WHERE metadata LIKE ?
               ORDER BY last_accessed_at DESC"#
        )
        .bind(format!("%{}%", goal_id_str))
        .fetch_all(&self.pool)
        .await?;

        let memories: Vec<Memory> = rows.into_iter()
            .filter_map(|r| r.try_into().ok())
            .filter(|m: &Memory| m.metadata.goal_id == Some(goal_id))
            .collect();

        Ok(memories)
    }

    async fn count_by_tier(&self) -> DomainResult<HashMap<MemoryTier, u64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT tier, COUNT(*) FROM memories GROUP BY tier"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut counts = HashMap::new();
        for (tier_str, count) in rows {
            if let Some(tier) = MemoryTier::from_str(&tier_str) {
                counts.insert(tier, count as u64);
            }
        }
        Ok(counts)
    }
}

#[derive(sqlx::FromRow)]
struct MemoryRow {
    id: String,
    namespace: String,
    key: String,
    content: Option<String>,
    value: String,
    memory_type: String,
    tier: Option<String>,
    metadata: Option<String>,
    access_count: i32,
    version: i64,
    created_at: String,
    updated_at: String,
    last_accessed_at: String,
    expires_at: Option<String>,
}

impl TryFrom<MemoryRow> for Memory {
    type Error = DomainError;

    fn try_from(row: MemoryRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        let tier = row.tier
            .as_deref()
            .and_then(MemoryTier::from_str)
            .unwrap_or(MemoryTier::Working);

        let memory_type = MemoryType::from_str(&row.memory_type)
            .unwrap_or(MemoryType::Fact);

        let metadata: MemoryMetadata = row.metadata
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .unwrap_or_default();

        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        let updated_at = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        let last_accessed = chrono::DateTime::parse_from_rfc3339(&row.last_accessed_at)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?
            .with_timezone(&chrono::Utc);

        let expires_at = row.expires_at
            .map(|s| chrono::DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;

        // Use content if available, fall back to value
        let content = row.content.unwrap_or(row.value);

        Ok(Memory {
            id,
            key: row.key,
            namespace: row.namespace,
            content,
            tier,
            memory_type,
            metadata,
            access_count: row.access_count as u32,
            last_accessed,
            created_at,
            updated_at,
            expires_at,
            version: row.version as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, Migrator, all_embedded_migrations};

    async fn setup_test_repo() -> SqliteMemoryRepository {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
        SqliteMemoryRepository::new(pool)
    }

    #[tokio::test]
    async fn test_store_and_get_memory() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("test_key", "test content")
            .with_namespace("test");

        repo.store(&memory).await.unwrap();

        let retrieved = repo.get(memory.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().key, "test_key");
    }

    #[tokio::test]
    async fn test_get_by_key() {
        let repo = setup_test_repo().await;

        let memory = Memory::episodic("lookup_key", "some value")
            .with_namespace("test");

        repo.store(&memory).await.unwrap();

        let retrieved = repo.get_by_key("lookup_key", "test").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "some value");
    }

    #[tokio::test]
    async fn test_list_by_tier() {
        let repo = setup_test_repo().await;

        let working = Memory::working("w1", "working memory");
        let semantic = Memory::semantic("s1", "semantic memory");

        repo.store(&working).await.unwrap();
        repo.store(&semantic).await.unwrap();

        let working_memories = repo.list_by_tier(MemoryTier::Working).await.unwrap();
        assert_eq!(working_memories.len(), 1);
        assert_eq!(working_memories[0].key, "w1");

        let semantic_memories = repo.list_by_tier(MemoryTier::Semantic).await.unwrap();
        assert_eq!(semantic_memories.len(), 1);
        assert_eq!(semantic_memories[0].key, "s1");
    }

    #[tokio::test]
    async fn test_count_by_tier() {
        let repo = setup_test_repo().await;

        repo.store(&Memory::working("w1", "content")).await.unwrap();
        repo.store(&Memory::working("w2", "content")).await.unwrap();
        repo.store(&Memory::semantic("s1", "content")).await.unwrap();

        let counts = repo.count_by_tier().await.unwrap();
        assert_eq!(*counts.get(&MemoryTier::Working).unwrap_or(&0), 2);
        assert_eq!(*counts.get(&MemoryTier::Semantic).unwrap_or(&0), 1);
    }
}

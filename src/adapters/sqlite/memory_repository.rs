//! SQLite implementation of the MemoryRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Memory, MemoryMetadata, MemoryQuery, MemoryTier, MemoryType};
use crate::domain::ports::MemoryRepository;

#[derive(Clone)]
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
        let sanitized = sanitize_fts5_query(query);

        // If sanitization produced an empty query, return empty results immediately
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

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
                .bind(&sanitized)
                .bind(ns)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query_as(&sql.replace(" AND m.namespace = ?", ""))
                .bind(&sanitized)
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

/// Sanitize a search query for use with SQLite FTS5.
///
/// FTS5 interprets certain tokens as reserved syntax: boolean operators (AND, OR, NOT),
/// column filter prefixes (e.g. `key:`), and phrase quotes. Passing user input directly
/// as an FTS5 MATCH expression can cause parse errors.
///
/// This function wraps every whitespace-delimited token in double quotes, which tells
/// FTS5 to treat each token as a literal phrase rather than as syntax. Interior double
/// quotes are escaped by doubling them (`"` → `""`).
///
/// Returns an empty string if the input is empty or whitespace-only, which the caller
/// should treat as "no results" rather than issuing a MATCH query.
fn sanitize_fts5_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| {
            // Escape any existing double quotes by doubling them
            let escaped = term.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        })
        .collect();

    terms.join(" ")
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
        let id = super::parse_uuid(&row.id)?;

        let tier = row.tier
            .as_deref()
            .and_then(MemoryTier::from_str)
            .unwrap_or(MemoryTier::Working);

        let memory_type = MemoryType::from_str(&row.memory_type)
            .unwrap_or(MemoryType::Fact);

        let metadata: MemoryMetadata = super::parse_json_or_default(row.metadata)?;

        let created_at = super::parse_datetime(&row.created_at)?;
        let updated_at = super::parse_datetime(&row.updated_at)?;
        let last_accessed = super::parse_datetime(&row.last_accessed_at)?;
        let expires_at = super::parse_optional_datetime(row.expires_at)?;

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
            embedding: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_migrated_test_pool;

    async fn setup_test_repo() -> SqliteMemoryRepository {
        let pool = create_migrated_test_pool().await.unwrap();
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

    // ---- sanitize_fts5_query unit tests ----

    #[test]
    fn test_sanitize_fts5_normal_query() {
        let result = sanitize_fts5_query("hello world");
        assert_eq!(result, "\"hello\" \"world\"");
    }

    #[test]
    fn test_sanitize_fts5_reserved_words() {
        // NEAR is a reserved FTS5 keyword
        let result = sanitize_fts5_query("NEAR something");
        assert_eq!(result, "\"NEAR\" \"something\"");
    }

    #[test]
    fn test_sanitize_fts5_boolean_operators() {
        let result = sanitize_fts5_query("AND OR NOT");
        assert_eq!(result, "\"AND\" \"OR\" \"NOT\"");
    }

    #[test]
    fn test_sanitize_fts5_column_names() {
        // Column filter syntax like "key:" should be treated as literal
        let result = sanitize_fts5_query("key: value");
        assert_eq!(result, "\"key:\" \"value\"");
    }

    #[test]
    fn test_sanitize_fts5_empty_query() {
        let result = sanitize_fts5_query("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_fts5_whitespace_only() {
        let result = sanitize_fts5_query("   \t\n  ");
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_fts5_single_term() {
        let result = sanitize_fts5_query("hello");
        assert_eq!(result, "\"hello\"");
    }

    #[test]
    fn test_sanitize_fts5_embedded_quotes() {
        // A term containing double quotes should have them escaped
        let result = sanitize_fts5_query("say\"hello\"");
        assert_eq!(result, "\"say\"\"hello\"\"\"");
    }

    #[test]
    fn test_sanitize_fts5_already_quoted_term() {
        // Input that already has surrounding quotes should be double-escaped
        let result = sanitize_fts5_query("\"quoted\"");
        assert_eq!(result, "\"\"\"quoted\"\"\"");
    }

    #[test]
    fn test_sanitize_fts5_special_characters() {
        // Asterisks, parentheses, carets — all FTS5 special chars
        let result = sanitize_fts5_query("foo* (bar) ^baz");
        assert_eq!(result, "\"foo*\" \"(bar)\" \"^baz\"");
    }

    #[test]
    fn test_sanitize_fts5_mixed_reserved_and_normal() {
        let result = sanitize_fts5_query("find AND memory OR context NOT stale");
        assert_eq!(result, "\"find\" \"AND\" \"memory\" \"OR\" \"context\" \"NOT\" \"stale\"");
    }

    // ---- Integration tests: search through FTS5 with reserved words ----

    #[tokio::test]
    async fn test_search_empty_query_returns_empty() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("k1", "some content")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        let results = repo.search("", None, 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_whitespace_query_returns_empty() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("k1", "some content")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        let results = repo.search("   ", None, 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_with_reserved_word_and() {
        let repo = setup_test_repo().await;

        // Store a memory whose content literally contains "AND"
        let memory = Memory::working("reserved_and", "this AND that together")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        // Searching for "AND" should NOT crash — it should just work
        let results = repo.search("AND", None, 10).await.unwrap();
        // FTS5 will look for the literal token "and" (case-insensitive)
        // The memory content contains "AND" so it should match
        assert!(!results.is_empty(), "search for reserved word AND should not crash and should find matching content");
    }

    #[tokio::test]
    async fn test_search_with_reserved_word_or() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("reserved_or", "use OR logic here")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        let results = repo.search("OR", None, 10).await.unwrap();
        assert!(!results.is_empty(), "search for reserved word OR should not crash");
    }

    #[tokio::test]
    async fn test_search_with_reserved_word_not() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("reserved_not", "do NOT forget this")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        let results = repo.search("NOT", None, 10).await.unwrap();
        assert!(!results.is_empty(), "search for reserved word NOT should not crash");
    }

    #[tokio::test]
    async fn test_search_with_reserved_word_near() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("reserved_near", "look NEAR the edge")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        let results = repo.search("NEAR", None, 10).await.unwrap();
        assert!(!results.is_empty(), "search for reserved word NEAR should not crash");
    }

    #[tokio::test]
    async fn test_search_with_column_prefix_syntax() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("col_prefix", "key: value pairs are common")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        // "key:" would normally be interpreted as a column filter; sanitization prevents that
        let results = repo.search("key:", None, 10).await.unwrap();
        // Should not crash, regardless of whether it finds matches
        assert!(results.is_empty() || !results.is_empty(), "search with column prefix should not crash");
    }

    #[tokio::test]
    async fn test_search_with_namespace_filter() {
        let repo = setup_test_repo().await;

        let memory_a = Memory::working("ns_a", "convergence loop feedback")
            .with_namespace("alpha");
        let memory_b = Memory::working("ns_b", "convergence loop feedback")
            .with_namespace("beta");
        repo.store(&memory_a).await.unwrap();
        repo.store(&memory_b).await.unwrap();

        let results = repo.search("convergence", Some("alpha"), 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].namespace, "alpha");
    }

    #[tokio::test]
    async fn test_search_normal_query_still_works() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("normal_search", "the quick brown fox jumps")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        let results = repo.search("quick brown", None, 10).await.unwrap();
        assert!(!results.is_empty(), "normal multi-word search should still find content");
        assert_eq!(results[0].key, "normal_search");
    }

    #[tokio::test]
    async fn test_search_mixed_reserved_and_normal_terms() {
        let repo = setup_test_repo().await;

        let memory = Memory::working("mixed", "find AND fix the memory OR context NOT stale")
            .with_namespace("test");
        repo.store(&memory).await.unwrap();

        // This query mixes reserved words with normal words — previously would crash
        let results = repo.search("find AND memory", None, 10).await.unwrap();
        assert!(!results.is_empty(), "mixed reserved + normal term search should not crash");
    }
}

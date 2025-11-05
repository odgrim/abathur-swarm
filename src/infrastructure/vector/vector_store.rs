//! Vector store implementation using sqlite-vec
//!
//! Provides vector storage and similarity search using the sqlite-vec extension.

use crate::domain::models::{Citation, SearchResult, VectorMemory};
use crate::domain::ports::{EmbeddingRepository, EmbeddingService};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;

/// Vector store for semantic search
///
/// Combines embedding generation with vector storage for efficient similarity search.
pub struct VectorStore {
    pool: Arc<SqlitePool>,
    embedding_service: Arc<dyn EmbeddingService>,
}

impl VectorStore {
    /// Create a new vector store
    ///
    /// # Arguments
    /// * `pool` - SQLite connection pool
    /// * `embedding_service` - Embedding generation service
    ///
    /// # Returns
    /// * `Ok(Self)` - A new vector store
    /// * `Err(_)` - If initialization fails
    pub fn new(pool: Arc<SqlitePool>, embedding_service: Arc<dyn EmbeddingService>) -> Result<Self> {
        Ok(Self {
            pool,
            embedding_service,
        })
    }

    /// Serialize embedding vector to bytes for storage
    fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
        embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect()
    }

    /// Deserialize embedding vector from bytes
    fn bytes_to_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
        if bytes.len() % 4 != 0 {
            return Err(anyhow!("Invalid embedding bytes length"));
        }

        Ok(bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
    }

    /// Calculate cosine distance between two vectors
    fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return f32::MAX;
        }

        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if mag_a == 0.0 || mag_b == 0.0 {
            return f32::MAX;
        }

        // Cosine distance = 1 - cosine_similarity
        // where cosine_similarity = dot / (mag_a * mag_b)
        1.0 - (dot / (mag_a * mag_b))
    }
}

#[async_trait]
impl EmbeddingRepository for VectorStore {
    async fn insert(
        &self,
        id: &str,
        namespace: &str,
        content: &str,
        metadata: Option<serde_json::Value>,
        citation: Option<Citation>,
    ) -> Result<i64> {
        // Generate embedding
        let embedding = self.embedding_service.embed(content).await?;

        // Convert embedding to bytes
        let embedding_bytes = Self::embedding_to_bytes(&embedding);

        // Start transaction
        let mut tx = self.pool.begin().await?;

        // Insert into vec_memory
        let result = sqlx::query(
            "INSERT INTO vec_memory (embedding) VALUES (?)"
        )
        .bind(&embedding_bytes)
        .execute(&mut *tx)
        .await?;

        let rowid = result.last_insert_rowid();

        // Insert into vector_memory
        let metadata_json = metadata.map(|m| m.to_string());
        let citation_json = citation.map(|c| serde_json::to_string(&c).unwrap_or_default());

        sqlx::query(
            r#"
            INSERT INTO vector_memory (id, namespace, content, metadata, source_citation, vector_rowid, created_by)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(id)
        .bind(namespace)
        .bind(content)
        .bind(metadata_json)
        .bind(citation_json)
        .bind(rowid)
        .bind("system")
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(rowid)
    }

    async fn insert_batch(
        &self,
        memories: Vec<(String, String, String)>,
    ) -> Result<Vec<i64>> {
        if memories.is_empty() {
            return Ok(Vec::new());
        }

        // Generate all embeddings in batch (more efficient)
        let contents: Vec<&str> = memories.iter().map(|(_, _, c)| c.as_str()).collect();
        let embeddings = self.embedding_service.embed_batch(&contents).await?;

        let mut rowids = Vec::new();

        // Start transaction for all inserts
        let mut tx = self.pool.begin().await?;

        for ((id, namespace, content), embedding) in memories.iter().zip(embeddings) {
            let embedding_bytes = Self::embedding_to_bytes(&embedding);

            // Insert into vec_memory
            let result = sqlx::query(
                "INSERT INTO vec_memory (embedding) VALUES (?)"
            )
            .bind(&embedding_bytes)
            .execute(&mut *tx)
            .await?;

            let rowid = result.last_insert_rowid();

            // Insert into vector_memory
            sqlx::query(
                r#"
                INSERT INTO vector_memory (id, namespace, content, vector_rowid, created_by)
                VALUES (?, ?, ?, ?, ?)
                "#
            )
            .bind(id)
            .bind(namespace)
            .bind(content)
            .bind(rowid)
            .bind("system")
            .execute(&mut *tx)
            .await?;

            rowids.push(rowid);
        }

        tx.commit().await?;

        Ok(rowids)
    }

    async fn search_similar(
        &self,
        query: &str,
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        // Generate query embedding
        let query_embedding = self.embedding_service.embed(query).await?;

        // Fetch all vectors (in a production system, you'd want to use a proper vector index)
        // For now, we'll do a full scan and calculate distances
        let sql = if let Some(_ns) = namespace_filter {
            format!(
                r#"
                SELECT vm.id, vm.namespace, vm.content, vm.metadata, vm.source_citation, v.embedding
                FROM vec_memory v
                JOIN vector_memory vm ON v.rowid = vm.vector_rowid
                WHERE vm.namespace LIKE ?
                "#
            )
        } else {
            r#"
            SELECT vm.id, vm.namespace, vm.content, vm.metadata, vm.source_citation, v.embedding
            FROM vec_memory v
            JOIN vector_memory vm ON v.rowid = vm.vector_rowid
            "#
            .to_string()
        };

        let rows = if let Some(ns) = namespace_filter {
            sqlx::query(&sql)
                .bind(format!("{}%", ns))
                .fetch_all(&*self.pool)
                .await?
        } else {
            sqlx::query(&sql)
                .fetch_all(&*self.pool)
                .await?
        };

        // Calculate distances and create results
        let mut results = Vec::new();

        for row in rows {
            let id: String = row.get("id");
            let namespace: String = row.get("namespace");
            let content: String = row.get("content");
            let metadata_str: Option<String> = row.get("metadata");
            let citation_str: Option<String> = row.get("source_citation");
            let embedding_bytes: Vec<u8> = row.get("embedding");

            let embedding = Self::bytes_to_embedding(&embedding_bytes)?;
            let distance = Self::cosine_distance(&query_embedding, &embedding);

            let metadata = metadata_str
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::json!({}));

            let citation = citation_str
                .and_then(|s| serde_json::from_str(&s).ok());

            results.push(SearchResult::new(
                id,
                namespace,
                content,
                distance,
                metadata,
                citation,
            ));
        }

        // Sort by distance (ascending) and take top results
        results.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());
        results.truncate(limit);

        Ok(results)
    }

    async fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        _alpha: f32,
    ) -> Result<Vec<SearchResult>> {
        // For now, just use vector search
        // TODO: Implement hybrid search with FTS5 for keyword matching
        self.search_similar(query, limit, None).await
    }

    async fn get(&self, id: &str) -> Result<Option<VectorMemory>> {
        let row = sqlx::query(
            r#"
            SELECT vm.id, vm.namespace, vm.content, vm.metadata, vm.source_citation,
                   vm.created_at, vm.updated_at, vm.created_by, v.embedding
            FROM vector_memory vm
            JOIN vec_memory v ON vm.vector_rowid = v.rowid
            WHERE vm.id = ?
            "#
        )
        .bind(id)
        .fetch_optional(&*self.pool)
        .await?;

        if let Some(row) = row {
            let id: String = row.get("id");
            let namespace: String = row.get("namespace");
            let content: String = row.get("content");
            let metadata_str: Option<String> = row.get("metadata");
            let citation_str: Option<String> = row.get("source_citation");
            let created_at_str: String = row.get("created_at");
            let updated_at_str: String = row.get("updated_at");
            let created_by: String = row.get("created_by");
            let embedding_bytes: Vec<u8> = row.get("embedding");

            let metadata = metadata_str
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::json!({}));

            let citation = citation_str
                .and_then(|s| serde_json::from_str(&s).ok());

            let embedding = Self::bytes_to_embedding(&embedding_bytes)?;

            let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());

            let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());

            Ok(Some(VectorMemory {
                id,
                namespace,
                content,
                embedding,
                metadata,
                source_citation: citation,
                created_at,
                updated_at,
                created_by,
            }))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, id: &str) -> Result<()> {
        // Get the vector_rowid first
        let row = sqlx::query(
            "SELECT vector_rowid FROM vector_memory WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&*self.pool)
        .await?;

        if let Some(row) = row {
            let vector_rowid: i64 = row.get("vector_rowid");

            let mut tx = self.pool.begin().await?;

            // Delete from vector_memory
            sqlx::query("DELETE FROM vector_memory WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;

            // Delete from vec_memory
            sqlx::query("DELETE FROM vec_memory WHERE rowid = ?")
                .bind(vector_rowid)
                .execute(&mut *tx)
                .await?;

            tx.commit().await?;

            Ok(())
        } else {
            Err(anyhow!("Vector memory not found: {}", id))
        }
    }

    async fn count(&self, namespace_prefix: &str) -> Result<usize> {
        let row = sqlx::query(
            "SELECT COUNT(*) as count FROM vector_memory WHERE namespace LIKE ?"
        )
        .bind(format!("{}%", namespace_prefix))
        .fetch_one(&*self.pool)
        .await?;

        let count: i64 = row.get("count");
        Ok(count as usize)
    }

    async fn list_namespaces(&self) -> Result<Vec<(String, usize)>> {
        let rows = sqlx::query(
            r#"
            SELECT namespace, COUNT(*) as count
            FROM vector_memory
            GROUP BY namespace
            ORDER BY count DESC
            "#
        )
        .fetch_all(&*self.pool)
        .await?;

        let mut namespaces = Vec::new();
        for row in rows {
            let namespace: String = row.get("namespace");
            let count: i64 = row.get("count");
            namespaces.push((namespace, count as usize));
        }

        Ok(namespaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_serialization() {
        let embedding = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let bytes = VectorStore::embedding_to_bytes(&embedding);
        let restored = VectorStore::bytes_to_embedding(&bytes).unwrap();

        assert_eq!(embedding.len(), restored.len());
        for (a, b) in embedding.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_cosine_distance() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let distance = VectorStore::cosine_distance(&a, &b);
        assert!((distance - 0.0).abs() < 1e-6); // Identical vectors

        let c = vec![0.0, 1.0, 0.0];
        let distance2 = VectorStore::cosine_distance(&a, &c);
        assert!((distance2 - 1.0).abs() < 1e-6); // Orthogonal vectors
    }
}

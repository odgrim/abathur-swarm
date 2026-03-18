//! SQLite implementation of the OutboxRepository port.

use crate::exec_tx;
use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ports::OutboxRepository;
use crate::services::event_bus::UnifiedEvent;

/// SQLite-backed outbox repository.
#[derive(Clone)]
pub struct SqliteOutboxRepository {
    pool: SqlitePool,
}

impl SqliteOutboxRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OutboxRepository for SqliteOutboxRepository {
    async fn insert(&self, event: &UnifiedEvent) -> DomainResult<()> {
        let id = event.id.0.to_string();
        let event_json = serde_json::to_string(event)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        let created_at = event.timestamp.to_rfc3339();

        let insert_q =
            sqlx::query("INSERT INTO event_outbox (id, event_json, created_at) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(&event_json)
                .bind(&created_at);
        exec_tx!(&self.pool, insert_q, execute)
            .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn fetch_unpublished(&self, limit: usize) -> DomainResult<Vec<UnifiedEvent>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT event_json FROM event_outbox WHERE published_at IS NULL ORDER BY created_at ASC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        let mut events = Vec::with_capacity(rows.len());
        for json in rows {
            let event: UnifiedEvent = serde_json::from_str(&json)
                .map_err(|e| DomainError::SerializationError(e.to_string()))?;
            events.push(event);
        }

        Ok(events)
    }

    async fn mark_published(&self, event_id: &str) -> DomainResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("UPDATE event_outbox SET published_at = ? WHERE id = ?")
            .bind(&now)
            .bind(event_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DomainError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn prune_published(&self, older_than: std::time::Duration) -> DomainResult<u64> {
        let cutoff = (chrono::Utc::now()
            - chrono::Duration::from_std(older_than).unwrap_or_default())
        .to_rfc3339();

        let result = sqlx::query(
            "DELETE FROM event_outbox WHERE published_at IS NOT NULL AND published_at < ?",
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
    use crate::adapters::sqlite::create_migrated_test_pool;
    use crate::services::event_bus::{
        EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn make_test_event() -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: Some(Uuid::new_v4()),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::OrchestratorStarted,
        }
    }

    #[tokio::test]
    async fn test_insert_and_fetch_unpublished() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool);

        let event = make_test_event();
        repo.insert(&event).await.unwrap();

        let unpublished = repo.fetch_unpublished(10).await.unwrap();
        assert_eq!(unpublished.len(), 1);
        assert_eq!(unpublished[0].id, event.id);
    }

    #[tokio::test]
    async fn test_mark_published() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool);

        let event = make_test_event();
        repo.insert(&event).await.unwrap();

        repo.mark_published(&event.id.0.to_string()).await.unwrap();

        let unpublished = repo.fetch_unpublished(10).await.unwrap();
        assert!(unpublished.is_empty());
    }

    #[tokio::test]
    async fn test_fetch_respects_limit() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool);

        for _ in 0..5 {
            repo.insert(&make_test_event()).await.unwrap();
        }

        let unpublished = repo.fetch_unpublished(3).await.unwrap();
        assert_eq!(unpublished.len(), 3);
    }

    #[tokio::test]
    async fn test_prune_published() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool);

        let event = make_test_event();
        repo.insert(&event).await.unwrap();
        repo.mark_published(&event.id.0.to_string()).await.unwrap();

        // Prune with zero duration should remove it (it's already in the past relative to "now")
        let pruned = repo
            .prune_published(std::time::Duration::from_secs(0))
            .await
            .unwrap();
        assert_eq!(pruned, 1);
    }

    #[tokio::test]
    async fn test_empty_fetch() {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool);

        let unpublished = repo.fetch_unpublished(10).await.unwrap();
        assert!(unpublished.is_empty());
    }

    /// Verify that outbox insert works within a shared transaction context
    /// and that both task mutation and outbox insert are committed atomically.
    #[tokio::test]
    async fn test_insert_in_transaction_scope() {
        use crate::adapters::sqlite::tx_context;
        use std::sync::Arc;

        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool.clone());
        let task_repo = crate::adapters::sqlite::SqliteTaskRepository::new(pool.clone());

        // Create a task and insert outbox event within the same transaction
        let tx = pool.begin().await.unwrap();
        let shared_tx: tx_context::SharedTx = Arc::new(tokio::sync::Mutex::new(tx));

        let task = crate::domain::models::Task::new("Test task for atomicity".to_string());
        let event = make_test_event();

        // Run both operations in the same transaction scope
        let tx_clone = shared_tx.clone();
        tx_context::run_in_tx_scope(tx_clone, async {
            use crate::domain::ports::TaskRepository;
            task_repo.create(&task).await.unwrap();
            repo.insert(&event).await.unwrap();
        })
        .await;

        // Commit the transaction — atomically persists both mutation and outbox event
        let tx = Arc::try_unwrap(shared_tx)
            .expect("no other references")
            .into_inner();
        tx.commit().await.unwrap();

        // After commit: both task and outbox event are visible
        let unpublished = repo.fetch_unpublished(10).await.unwrap();
        assert_eq!(unpublished.len(), 1);
        assert_eq!(unpublished[0].id, event.id);

        let task_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?)",
        )
        .bind(task.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(task_exists, "task should be visible after commit");
    }

    /// Verify that rolling back a transaction discards both mutations and outbox events.
    #[tokio::test]
    async fn test_transaction_rollback_discards_both() {
        use crate::adapters::sqlite::tx_context;
        use std::sync::Arc;

        let pool = create_migrated_test_pool().await.unwrap();
        let repo = SqliteOutboxRepository::new(pool.clone());
        let task_repo = crate::adapters::sqlite::SqliteTaskRepository::new(pool.clone());

        let tx = pool.begin().await.unwrap();
        let shared_tx: tx_context::SharedTx = Arc::new(tokio::sync::Mutex::new(tx));

        let task = crate::domain::models::Task::new("Task that will be rolled back".to_string());
        let event = make_test_event();

        let tx_clone = shared_tx.clone();
        tx_context::run_in_tx_scope(tx_clone, async {
            use crate::domain::ports::TaskRepository;
            task_repo.create(&task).await.unwrap();
            repo.insert(&event).await.unwrap();
        })
        .await;

        // Drop the transaction without committing (rollback)
        drop(
            Arc::try_unwrap(shared_tx)
                .expect("no other references")
                .into_inner(),
        );

        // Neither the task nor the outbox event should exist
        let unpublished = repo.fetch_unpublished(10).await.unwrap();
        assert!(unpublished.is_empty(), "outbox should be empty after rollback");

        let task_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?)",
        )
        .bind(task.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!task_exists, "task should not exist after rollback");
    }
}

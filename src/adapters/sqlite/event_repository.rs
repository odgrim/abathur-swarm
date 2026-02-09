//! SQLite implementation of the EventStore trait.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::time::Duration;

use crate::services::event_bus::{
    EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};
use crate::services::event_store::{EventQuery, EventStore, EventStoreError, EventStoreStats};

/// SQLite-backed event repository.
#[derive(Clone)]
pub struct SqliteEventRepository {
    pool: SqlitePool,
}

impl SqliteEventRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn severity_to_string(severity: EventSeverity) -> &'static str {
        match severity {
            EventSeverity::Debug => "debug",
            EventSeverity::Info => "info",
            EventSeverity::Warning => "warning",
            EventSeverity::Error => "error",
            EventSeverity::Critical => "critical",
        }
    }

    fn string_to_severity(s: &str) -> EventSeverity {
        match s {
            "debug" => EventSeverity::Debug,
            "info" => EventSeverity::Info,
            "warning" => EventSeverity::Warning,
            "error" => EventSeverity::Error,
            "critical" => EventSeverity::Critical,
            _ => EventSeverity::Info,
        }
    }

    fn category_to_string(category: EventCategory) -> &'static str {
        match category {
            EventCategory::Orchestrator => "orchestrator",
            EventCategory::Goal => "goal",
            EventCategory::Task => "task",
            EventCategory::Execution => "execution",
            EventCategory::Agent => "agent",
            EventCategory::Verification => "verification",
            EventCategory::Escalation => "escalation",
            EventCategory::Memory => "memory",
            EventCategory::Scheduler => "scheduler",
        }
    }

    fn string_to_category(s: &str) -> EventCategory {
        match s {
            "orchestrator" => EventCategory::Orchestrator,
            "goal" => EventCategory::Goal,
            "task" => EventCategory::Task,
            "execution" => EventCategory::Execution,
            "agent" => EventCategory::Agent,
            "verification" => EventCategory::Verification,
            "escalation" => EventCategory::Escalation,
            "memory" => EventCategory::Memory,
            "scheduler" => EventCategory::Scheduler,
            _ => EventCategory::Task,
        }
    }

    fn severity_order(severity: EventSeverity) -> i32 {
        match severity {
            EventSeverity::Debug => 0,
            EventSeverity::Info => 1,
            EventSeverity::Warning => 2,
            EventSeverity::Error => 3,
            EventSeverity::Critical => 4,
        }
    }
}

#[async_trait]
impl EventStore for SqliteEventRepository {
    async fn append(&self, event: &UnifiedEvent) -> Result<(), EventStoreError> {
        let payload_json = serde_json::to_string(&event.payload)
            .map_err(|e| EventStoreError::SerializationError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO events (id, sequence, timestamp, severity, category, goal_id, task_id, correlation_id, payload)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(event.id.0.to_string())
        .bind(event.sequence.0 as i64)
        .bind(event.timestamp.to_rfc3339())
        .bind(Self::severity_to_string(event.severity))
        .bind(Self::category_to_string(event.category))
        .bind(event.goal_id.map(|id| id.to_string()))
        .bind(event.task_id.map(|id| id.to_string()))
        .bind(event.correlation_id.map(|id| id.to_string()))
        .bind(payload_json)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::AppendError(e.to_string()))?;

        Ok(())
    }

    async fn query(&self, query: EventQuery) -> Result<Vec<UnifiedEvent>, EventStoreError> {
        let mut sql = String::from("SELECT id, sequence, timestamp, severity, category, goal_id, task_id, correlation_id, payload FROM events WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

        if let Some(since) = query.since_sequence {
            sql.push_str(&format!(" AND sequence >= {}", since.0));
        }
        if let Some(until) = query.until_sequence {
            sql.push_str(&format!(" AND sequence <= {}", until.0));
        }
        if let Some(goal_id) = query.goal_id {
            params.push(goal_id.to_string());
            sql.push_str(&format!(" AND goal_id = '{}'", goal_id));
        }
        if let Some(task_id) = query.task_id {
            params.push(task_id.to_string());
            sql.push_str(&format!(" AND task_id = '{}'", task_id));
        }
        if let Some(corr_id) = query.correlation_id {
            params.push(corr_id.to_string());
            sql.push_str(&format!(" AND correlation_id = '{}'", corr_id));
        }
        if let Some(category) = query.category {
            sql.push_str(&format!(
                " AND category = '{}'",
                Self::category_to_string(category)
            ));
        }
        if let Some(min_sev) = query.min_severity {
            // Filter by severity order
            let sev_order = Self::severity_order(min_sev);
            sql.push_str(&format!(
                " AND CASE severity \
                    WHEN 'debug' THEN 0 \
                    WHEN 'info' THEN 1 \
                    WHEN 'warning' THEN 2 \
                    WHEN 'error' THEN 3 \
                    WHEN 'critical' THEN 4 \
                    ELSE 1 END >= {}",
                sev_order
            ));
        }
        if let Some(since_time) = query.since_time {
            sql.push_str(&format!(" AND timestamp >= '{}'", since_time.to_rfc3339()));
        }
        if let Some(until_time) = query.until_time {
            sql.push_str(&format!(" AND timestamp <= '{}'", until_time.to_rfc3339()));
        }

        // Order
        if query.ascending {
            sql.push_str(" ORDER BY sequence ASC");
        } else {
            sql.push_str(" ORDER BY sequence DESC");
        }

        // Pagination
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = query.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        let rows = sqlx::query_as::<_, EventRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let event = self.row_to_event(row)?;
            events.push(event);
        }

        Ok(events)
    }

    async fn latest_sequence(&self) -> Result<Option<SequenceNumber>, EventStoreError> {
        let result: Option<(i64,)> =
            sqlx::query_as("SELECT MAX(sequence) FROM events")
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| EventStoreError::SequenceError(e.to_string()))?;

        Ok(result.and_then(|(seq,)| {
            if seq >= 0 {
                Some(SequenceNumber(seq as u64))
            } else {
                None
            }
        }))
    }

    async fn count(&self) -> Result<u64, EventStoreError> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(result.0 as u64)
    }

    async fn prune_older_than(&self, duration: Duration) -> Result<u64, EventStoreError> {
        let cutoff = Utc::now() - chrono::Duration::from_std(duration).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();

        let result = sqlx::query("DELETE FROM events WHERE timestamp < ?")
            .bind(cutoff_str)
            .execute(&self.pool)
            .await
            .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn get_watermark(&self, handler_name: &str) -> Result<Option<SequenceNumber>, EventStoreError> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT last_sequence FROM handler_watermarks WHERE handler_name = ?"
        )
        .bind(handler_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(result.map(|(seq,)| SequenceNumber(seq as u64)))
    }

    async fn set_watermark(&self, handler_name: &str, seq: SequenceNumber) -> Result<(), EventStoreError> {
        sqlx::query(
            r#"
            INSERT INTO handler_watermarks (handler_name, last_sequence, updated_at)
            VALUES (?, ?, datetime('now'))
            ON CONFLICT(handler_name) DO UPDATE SET
                last_sequence = excluded.last_sequence,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(handler_name)
        .bind(seq.0 as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn detect_sequence_gaps(&self, from: u64, to: u64) -> Result<Vec<(u64, u64)>, EventStoreError> {
        // Query all sequence numbers in the range
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT sequence FROM events WHERE sequence >= ? AND sequence <= ? ORDER BY sequence ASC"
        )
        .bind(from as i64)
        .bind(to as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        let sequences: Vec<u64> = rows.into_iter().map(|(s,)| s as u64).collect();

        let mut gaps = Vec::new();
        let mut expected = from;

        for seq in &sequences {
            if *seq > expected {
                gaps.push((expected, *seq - 1));
            }
            expected = *seq + 1;
        }

        // Check for gap at the end
        if expected <= to {
            gaps.push((expected, to));
        }

        Ok(gaps)
    }

    async fn stats(&self) -> Result<EventStoreStats, EventStoreError> {
        let count = self.count().await?;
        let latest = self.latest_sequence().await?;

        // Get oldest and newest timestamps
        let time_bounds: Option<(String, String)> = sqlx::query_as(
            "SELECT MIN(timestamp), MAX(timestamp) FROM events"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        let (oldest, newest) = if let Some((min_ts, max_ts)) = time_bounds {
            (
                DateTime::parse_from_rfc3339(&min_ts).ok().map(|dt| dt.with_timezone(&Utc)),
                DateTime::parse_from_rfc3339(&max_ts).ok().map(|dt| dt.with_timezone(&Utc)),
            )
        } else {
            (None, None)
        };

        // Get counts by category
        let category_counts: Vec<(String, i64)> = sqlx::query_as(
            "SELECT category, COUNT(*) FROM events GROUP BY category"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        let events_by_category: Vec<(EventCategory, u64)> = category_counts
            .into_iter()
            .map(|(cat, cnt)| (Self::string_to_category(&cat), cnt as u64))
            .collect();

        Ok(EventStoreStats {
            total_events: count,
            latest_sequence: latest,
            oldest_event: oldest,
            newest_event: newest,
            events_by_category,
        })
    }
}

impl SqliteEventRepository {
    fn row_to_event(&self, row: EventRow) -> Result<UnifiedEvent, EventStoreError> {
        let id = uuid::Uuid::parse_str(&row.id)
            .map_err(|e| EventStoreError::QueryError(format!("Invalid event ID: {}", e)))?;

        let timestamp = DateTime::parse_from_rfc3339(&row.timestamp)
            .map_err(|e| EventStoreError::QueryError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&Utc);

        let goal_id = row
            .goal_id
            .as_ref()
            .map(|s| uuid::Uuid::parse_str(s))
            .transpose()
            .map_err(|e| EventStoreError::QueryError(format!("Invalid goal_id: {}", e)))?;

        let task_id = row
            .task_id
            .as_ref()
            .map(|s| uuid::Uuid::parse_str(s))
            .transpose()
            .map_err(|e| EventStoreError::QueryError(format!("Invalid task_id: {}", e)))?;

        let correlation_id = row
            .correlation_id
            .as_ref()
            .map(|s| uuid::Uuid::parse_str(s))
            .transpose()
            .map_err(|e| EventStoreError::QueryError(format!("Invalid correlation_id: {}", e)))?;

        let payload: EventPayload = serde_json::from_str(&row.payload)
            .map_err(|e| EventStoreError::SerializationError(format!("Invalid payload: {}", e)))?;

        Ok(UnifiedEvent {
            id: EventId(id),
            sequence: SequenceNumber(row.sequence as u64),
            timestamp,
            severity: Self::string_to_severity(&row.severity),
            category: Self::string_to_category(&row.category),
            goal_id,
            task_id,
            correlation_id,
            payload,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct EventRow {
    id: String,
    sequence: i64,
    timestamp: String,
    severity: String,
    category: String,
    goal_id: Option<String>,
    task_id: Option<String>,
    correlation_id: Option<String>,
    payload: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_test_pool;

    async fn setup_test_db() -> SqlitePool {
        let pool = create_test_pool().await.unwrap();

        // Create events table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                sequence INTEGER NOT NULL UNIQUE,
                timestamp TEXT NOT NULL,
                severity TEXT NOT NULL,
                category TEXT NOT NULL,
                goal_id TEXT,
                task_id TEXT,
                correlation_id TEXT,
                payload TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    fn make_test_event(seq: u64) -> UnifiedEvent {
        UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(seq),
            timestamp: Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Task,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            payload: EventPayload::OrchestratorStarted,
        }
    }

    #[tokio::test]
    async fn test_sqlite_event_store_append_and_query() {
        let pool = setup_test_db().await;
        let store = SqliteEventRepository::new(pool);

        store.append(&make_test_event(0)).await.unwrap();
        store.append(&make_test_event(1)).await.unwrap();
        store.append(&make_test_event(2)).await.unwrap();

        let all = store.query(EventQuery::new()).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn test_sqlite_event_store_count_and_latest() {
        let pool = setup_test_db().await;
        let store = SqliteEventRepository::new(pool);

        assert_eq!(store.count().await.unwrap(), 0);

        store.append(&make_test_event(5)).await.unwrap();
        store.append(&make_test_event(10)).await.unwrap();

        assert_eq!(store.count().await.unwrap(), 2);
        assert_eq!(
            store.latest_sequence().await.unwrap(),
            Some(SequenceNumber(10))
        );
    }

    #[tokio::test]
    async fn test_sqlite_event_store_filter_by_sequence() {
        let pool = setup_test_db().await;
        let store = SqliteEventRepository::new(pool);

        for i in 0..10 {
            store.append(&make_test_event(i)).await.unwrap();
        }

        let since = store
            .query(EventQuery::new().since_sequence(SequenceNumber(5)))
            .await
            .unwrap();
        assert_eq!(since.len(), 5);
    }
}

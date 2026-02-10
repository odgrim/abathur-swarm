//! SQLite implementation of the EventStore trait.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::time::Duration;

use crate::services::event_bus::{
    EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};
use crate::services::event_store::{CircuitBreakerRecord, DeadLetterEntry, EventQuery, EventStore, EventStoreError, EventStoreStats, WebhookSubscription};

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
            EventCategory::Convergence => "convergence",
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
            "convergence" => EventCategory::Convergence,
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
            INSERT INTO events (id, sequence, timestamp, severity, category, goal_id, task_id, correlation_id, source_process_id, payload)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(event.source_process_id.map(|id| id.to_string()))
        .bind(payload_json)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::AppendError(e.to_string()))?;

        Ok(())
    }

    async fn query(&self, query: EventQuery) -> Result<Vec<UnifiedEvent>, EventStoreError> {
        let mut sql = String::from("SELECT id, sequence, timestamp, severity, category, goal_id, task_id, correlation_id, source_process_id, payload FROM events WHERE 1=1");
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

    async fn load_circuit_breaker_states(&self) -> Result<Vec<CircuitBreakerRecord>, EventStoreError> {
        #[derive(sqlx::FromRow)]
        struct CbRow {
            handler_name: String,
            failure_count: i64,
            tripped: i64,
            tripped_at: Option<String>,
            last_failure: Option<String>,
        }

        let rows: Vec<CbRow> = sqlx::query_as(
            "SELECT handler_name, failure_count, tripped, tripped_at, last_failure FROM circuit_breaker_state",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| CircuitBreakerRecord {
                handler_name: r.handler_name,
                failure_count: r.failure_count as u32,
                tripped: r.tripped != 0,
                tripped_at: r.tripped_at
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                last_failure: r.last_failure
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
            })
            .collect())
    }

    async fn save_circuit_breaker_state(
        &self,
        handler_name: &str,
        failure_count: u32,
        tripped: bool,
        tripped_at: Option<DateTime<Utc>>,
        last_failure: Option<DateTime<Utc>>,
    ) -> Result<(), EventStoreError> {
        sqlx::query(
            r#"
            INSERT INTO circuit_breaker_state (handler_name, failure_count, tripped, tripped_at, last_failure)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(handler_name) DO UPDATE SET
                failure_count = excluded.failure_count,
                tripped = excluded.tripped,
                tripped_at = excluded.tripped_at,
                last_failure = excluded.last_failure
            "#,
        )
        .bind(handler_name)
        .bind(failure_count as i64)
        .bind(if tripped { 1i64 } else { 0 })
        .bind(tripped_at.map(|t| t.to_rfc3339()))
        .bind(last_failure.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn append_dead_letter(
        &self,
        event_id: &str,
        event_sequence: u64,
        handler_name: &str,
        error_message: &str,
        max_retries: u32,
    ) -> Result<(), EventStoreError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        // First retry after 2 seconds (2^0 * 2)
        let next_retry = (Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO dead_letter_events (id, event_id, event_sequence, handler_name, error_message, retry_count, max_retries, next_retry_at, created_at)
            VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(event_id)
        .bind(event_sequence as i64)
        .bind(handler_name)
        .bind(error_message)
        .bind(max_retries as i64)
        .bind(&next_retry)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn get_retryable_dead_letters(&self, limit: u32) -> Result<Vec<DeadLetterEntry>, EventStoreError> {
        let now = Utc::now().to_rfc3339();

        let rows: Vec<DeadLetterRow> = sqlx::query_as(
            r#"
            SELECT id, event_id, event_sequence, handler_name, error_message,
                   retry_count, max_retries, next_retry_at, created_at, resolved_at
            FROM dead_letter_events
            WHERE resolved_at IS NULL
              AND retry_count < max_retries
              AND (next_retry_at IS NULL OR next_retry_at <= ?)
            ORDER BY next_retry_at ASC
            LIMIT ?
            "#,
        )
        .bind(&now)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            entries.push(DeadLetterEntry {
                id: row.id,
                event_id: row.event_id,
                event_sequence: row.event_sequence as u64,
                handler_name: row.handler_name,
                error_message: row.error_message,
                retry_count: row.retry_count as u32,
                max_retries: row.max_retries as u32,
                next_retry_at: row.next_retry_at
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                created_at: DateTime::parse_from_rfc3339(&row.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                resolved_at: None,
            });
        }

        Ok(entries)
    }

    async fn resolve_dead_letter(&self, id: &str) -> Result<(), EventStoreError> {
        let now = Utc::now().to_rfc3339();

        sqlx::query("UPDATE dead_letter_events SET resolved_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn increment_dead_letter_retry(&self, id: &str, next_retry_at: DateTime<Utc>) -> Result<(), EventStoreError> {
        sqlx::query(
            "UPDATE dead_letter_events SET retry_count = retry_count + 1, next_retry_at = ? WHERE id = ?",
        )
        .bind(next_retry_at.to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn list_dead_letters(
        &self,
        handler_name: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DeadLetterEntry>, EventStoreError> {
        let rows: Vec<DeadLetterRow> = if let Some(handler) = handler_name {
            sqlx::query_as(
                r#"
                SELECT id, event_id, event_sequence, handler_name, error_message,
                       retry_count, max_retries, next_retry_at, created_at, resolved_at
                FROM dead_letter_events
                WHERE resolved_at IS NULL AND handler_name = ?
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(handler)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as(
                r#"
                SELECT id, event_id, event_sequence, handler_name, error_message,
                       retry_count, max_retries, next_retry_at, created_at, resolved_at
                FROM dead_letter_events
                WHERE resolved_at IS NULL
                ORDER BY created_at DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            entries.push(DeadLetterEntry {
                id: row.id,
                event_id: row.event_id,
                event_sequence: row.event_sequence as u64,
                handler_name: row.handler_name,
                error_message: row.error_message,
                retry_count: row.retry_count as u32,
                max_retries: row.max_retries as u32,
                next_retry_at: row.next_retry_at
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                created_at: DateTime::parse_from_rfc3339(&row.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                resolved_at: None,
            });
        }

        Ok(entries)
    }

    async fn purge_dead_letters(&self, older_than: std::time::Duration) -> Result<u64, EventStoreError> {
        let cutoff = (Utc::now() - chrono::Duration::from_std(older_than).unwrap_or_default()).to_rfc3339();

        let result = sqlx::query(
            "DELETE FROM dead_letter_events WHERE resolved_at IS NOT NULL AND created_at < ?",
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected())
    }

    // -- Webhook management --

    async fn create_webhook(
        &self,
        id: &str,
        url: &str,
        secret: Option<&str>,
        filter_json: &str,
        max_failures: u32,
        created_at: &str,
    ) -> Result<(), EventStoreError> {
        sqlx::query(
            r#"
            INSERT INTO webhook_subscriptions (id, url, secret, filter_json, active, max_failures, failure_count, last_delivered_sequence, created_at, updated_at)
            VALUES (?, ?, ?, ?, 1, ?, 0, 0, ?, ?)
            "#,
        )
        .bind(id)
        .bind(url)
        .bind(secret)
        .bind(filter_json)
        .bind(max_failures as i64)
        .bind(created_at)
        .bind(created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn list_webhooks(&self) -> Result<Vec<WebhookSubscription>, EventStoreError> {
        let rows: Vec<WebhookRow> = sqlx::query_as(
            "SELECT id, url, secret, filter_json, active, max_failures, failure_count, last_delivered_sequence, created_at FROM webhook_subscriptions ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn get_webhook(&self, id: &str) -> Result<Option<WebhookSubscription>, EventStoreError> {
        let row: Option<WebhookRow> = sqlx::query_as(
            "SELECT id, url, secret, filter_json, active, max_failures, failure_count, last_delivered_sequence, created_at FROM webhook_subscriptions WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(row.map(|r| r.into()))
    }

    async fn delete_webhook(&self, id: &str) -> Result<(), EventStoreError> {
        sqlx::query("DELETE FROM webhook_subscriptions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn list_active_webhooks_for_category(&self, category: &str) -> Result<Vec<WebhookSubscription>, EventStoreError> {
        let rows: Vec<WebhookRow> = sqlx::query_as(
            r#"
            SELECT id, url, secret, filter_json, active, max_failures, failure_count, last_delivered_sequence, created_at
            FROM webhook_subscriptions
            WHERE active = 1
              AND (filter_json = '{}' OR filter_json LIKE '%"category":"' || ? || '"%' OR filter_json LIKE '%"category":null%')
            "#
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EventStoreError::QueryError(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn record_webhook_failure(&self, id: &str) -> Result<(), EventStoreError> {
        sqlx::query(
            r#"
            UPDATE webhook_subscriptions
            SET failure_count = failure_count + 1,
                active = CASE WHEN failure_count + 1 >= max_failures THEN 0 ELSE active END,
                updated_at = datetime('now')
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn update_webhook_sequence(&self, id: &str, sequence: u64) -> Result<(), EventStoreError> {
        sqlx::query(
            "UPDATE webhook_subscriptions SET last_delivered_sequence = ?, failure_count = 0, updated_at = datetime('now') WHERE id = ?"
        )
        .bind(sequence as i64)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| EventStoreError::DatabaseError(e.to_string()))?;

        Ok(())
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
            source_process_id: row.source_process_id
                .as_ref()
                .map(|s| uuid::Uuid::parse_str(s))
                .transpose()
                .map_err(|e| EventStoreError::QueryError(format!("Invalid source_process_id: {}", e)))?,
            payload,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct DeadLetterRow {
    id: String,
    event_id: String,
    event_sequence: i64,
    handler_name: String,
    error_message: String,
    retry_count: i64,
    max_retries: i64,
    next_retry_at: Option<String>,
    created_at: String,
    resolved_at: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct WebhookRow {
    id: String,
    url: String,
    secret: Option<String>,
    filter_json: String,
    active: i64,
    max_failures: i64,
    failure_count: i64,
    last_delivered_sequence: i64,
    created_at: String,
}

impl From<WebhookRow> for WebhookSubscription {
    fn from(row: WebhookRow) -> Self {
        let filter_category = serde_json::from_str::<serde_json::Value>(&row.filter_json)
            .ok()
            .and_then(|v| v.get("category").and_then(|c| c.as_str().map(String::from)));

        WebhookSubscription {
            id: row.id,
            url: row.url,
            secret: row.secret,
            filter_category,
            active: row.active != 0,
            failure_count: row.failure_count as u32,
            max_failures: row.max_failures as u32,
            last_delivered_sequence: row.last_delivered_sequence as u64,
            created_at: row.created_at,
        }
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
    source_process_id: Option<String>,
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
                source_process_id TEXT,
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
            source_process_id: None,
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

//! EventStore trait for event persistence.
//!
//! Defines the interface for storing and querying unified events.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use std::time::Duration;
use uuid::Uuid;

use super::event_bus::{EventCategory, EventSeverity, SequenceNumber, UnifiedEvent};

/// Error type for EventStore operations.
#[derive(Debug, Error)]
pub enum EventStoreError {
    #[error("Failed to append event: {0}")]
    AppendError(String),

    #[error("Failed to query events: {0}")]
    QueryError(String),

    #[error("Failed to get sequence: {0}")]
    SequenceError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),
}

/// Query parameters for event retrieval.
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    /// Filter by minimum sequence number (inclusive).
    pub since_sequence: Option<SequenceNumber>,
    /// Filter by maximum sequence number (inclusive).
    pub until_sequence: Option<SequenceNumber>,
    /// Filter by goal ID.
    pub goal_id: Option<Uuid>,
    /// Filter by task ID.
    pub task_id: Option<Uuid>,
    /// Filter by correlation ID.
    pub correlation_id: Option<Uuid>,
    /// Filter by category.
    pub category: Option<EventCategory>,
    /// Filter by minimum severity.
    pub min_severity: Option<EventSeverity>,
    /// Filter by timestamp (events after this time).
    pub since_time: Option<DateTime<Utc>>,
    /// Filter by timestamp (events before this time).
    pub until_time: Option<DateTime<Utc>>,
    /// Maximum number of events to return.
    pub limit: Option<u32>,
    /// Offset for pagination.
    pub offset: Option<u32>,
    /// Sort order (true = ascending by sequence, false = descending).
    pub ascending: bool,
}

impl EventQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn since_sequence(mut self, seq: SequenceNumber) -> Self {
        self.since_sequence = Some(seq);
        self
    }

    pub fn until_sequence(mut self, seq: SequenceNumber) -> Self {
        self.until_sequence = Some(seq);
        self
    }

    pub fn goal_id(mut self, id: Uuid) -> Self {
        self.goal_id = Some(id);
        self
    }

    pub fn task_id(mut self, id: Uuid) -> Self {
        self.task_id = Some(id);
        self
    }

    pub fn correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }

    pub fn category(mut self, category: EventCategory) -> Self {
        self.category = Some(category);
        self
    }

    pub fn min_severity(mut self, severity: EventSeverity) -> Self {
        self.min_severity = Some(severity);
        self
    }

    pub fn since_time(mut self, time: DateTime<Utc>) -> Self {
        self.since_time = Some(time);
        self
    }

    pub fn until_time(mut self, time: DateTime<Utc>) -> Self {
        self.until_time = Some(time);
        self
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn ascending(mut self) -> Self {
        self.ascending = true;
        self
    }

    pub fn descending(mut self) -> Self {
        self.ascending = false;
        self
    }
}

/// Statistics about the event store.
#[derive(Debug, Clone, Default)]
pub struct EventStoreStats {
    /// Total number of events stored.
    pub total_events: u64,
    /// Latest sequence number.
    pub latest_sequence: Option<SequenceNumber>,
    /// Oldest event timestamp.
    pub oldest_event: Option<DateTime<Utc>>,
    /// Newest event timestamp.
    pub newest_event: Option<DateTime<Utc>>,
    /// Events by category.
    pub events_by_category: Vec<(EventCategory, u64)>,
}

/// Trait for event persistence implementations.
#[async_trait]
pub trait EventStore: Send + Sync {
    /// Append an event to the store.
    async fn append(&self, event: &UnifiedEvent) -> Result<(), EventStoreError>;

    /// Query events based on filter criteria.
    async fn query(&self, query: EventQuery) -> Result<Vec<UnifiedEvent>, EventStoreError>;

    /// Get the latest sequence number in the store.
    async fn latest_sequence(&self) -> Result<Option<SequenceNumber>, EventStoreError>;

    /// Count total events in the store.
    async fn count(&self) -> Result<u64, EventStoreError>;

    /// Prune events older than the specified duration.
    async fn prune_older_than(&self, duration: Duration) -> Result<u64, EventStoreError>;

    /// Get store statistics.
    async fn stats(&self) -> Result<EventStoreStats, EventStoreError> {
        let count = self.count().await?;
        let latest = self.latest_sequence().await?;
        Ok(EventStoreStats {
            total_events: count,
            latest_sequence: latest,
            ..Default::default()
        })
    }

    /// Get a single event by sequence number.
    async fn get_by_sequence(&self, sequence: SequenceNumber) -> Result<Option<UnifiedEvent>, EventStoreError> {
        let events = self.query(
            EventQuery::new()
                .since_sequence(sequence)
                .until_sequence(sequence)
                .limit(1)
        ).await?;
        Ok(events.into_iter().next())
    }

    /// Get events since a sequence number (for replay).
    async fn replay_since(&self, sequence: SequenceNumber) -> Result<Vec<UnifiedEvent>, EventStoreError> {
        self.query(EventQuery::new().since_sequence(sequence).ascending()).await
    }

    /// Get the last processed sequence number for a handler.
    async fn get_watermark(&self, _handler_name: &str) -> Result<Option<SequenceNumber>, EventStoreError> {
        Ok(None)
    }

    /// Set the last processed sequence number for a handler.
    async fn set_watermark(&self, _handler_name: &str, _seq: SequenceNumber) -> Result<(), EventStoreError> {
        Ok(())
    }

    /// Detect gaps in the sequence number range [from, to].
    ///
    /// Returns a list of (gap_start, gap_end) pairs representing missing
    /// sequence ranges. Used by ReconciliationHandler to detect lost events.
    async fn detect_sequence_gaps(&self, _from: u64, _to: u64) -> Result<Vec<(u64, u64)>, EventStoreError> {
        Ok(vec![])
    }

    /// Append a dead letter entry for a handler that failed to process an event.
    async fn append_dead_letter(
        &self,
        _event_id: &str,
        _event_sequence: u64,
        _handler_name: &str,
        _error_message: &str,
        _max_retries: u32,
    ) -> Result<(), EventStoreError> {
        Ok(())
    }

    /// Get dead letter entries that are ready for retry (resolved_at IS NULL,
    /// retry_count < max_retries, next_retry_at <= now).
    async fn get_retryable_dead_letters(&self, _limit: u32) -> Result<Vec<DeadLetterEntry>, EventStoreError> {
        Ok(vec![])
    }

    /// Mark a dead letter entry as resolved.
    async fn resolve_dead_letter(&self, _id: &str) -> Result<(), EventStoreError> {
        Ok(())
    }

    /// Increment retry count and set next_retry_at for a dead letter entry.
    async fn increment_dead_letter_retry(&self, _id: &str, _next_retry_at: DateTime<Utc>) -> Result<(), EventStoreError> {
        Ok(())
    }

    // -- Circuit breaker persistence --

    /// Load all persisted circuit breaker states.
    async fn load_circuit_breaker_states(&self) -> Result<Vec<CircuitBreakerRecord>, EventStoreError> {
        Ok(vec![])
    }

    /// Persist a circuit breaker state change.
    async fn save_circuit_breaker_state(
        &self,
        _handler_name: &str,
        _failure_count: u32,
        _tripped: bool,
        _tripped_at: Option<DateTime<Utc>>,
        _last_failure: Option<DateTime<Utc>>,
    ) -> Result<(), EventStoreError> {
        Ok(())
    }

    // -- Dead letter queue management --

    /// List dead letter entries with optional handler filter.
    async fn list_dead_letters(
        &self,
        _handler_name: Option<&str>,
        _limit: u32,
    ) -> Result<Vec<DeadLetterEntry>, EventStoreError> {
        Ok(vec![])
    }

    /// Purge resolved dead letter entries older than the given duration.
    /// Returns the number of entries purged.
    async fn purge_dead_letters(&self, _older_than: Duration) -> Result<u64, EventStoreError> {
        Ok(0)
    }

    // -- Webhook management --

    /// Create a webhook subscription.
    async fn create_webhook(
        &self,
        _id: &str,
        _url: &str,
        _secret: Option<&str>,
        _filter_json: &str,
        _max_failures: u32,
        _created_at: &str,
    ) -> Result<(), EventStoreError> {
        Err(EventStoreError::QueryError("Webhooks not supported".into()))
    }

    /// List all webhook subscriptions.
    async fn list_webhooks(&self) -> Result<Vec<WebhookSubscription>, EventStoreError> {
        Ok(vec![])
    }

    /// Get a webhook subscription by ID.
    async fn get_webhook(&self, _id: &str) -> Result<Option<WebhookSubscription>, EventStoreError> {
        Ok(None)
    }

    /// Delete a webhook subscription.
    async fn delete_webhook(&self, _id: &str) -> Result<(), EventStoreError> {
        Ok(())
    }

    /// List active webhooks matching a category filter.
    async fn list_active_webhooks_for_category(&self, _category: &str) -> Result<Vec<WebhookSubscription>, EventStoreError> {
        Ok(vec![])
    }

    /// Increment failure count for a webhook; deactivate if over max.
    async fn record_webhook_failure(&self, _id: &str) -> Result<(), EventStoreError> {
        Ok(())
    }

    /// Update the last delivered sequence for a webhook.
    async fn update_webhook_sequence(&self, _id: &str, _sequence: u64) -> Result<(), EventStoreError> {
        Ok(())
    }
}

/// A webhook subscription record.
#[derive(Debug, Clone)]
pub struct WebhookSubscription {
    pub id: String,
    pub url: String,
    pub secret: Option<String>,
    pub filter_category: Option<String>,
    pub active: bool,
    pub failure_count: u32,
    pub max_failures: u32,
    pub last_delivered_sequence: u64,
    pub created_at: String,
}

/// A persisted circuit breaker state record.
#[derive(Debug, Clone)]
pub struct CircuitBreakerRecord {
    pub handler_name: String,
    pub failure_count: u32,
    pub tripped: bool,
    pub tripped_at: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
}

/// A dead letter queue entry representing a handler failure.
#[derive(Debug, Clone)]
pub struct DeadLetterEntry {
    pub id: String,
    pub event_id: String,
    pub event_sequence: u64,
    pub handler_name: String,
    pub error_message: String,
    pub retry_count: u32,
    pub max_retries: u32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// In-memory event store for testing.
#[derive(Debug, Default)]
pub struct InMemoryEventStore {
    events: tokio::sync::RwLock<Vec<UnifiedEvent>>,
    watermarks: tokio::sync::RwLock<std::collections::HashMap<String, SequenceNumber>>,
    dead_letters: tokio::sync::RwLock<Vec<DeadLetterEntry>>,
    circuit_breakers: tokio::sync::RwLock<Vec<CircuitBreakerRecord>>,
    webhooks: tokio::sync::RwLock<Vec<WebhookSubscription>>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn append(&self, event: &UnifiedEvent) -> Result<(), EventStoreError> {
        let mut events = self.events.write().await;
        events.push(event.clone());
        Ok(())
    }

    async fn query(&self, query: EventQuery) -> Result<Vec<UnifiedEvent>, EventStoreError> {
        let events = self.events.read().await;
        let mut result: Vec<_> = events
            .iter()
            .filter(|e| {
                if let Some(seq) = query.since_sequence {
                    if e.sequence < seq {
                        return false;
                    }
                }
                if let Some(seq) = query.until_sequence {
                    if e.sequence > seq {
                        return false;
                    }
                }
                if let Some(goal_id) = query.goal_id {
                    if e.goal_id != Some(goal_id) {
                        return false;
                    }
                }
                if let Some(task_id) = query.task_id {
                    if e.task_id != Some(task_id) {
                        return false;
                    }
                }
                if let Some(corr_id) = query.correlation_id {
                    if e.correlation_id != Some(corr_id) {
                        return false;
                    }
                }
                if let Some(category) = query.category {
                    if e.category != category {
                        return false;
                    }
                }
                if let Some(since) = query.since_time {
                    if e.timestamp < since {
                        return false;
                    }
                }
                if let Some(until) = query.until_time {
                    if e.timestamp > until {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        if query.ascending {
            result.sort_by_key(|e| e.sequence);
        } else {
            result.sort_by_key(|e| std::cmp::Reverse(e.sequence));
        }

        if let Some(offset) = query.offset {
            result = result.into_iter().skip(offset as usize).collect();
        }

        if let Some(limit) = query.limit {
            result.truncate(limit as usize);
        }

        Ok(result)
    }

    async fn latest_sequence(&self) -> Result<Option<SequenceNumber>, EventStoreError> {
        let events = self.events.read().await;
        Ok(events.iter().map(|e| e.sequence).max())
    }

    async fn count(&self) -> Result<u64, EventStoreError> {
        let events = self.events.read().await;
        Ok(events.len() as u64)
    }

    async fn prune_older_than(&self, duration: Duration) -> Result<u64, EventStoreError> {
        let cutoff = Utc::now() - chrono::Duration::from_std(duration).unwrap_or_default();
        let mut events = self.events.write().await;
        let original_len = events.len();
        events.retain(|e| e.timestamp >= cutoff);
        Ok((original_len - events.len()) as u64)
    }

    // === Watermarks ===

    async fn get_watermark(&self, handler_name: &str) -> Result<Option<SequenceNumber>, EventStoreError> {
        let watermarks = self.watermarks.read().await;
        Ok(watermarks.get(handler_name).copied())
    }

    async fn set_watermark(&self, handler_name: &str, seq: SequenceNumber) -> Result<(), EventStoreError> {
        let mut watermarks = self.watermarks.write().await;
        watermarks.insert(handler_name.to_string(), seq);
        Ok(())
    }

    // === Sequence Gap Detection ===

    async fn detect_sequence_gaps(&self, from: u64, to: u64) -> Result<Vec<(u64, u64)>, EventStoreError> {
        let events = self.events.read().await;
        let present: std::collections::HashSet<u64> = events
            .iter()
            .filter(|e| e.sequence.0 >= from && e.sequence.0 <= to)
            .map(|e| e.sequence.0)
            .collect();

        let mut gaps = Vec::new();
        let mut gap_start: Option<u64> = None;
        for seq in from..=to {
            if !present.contains(&seq) {
                if gap_start.is_none() {
                    gap_start = Some(seq);
                }
            } else if let Some(start) = gap_start.take() {
                gaps.push((start, seq - 1));
            }
        }
        if let Some(start) = gap_start {
            gaps.push((start, to));
        }
        Ok(gaps)
    }

    // === Dead Letters ===

    async fn append_dead_letter(
        &self,
        event_id: &str,
        event_sequence: u64,
        handler_name: &str,
        error_message: &str,
        max_retries: u32,
    ) -> Result<(), EventStoreError> {
        let mut dead_letters = self.dead_letters.write().await;
        dead_letters.push(DeadLetterEntry {
            id: Uuid::new_v4().to_string(),
            event_id: event_id.to_string(),
            event_sequence,
            handler_name: handler_name.to_string(),
            error_message: error_message.to_string(),
            retry_count: 0,
            max_retries,
            next_retry_at: None,
            created_at: Utc::now(),
            resolved_at: None,
        });
        Ok(())
    }

    async fn get_retryable_dead_letters(&self, limit: u32) -> Result<Vec<DeadLetterEntry>, EventStoreError> {
        let dead_letters = self.dead_letters.read().await;
        let now = Utc::now();
        let result: Vec<_> = dead_letters
            .iter()
            .filter(|dl| {
                dl.resolved_at.is_none()
                    && dl.retry_count < dl.max_retries
                    && dl.next_retry_at.map_or(true, |t| t <= now)
            })
            .take(limit as usize)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn resolve_dead_letter(&self, id: &str) -> Result<(), EventStoreError> {
        let mut dead_letters = self.dead_letters.write().await;
        if let Some(dl) = dead_letters.iter_mut().find(|dl| dl.id == id) {
            dl.resolved_at = Some(Utc::now());
        }
        Ok(())
    }

    async fn increment_dead_letter_retry(&self, id: &str, next_retry_at: DateTime<Utc>) -> Result<(), EventStoreError> {
        let mut dead_letters = self.dead_letters.write().await;
        if let Some(dl) = dead_letters.iter_mut().find(|dl| dl.id == id) {
            dl.retry_count += 1;
            dl.next_retry_at = Some(next_retry_at);
        }
        Ok(())
    }

    async fn list_dead_letters(
        &self,
        handler_name: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DeadLetterEntry>, EventStoreError> {
        let dead_letters = self.dead_letters.read().await;
        let result: Vec<_> = dead_letters
            .iter()
            .filter(|dl| handler_name.map_or(true, |h| dl.handler_name == h))
            .take(limit as usize)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn purge_dead_letters(&self, older_than: Duration) -> Result<u64, EventStoreError> {
        let cutoff = Utc::now() - chrono::Duration::from_std(older_than).unwrap_or_default();
        let mut dead_letters = self.dead_letters.write().await;
        let original_len = dead_letters.len();
        dead_letters.retain(|dl| {
            // Only purge resolved entries older than cutoff
            !(dl.resolved_at.is_some() && dl.created_at < cutoff)
        });
        Ok((original_len - dead_letters.len()) as u64)
    }

    // === Circuit Breakers ===

    async fn load_circuit_breaker_states(&self) -> Result<Vec<CircuitBreakerRecord>, EventStoreError> {
        let cbs = self.circuit_breakers.read().await;
        Ok(cbs.clone())
    }

    async fn save_circuit_breaker_state(
        &self,
        handler_name: &str,
        failure_count: u32,
        tripped: bool,
        tripped_at: Option<DateTime<Utc>>,
        last_failure: Option<DateTime<Utc>>,
    ) -> Result<(), EventStoreError> {
        let mut cbs = self.circuit_breakers.write().await;
        if let Some(cb) = cbs.iter_mut().find(|cb| cb.handler_name == handler_name) {
            cb.failure_count = failure_count;
            cb.tripped = tripped;
            cb.tripped_at = tripped_at;
            cb.last_failure = last_failure;
        } else {
            cbs.push(CircuitBreakerRecord {
                handler_name: handler_name.to_string(),
                failure_count,
                tripped,
                tripped_at,
                last_failure,
            });
        }
        Ok(())
    }

    // === Webhooks ===

    async fn create_webhook(
        &self,
        id: &str,
        url: &str,
        secret: Option<&str>,
        filter_json: &str,
        max_failures: u32,
        created_at: &str,
    ) -> Result<(), EventStoreError> {
        let mut webhooks = self.webhooks.write().await;
        let filter_category = serde_json::from_str::<serde_json::Value>(filter_json)
            .ok()
            .and_then(|v| v.get("category").and_then(|c| c.as_str().map(String::from)));
        webhooks.push(WebhookSubscription {
            id: id.to_string(),
            url: url.to_string(),
            secret: secret.map(String::from),
            filter_category,
            active: true,
            failure_count: 0,
            max_failures,
            last_delivered_sequence: 0,
            created_at: created_at.to_string(),
        });
        Ok(())
    }

    async fn list_webhooks(&self) -> Result<Vec<WebhookSubscription>, EventStoreError> {
        let webhooks = self.webhooks.read().await;
        Ok(webhooks.clone())
    }

    async fn get_webhook(&self, id: &str) -> Result<Option<WebhookSubscription>, EventStoreError> {
        let webhooks = self.webhooks.read().await;
        Ok(webhooks.iter().find(|w| w.id == id).cloned())
    }

    async fn delete_webhook(&self, id: &str) -> Result<(), EventStoreError> {
        let mut webhooks = self.webhooks.write().await;
        webhooks.retain(|w| w.id != id);
        Ok(())
    }

    async fn list_active_webhooks_for_category(&self, category: &str) -> Result<Vec<WebhookSubscription>, EventStoreError> {
        let webhooks = self.webhooks.read().await;
        let result: Vec<_> = webhooks
            .iter()
            .filter(|w| {
                w.active && w.filter_category.as_deref().map_or(true, |c| c == category)
            })
            .cloned()
            .collect();
        Ok(result)
    }

    async fn record_webhook_failure(&self, id: &str) -> Result<(), EventStoreError> {
        let mut webhooks = self.webhooks.write().await;
        if let Some(w) = webhooks.iter_mut().find(|w| w.id == id) {
            w.failure_count += 1;
            if w.failure_count >= w.max_failures {
                w.active = false;
            }
        }
        Ok(())
    }

    async fn update_webhook_sequence(&self, id: &str, sequence: u64) -> Result<(), EventStoreError> {
        let mut webhooks = self.webhooks.write().await;
        if let Some(w) = webhooks.iter_mut().find(|w| w.id == id) {
            w.last_delivered_sequence = sequence;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::event_bus::{EventId, EventPayload};

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
    async fn test_in_memory_store_append_and_query() {
        let store = InMemoryEventStore::new();

        store.append(&make_test_event(0)).await.unwrap();
        store.append(&make_test_event(1)).await.unwrap();
        store.append(&make_test_event(2)).await.unwrap();

        let all = store.query(EventQuery::new()).await.unwrap();
        assert_eq!(all.len(), 3);

        let since = store.query(EventQuery::new().since_sequence(SequenceNumber(1))).await.unwrap();
        assert_eq!(since.len(), 2);
    }

    #[tokio::test]
    async fn test_in_memory_store_pagination() {
        let store = InMemoryEventStore::new();

        for i in 0..10 {
            store.append(&make_test_event(i)).await.unwrap();
        }

        let page1 = store.query(EventQuery::new().limit(3).ascending()).await.unwrap();
        assert_eq!(page1.len(), 3);
        assert_eq!(page1[0].sequence.0, 0);

        let page2 = store.query(EventQuery::new().limit(3).offset(3).ascending()).await.unwrap();
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].sequence.0, 3);
    }

    #[tokio::test]
    async fn test_in_memory_store_count_and_latest() {
        let store = InMemoryEventStore::new();

        assert_eq!(store.count().await.unwrap(), 0);
        assert!(store.latest_sequence().await.unwrap().is_none());

        store.append(&make_test_event(5)).await.unwrap();
        store.append(&make_test_event(10)).await.unwrap();

        assert_eq!(store.count().await.unwrap(), 2);
        assert_eq!(store.latest_sequence().await.unwrap(), Some(SequenceNumber(10)));
    }
}

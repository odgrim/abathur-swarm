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
}

/// In-memory event store for testing.
#[derive(Debug, Default)]
pub struct InMemoryEventStore {
    events: tokio::sync::RwLock<Vec<UnifiedEvent>>,
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

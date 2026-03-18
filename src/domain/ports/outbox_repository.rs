//! Outbox repository port for the transactional outbox pattern.
//!
//! Events are written to the outbox within the same SQLite transaction as domain
//! mutations (via the task-local [`tx_context::ACTIVE_TX`](crate::adapters::sqlite::tx_context)
//! shared transaction), then a background poller reads and publishes them to the EventBus.
//!
//! # Atomicity guarantee
//!
//! When the CommandBus dispatches with outbox enabled, it wraps both the handler's
//! repository operations and the outbox inserts in a single SQLite transaction. If
//! either fails, the entire transaction rolls back — neither the domain mutation
//! nor the outbox events are persisted.
//!
//! # At-least-once delivery
//!
//! The outbox poller reads unpublished events and publishes them to the EventBus.
//! If `mark_published` fails after a successful publish, the event will be
//! re-published on the next poll cycle. **Downstream event handlers MUST be
//! idempotent** to tolerate duplicate delivery.

use async_trait::async_trait;

use crate::domain::errors::DomainResult;
use crate::services::event_bus::UnifiedEvent;

/// Repository interface for the event outbox.
#[async_trait]
pub trait OutboxRepository: Send + Sync {
    /// Insert an event into the outbox (unpublished).
    async fn insert(&self, event: &UnifiedEvent) -> DomainResult<()>;

    /// Fetch up to `limit` unpublished events, ordered by creation time.
    async fn fetch_unpublished(&self, limit: usize) -> DomainResult<Vec<UnifiedEvent>>;

    /// Mark an event as published.
    async fn mark_published(&self, event_id: &str) -> DomainResult<()>;

    /// Delete published events older than the given duration (cleanup).
    async fn prune_published(&self, older_than: std::time::Duration) -> DomainResult<u64>;
}

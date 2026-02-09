//! Shared EventBus factory for CLI and MCP commands.
//!
//! Creates a persistent EventBus backed by the SQLite event store,
//! so events from CLI commands survive restarts and can be replayed
//! by the orchestrator.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::adapters::sqlite::SqliteEventRepository;
use crate::services::event_bus::{EventBus, EventBusConfig};
use crate::services::event_store::EventStore;

/// Create an EventBus that persists events to SQLite.
///
/// CLI and standalone MCP commands should use this instead of
/// `EventBus::new(EventBusConfig::default())` so that events
/// are written to the store and available for orchestrator replay.
pub fn create_persistent_event_bus(pool: SqlitePool) -> Arc<EventBus> {
    let event_store = Arc::new(SqliteEventRepository::new(pool));
    Arc::new(
        EventBus::new(EventBusConfig {
            persist_events: true,
            ..Default::default()
        })
        .with_store(event_store as Arc<dyn EventStore>),
    )
}

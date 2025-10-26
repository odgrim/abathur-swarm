use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::models::{Session, SessionEvent};

/// Repository trait for session persistence operations
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Create a new session
    async fn create(&self, session: Session) -> anyhow::Result<Uuid>;

    /// Get a session by ID
    async fn get(&self, id: Uuid) -> anyhow::Result<Option<Session>>;

    /// Append an event to a session's history
    async fn append_event(&self, session_id: Uuid, event: SessionEvent) -> anyhow::Result<()>;

    /// Get all events for a session, ordered by timestamp
    async fn get_events(&self, session_id: Uuid) -> anyhow::Result<Vec<SessionEvent>>;

    /// Get a specific state value from a session's state object
    async fn get_state(&self, session_id: Uuid, key: &str) -> anyhow::Result<Option<Value>>;

    /// Set a state value in a session's state object (merges, doesn't replace)
    async fn set_state(&self, session_id: Uuid, key: &str, value: Value) -> anyhow::Result<()>;
}

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::errors::DatabaseError;
use super::utils::parse_datetime;
use crate::domain::models::{Session, SessionEvent};
use crate::domain::ports::SessionRepository;

/// `SQLite` implementation of `SessionRepository`
pub struct SessionRepositoryImpl {
    pool: SqlitePool,
}

impl SessionRepositoryImpl {
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionRepository for SessionRepositoryImpl {
    async fn create(&self, session: Session) -> Result<Uuid> {
        let state_json =
            serde_json::to_string(&session.state).context("failed to serialize session state")?;

        let id_str = session.id.to_string();
        let created_at_str = session.created_at.to_rfc3339();
        let updated_at_str = session.updated_at.to_rfc3339();

        sqlx::query!(
            r#"
            INSERT INTO sessions (id, app_name, user_id, project_id, state, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
            id_str,
            session.app_name,
            session.user_id,
            session.project_id,
            state_json,
            created_at_str,
            updated_at_str
        )
        .execute(&self.pool)
        .await
        .context("failed to insert session")?;

        Ok(session.id)
    }

    async fn get(&self, id: Uuid) -> Result<Option<Session>> {
        let id_str = id.to_string();
        let row = sqlx::query!(
            r#"
            SELECT id, app_name, user_id, project_id, state, created_at, updated_at
            FROM sessions
            WHERE id = ?
            "#,
            id_str
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch session")?;

        match row {
            Some(r) => {
                let session = Session {
                    id: Uuid::parse_str(&r.id).context("invalid UUID in database")?,
                    app_name: r.app_name,
                    user_id: r.user_id,
                    project_id: r.project_id,
                    state: serde_json::from_str(&r.state).context("failed to deserialize state")?,
                    created_at: parse_datetime(&r.created_at)
                        .context("invalid created_at timestamp")?,
                    updated_at: parse_datetime(&r.updated_at)
                        .context("invalid updated_at timestamp")?,
                };
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    async fn append_event(&self, session_id: Uuid, event: SessionEvent) -> Result<()> {
        let content_json =
            serde_json::to_string(&event.content).context("failed to serialize event content")?;

        let session_id_str = session_id.to_string();
        let event_id_str = event.event_id.to_string();
        let timestamp_str = event.timestamp.to_rfc3339();

        sqlx::query!(
            r#"
            INSERT INTO session_events (session_id, event_id, event_type, actor, content, timestamp)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
            session_id_str,
            event_id_str,
            event.event_type,
            event.actor,
            content_json,
            timestamp_str
        )
        .execute(&self.pool)
        .await
        .context("failed to insert session event")?;

        Ok(())
    }

    async fn get_events(&self, session_id: Uuid) -> Result<Vec<SessionEvent>> {
        let session_id_str = session_id.to_string();
        let rows = sqlx::query!(
            r#"
            SELECT id, session_id, event_id, event_type, actor, content, timestamp
            FROM session_events
            WHERE session_id = ?
            ORDER BY timestamp ASC
            "#,
            session_id_str
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to fetch session events")?;

        let events: Result<Vec<SessionEvent>> = rows
            .into_iter()
            .map(|r| {
                Ok(SessionEvent {
                    id: r.id.unwrap_or(0), // AUTO INCREMENT should always provide an id
                    session_id: Uuid::parse_str(&r.session_id)
                        .context("invalid session_id UUID")?,
                    event_id: Uuid::parse_str(&r.event_id).context("invalid event_id UUID")?,
                    event_type: r.event_type,
                    actor: r.actor,
                    content: serde_json::from_str(&r.content)
                        .context("failed to deserialize event content")?,
                    timestamp: parse_datetime(&r.timestamp)
                        .context("invalid timestamp")?,
                })
            })
            .collect();

        events
    }

    async fn get_state(&self, session_id: Uuid, key: &str) -> Result<Option<Value>> {
        let session_id_str = session_id.to_string();
        let row = sqlx::query!(
            r#"
            SELECT state
            FROM sessions
            WHERE id = ?
            "#,
            session_id_str
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch session state")?;

        match row {
            Some(r) => {
                let state: Value =
                    serde_json::from_str(&r.state).context("failed to deserialize state")?;

                // Extract the value at the given key
                let value = state.get(key).cloned();
                Ok(value)
            }
            None => Ok(None),
        }
    }

    async fn set_state(&self, session_id: Uuid, key: &str, value: Value) -> Result<()> {
        // First, get the current state
        let session_id_str = session_id.to_string();
        let row = sqlx::query!(
            r#"
            SELECT state
            FROM sessions
            WHERE id = ?
            "#,
            session_id_str
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch current session state")?;

        let row = row.ok_or_else(|| DatabaseError::SessionNotFound(session_id))?;

        // Parse the current state
        let mut state: Value =
            serde_json::from_str(&row.state).context("failed to deserialize current state")?;

        // Merge the new value into the state
        if let Value::Object(ref mut map) = state {
            map.insert(key.to_string(), value);
        } else {
            return Err(DatabaseError::InvalidStateUpdate(
                "session state is not a JSON object".to_string(),
            )
            .into());
        }

        // Serialize the updated state
        let state_json =
            serde_json::to_string(&state).context("failed to serialize updated state")?;

        let now = Utc::now();
        let updated_at_str = now.to_rfc3339();
        let session_id_str2 = session_id.to_string();

        // Update the session with the new state
        sqlx::query!(
            r#"
            UPDATE sessions
            SET state = ?, updated_at = ?
            WHERE id = ?
            "#,
            state_json,
            updated_at_str,
            session_id_str2
        )
        .execute(&self.pool)
        .await
        .context("failed to update session state")?;

        Ok(())
    }
}

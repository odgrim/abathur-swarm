use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: Uuid,
    pub app_name: String,
    pub user_id: String,
    pub project_id: Option<String>,
    pub state: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEvent {
    pub id: i64,
    pub session_id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub actor: String,
    pub content: Value,
    pub timestamp: DateTime<Utc>,
}

impl Session {
    pub fn new(app_name: String, user_id: String, project_id: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            app_name,
            user_id,
            project_id,
            state: Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
        }
    }
}

impl SessionEvent {
    pub fn new(session_id: Uuid, event_type: String, actor: String, content: Value) -> Self {
        Self {
            id: 0, // Will be set by database
            session_id,
            event_id: Uuid::new_v4(),
            event_type,
            actor,
            content,
            timestamp: Utc::now(),
        }
    }
}

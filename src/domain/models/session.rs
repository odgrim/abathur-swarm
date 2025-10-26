use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub id: Uuid,
    pub app_name: String,
    pub user_id: String,
    pub project_id: Option<String>,
    pub state: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_session_new() {
        let session = Session::new(
            "test_app".to_string(),
            "user123".to_string(),
            Some("project456".to_string()),
        );

        assert_eq!(session.app_name, "test_app");
        assert_eq!(session.user_id, "user123");
        assert_eq!(session.project_id, Some("project456".to_string()));
        assert_eq!(session.state, Value::Object(serde_json::Map::new()));
        assert!(session.created_at <= Utc::now());
        assert!(session.updated_at <= Utc::now());
    }

    #[test]
    fn test_session_new_without_project() {
        let session = Session::new("test_app".to_string(), "user123".to_string(), None);

        assert_eq!(session.app_name, "test_app");
        assert_eq!(session.user_id, "user123");
        assert_eq!(session.project_id, None);
    }

    #[test]
    fn test_session_serialization() {
        let session = Session::new(
            "test_app".to_string(),
            "user123".to_string(),
            Some("project456".to_string()),
        );

        let serialized = serde_json::to_string(&session).unwrap();
        let deserialized: Session = serde_json::from_str(&serialized).unwrap();

        assert_eq!(session.id, deserialized.id);
        assert_eq!(session.app_name, deserialized.app_name);
        assert_eq!(session.user_id, deserialized.user_id);
        assert_eq!(session.project_id, deserialized.project_id);
    }

    #[test]
    fn test_session_event_new() {
        let session_id = Uuid::new_v4();
        let event = SessionEvent::new(
            session_id,
            "task_completed".to_string(),
            "agent_planner".to_string(),
            json!({"task_id": "123", "status": "success"}),
        );

        assert_eq!(event.id, 0); // Will be set by database
        assert_eq!(event.session_id, session_id);
        assert_eq!(event.event_type, "task_completed");
        assert_eq!(event.actor, "agent_planner");
        assert_eq!(event.content["task_id"], "123");
        assert_eq!(event.content["status"], "success");
        assert!(event.timestamp <= Utc::now());
    }

    #[test]
    fn test_session_event_serialization() {
        let session_id = Uuid::new_v4();
        let event = SessionEvent::new(
            session_id,
            "user_action".to_string(),
            "alice".to_string(),
            json!({"action": "submit", "data": "test"}),
        );

        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: SessionEvent = serde_json::from_str(&serialized).unwrap();

        assert_eq!(event.event_id, deserialized.event_id);
        assert_eq!(event.session_id, deserialized.session_id);
        assert_eq!(event.event_type, deserialized.event_type);
        assert_eq!(event.actor, deserialized.actor);
        assert_eq!(event.content, deserialized.content);
    }

    #[test]
    fn test_session_clone() {
        let session = Session::new(
            "test_app".to_string(),
            "user123".to_string(),
            Some("project456".to_string()),
        );

        let cloned = session.clone();
        assert_eq!(session, cloned);
    }

    #[test]
    fn test_session_event_clone() {
        let session_id = Uuid::new_v4();
        let event = SessionEvent::new(
            session_id,
            "test_event".to_string(),
            "test_actor".to_string(),
            json!({"key": "value"}),
        );

        let cloned = event.clone();
        assert_eq!(event, cloned);
    }
}

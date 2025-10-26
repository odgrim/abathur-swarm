/// Domain models for session management.
///
/// Sessions track conversation state, event history, and key-value state storage
/// with namespace support for hierarchical organization.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Session lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Session created but not yet active
    Created,
    /// Session is active and accepting events
    Active,
    /// Session temporarily paused
    Paused,
    /// Session terminated, no further events accepted
    Terminated,
    /// Session archived for long-term storage
    Archived,
}

/// Event in session history
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Unique event identifier
    pub event_id: String,

    /// ISO 8601 timestamp
    pub timestamp: DateTime<Utc>,

    /// Event type (message|action|tool_call|reflection)
    pub event_type: String,

    /// Actor identifier (user|agent:<agent_id>|system)
    pub actor: String,

    /// Event-specific data
    pub content: HashMap<String, serde_json::Value>,

    /// Whether this is the final response in a conversation turn
    #[serde(default)]
    pub is_final_response: bool,
}

impl Event {
    /// Creates a new event with required fields
    pub fn new(
        event_id: String,
        timestamp: DateTime<Utc>,
        event_type: String,
        actor: String,
        content: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            event_id,
            timestamp,
            event_type,
            actor,
            content,
            is_final_response: false,
        }
    }

    /// Creates a new event marked as final response
    pub fn with_final_response(mut self) -> Self {
        self.is_final_response = true;
        self
    }
}

/// Conversation session with event history and state management
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier
    pub id: String,

    /// Application context (e.g., "abathur")
    pub app_name: String,

    /// User identifier
    pub user_id: String,

    /// Optional project association for cross-agent collaboration
    pub project_id: Option<String>,

    /// Current lifecycle status
    pub status: SessionStatus,

    /// Ordered list of events in session history
    pub events: Vec<Event>,

    /// Key-value state storage with namespace prefixes
    /// Examples: "session:abc123:current_task", "user:alice:theme"
    pub state: HashMap<String, serde_json::Value>,

    /// Extensible metadata for future use
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,

    /// Session creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last modification timestamp
    pub last_update_time: DateTime<Utc>,

    /// Termination timestamp (if terminated)
    pub terminated_at: Option<DateTime<Utc>>,

    /// Archive timestamp (if archived)
    pub archived_at: Option<DateTime<Utc>>,
}

impl Session {
    /// Creates a new session with required fields
    pub fn new(id: String, app_name: String, user_id: String, project_id: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id,
            app_name,
            user_id,
            project_id,
            status: SessionStatus::Created,
            events: Vec::new(),
            state: HashMap::new(),
            metadata: HashMap::new(),
            created_at: now,
            last_update_time: now,
            terminated_at: None,
            archived_at: None,
        }
    }

    /// Creates a new session with UUID identifier
    pub fn new_with_uuid(app_name: String, user_id: String, project_id: Option<String>) -> Self {
        Self::new(Uuid::new_v4().to_string(), app_name, user_id, project_id)
    }

    /// Appends an event to session history
    pub fn append_event(&mut self, event: Event) {
        self.events.push(event);
        self.last_update_time = Utc::now();
    }

    /// Updates session status
    pub fn update_status(&mut self, status: SessionStatus) {
        self.status = status;
        self.last_update_time = Utc::now();

        if status == SessionStatus::Terminated {
            self.terminated_at = Some(Utc::now());
        } else if status == SessionStatus::Archived {
            self.archived_at = Some(Utc::now());
        }
    }

    /// Gets a state value by key
    pub fn get_state(&self, key: &str) -> Option<&serde_json::Value> {
        self.state.get(key)
    }

    /// Sets a state value
    pub fn set_state(&mut self, key: String, value: serde_json::Value) {
        self.state.insert(key, value);
        self.last_update_time = Utc::now();
    }

    /// Merges state delta into current state
    pub fn merge_state(&mut self, delta: HashMap<String, serde_json::Value>) {
        for (key, value) in delta {
            self.state.insert(key, value);
        }
        self.last_update_time = Utc::now();
    }

    /// Returns true if session is active
    pub fn is_active(&self) -> bool {
        self.status == SessionStatus::Active
    }

    /// Returns true if session can accept new events
    pub fn can_accept_events(&self) -> bool {
        matches!(self.status, SessionStatus::Created | SessionStatus::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_new_session() {
        let session = Session::new(
            "test_id".to_string(),
            "abathur".to_string(),
            "alice".to_string(),
            Some("project1".to_string()),
        );

        assert_eq!(session.id, "test_id");
        assert_eq!(session.app_name, "abathur");
        assert_eq!(session.user_id, "alice");
        assert_eq!(session.project_id, Some("project1".to_string()));
        assert_eq!(session.status, SessionStatus::Created);
        assert!(session.events.is_empty());
        assert!(session.state.is_empty());
    }

    #[test]
    fn test_new_session_with_uuid() {
        let session = Session::new_with_uuid("abathur".to_string(), "alice".to_string(), None);

        assert!(!session.id.is_empty());
        assert_eq!(session.app_name, "abathur");
        assert_eq!(session.user_id, "alice");
        assert_eq!(session.project_id, None);
    }

    #[test]
    fn test_append_event() {
        let mut session = Session::new(
            "test_id".to_string(),
            "abathur".to_string(),
            "alice".to_string(),
            None,
        );

        let event = Event::new(
            "evt_001".to_string(),
            Utc::now(),
            "message".to_string(),
            "user".to_string(),
            HashMap::new(),
        );

        session.append_event(event);
        assert_eq!(session.events.len(), 1);
        assert_eq!(session.events[0].event_id, "evt_001");
    }

    #[test]
    fn test_update_status() {
        let mut session = Session::new(
            "test_id".to_string(),
            "abathur".to_string(),
            "alice".to_string(),
            None,
        );

        session.update_status(SessionStatus::Active);
        assert_eq!(session.status, SessionStatus::Active);
        assert!(session.terminated_at.is_none());

        session.update_status(SessionStatus::Terminated);
        assert_eq!(session.status, SessionStatus::Terminated);
        assert!(session.terminated_at.is_some());
    }

    #[test]
    fn test_state_management() {
        let mut session = Session::new(
            "test_id".to_string(),
            "abathur".to_string(),
            "alice".to_string(),
            None,
        );

        session.set_state("user:alice:theme".to_string(), json!("dark"));
        assert_eq!(session.get_state("user:alice:theme"), Some(&json!("dark")));

        let mut delta = HashMap::new();
        delta.insert("session:test_id:task".to_string(), json!("design"));
        session.merge_state(delta);

        assert_eq!(session.state.len(), 2);
        assert_eq!(
            session.get_state("session:test_id:task"),
            Some(&json!("design"))
        );
    }

    #[test]
    fn test_can_accept_events() {
        let mut session = Session::new(
            "test_id".to_string(),
            "abathur".to_string(),
            "alice".to_string(),
            None,
        );

        assert!(session.can_accept_events());

        session.update_status(SessionStatus::Active);
        assert!(session.can_accept_events());

        session.update_status(SessionStatus::Paused);
        assert!(!session.can_accept_events());

        session.update_status(SessionStatus::Terminated);
        assert!(!session.can_accept_events());
    }

    #[test]
    fn test_event_with_final_response() {
        let event = Event::new(
            "evt_001".to_string(),
            Utc::now(),
            "message".to_string(),
            "agent:gpt4".to_string(),
            HashMap::new(),
        )
        .with_final_response();

        assert!(event.is_final_response);
    }
}

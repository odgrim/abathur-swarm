/// Session management service coordinating session operations with repository.
///
/// This service implements business logic for session lifecycle, event management,
/// and state coordination following Clean Architecture principles.
use crate::domain::models::{Event, Session, SessionStatus};
use crate::domain::ports::SessionRepository;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{instrument, warn};

/// Service for managing conversation sessions with events and state.
///
/// Coordinates session creation, event appending, and state management
/// through the SessionRepository trait, enabling dependency injection
/// and testability.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use abathur::services::SessionService;
/// use abathur::domain::ports::SessionRepository;
///
/// async fn example(repo: Arc<dyn SessionRepository>) {
///     let service = SessionService::new(repo);
///
///     let session = Session::new_with_uuid(
///         "abathur".to_string(),
///         "alice".to_string(),
///         Some("project1".to_string()),
///     );
///
///     service.create(session).await.unwrap();
/// }
/// ```
pub struct SessionService {
    /// Repository for session persistence
    repo: Arc<dyn SessionRepository>,
}

impl SessionService {
    /// Creates a new SessionService with the provided repository
    ///
    /// # Arguments
    /// - `repo`: Session repository implementation (injected dependency)
    pub fn new(repo: Arc<dyn SessionRepository>) -> Self {
        Self { repo }
    }

    /// Creates a new session
    ///
    /// # Arguments
    /// - `session`: Session to create
    ///
    /// # Errors
    /// Returns error if:
    /// - Session ID already exists
    /// - Repository operation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use abathur::services::SessionService;
    /// # use abathur::domain::models::Session;
    /// # async fn example(service: SessionService) -> anyhow::Result<()> {
    /// let session = Session::new(
    ///     "session_123".to_string(),
    ///     "abathur".to_string(),
    ///     "alice".to_string(),
    ///     None,
    /// );
    /// service.create(session).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), fields(session_id = %session.id), err)]
    pub async fn create(&self, session: Session) -> Result<()> {
        // Validate session ID is not empty
        if session.id.is_empty() {
            return Err(anyhow!("Session ID cannot be empty"));
        }

        // Validate required fields
        if session.app_name.is_empty() {
            return Err(anyhow!("App name cannot be empty"));
        }
        if session.user_id.is_empty() {
            return Err(anyhow!("User ID cannot be empty"));
        }

        // Check if session already exists
        let exists = self
            .repo
            .exists(&session.id)
            .await
            .context("Failed to check if session exists")?;

        if exists {
            return Err(anyhow!("Session {} already exists", session.id));
        }

        // Create session via repository
        self.repo
            .create(session)
            .await
            .context("Failed to create session")?;

        Ok(())
    }

    /// Retrieves session by ID
    ///
    /// # Arguments
    /// - `session_id`: Session identifier
    ///
    /// # Returns
    /// - `Some(Session)` if found
    /// - `None` if not found
    ///
    /// # Errors
    /// Returns error if repository operation fails
    #[instrument(skip(self), err)]
    pub async fn get(&self, session_id: &str) -> Result<Option<Session>> {
        if session_id.is_empty() {
            return Err(anyhow!("Session ID cannot be empty"));
        }

        self.repo
            .get(session_id)
            .await
            .context("Failed to retrieve session")
    }

    /// Appends an event to session history with optional state update
    ///
    /// # Arguments
    /// - `session_id`: Session identifier
    /// - `event`: Event to append
    /// - `state_delta`: Optional state changes to merge
    ///
    /// # Errors
    /// Returns error if:
    /// - Session not found
    /// - Session cannot accept events (terminated/archived)
    /// - Repository operation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use abathur::services::SessionService;
    /// # use abathur::domain::models::Event;
    /// # use chrono::Utc;
    /// # use std::collections::HashMap;
    /// # async fn example(service: SessionService) -> anyhow::Result<()> {
    /// let event = Event::new(
    ///     "evt_001".to_string(),
    ///     Utc::now(),
    ///     "message".to_string(),
    ///     "user".to_string(),
    ///     HashMap::new(),
    /// );
    /// service.append_event("session_123", event, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self, event), fields(session_id, event_id = %event.event_id), err)]
    pub async fn append_event(
        &self,
        session_id: &str,
        event: Event,
        state_delta: Option<HashMap<String, Value>>,
    ) -> Result<()> {
        // Validate inputs
        if session_id.is_empty() {
            return Err(anyhow!("Session ID cannot be empty"));
        }
        if event.event_id.is_empty() {
            return Err(anyhow!("Event ID cannot be empty"));
        }

        // Fetch session
        let mut session = self
            .get(session_id)
            .await?
            .ok_or_else(|| anyhow!("Session {} not found", session_id))?;

        // Validate session can accept events
        if !session.can_accept_events() {
            warn!(
                "Attempted to append event to session {} with status {:?}",
                session_id, session.status
            );
            return Err(anyhow!(
                "Session {} cannot accept events (status: {:?})",
                session_id,
                session.status
            ));
        }

        // Apply business logic
        session.append_event(event);

        // Merge state delta if provided
        if let Some(delta) = state_delta {
            session.merge_state(delta);
        }

        // Persist changes
        self.repo
            .update(session)
            .await
            .context("Failed to update session with new event")?;

        Ok(())
    }

    /// Gets a specific state value from session
    ///
    /// # Arguments
    /// - `session_id`: Session identifier
    /// - `key`: State key (with namespace prefix, e.g., "user:alice:theme")
    ///
    /// # Returns
    /// - `Some(Value)` if key exists
    /// - `None` if key doesn't exist or session not found
    ///
    /// # Errors
    /// Returns error if repository operation fails
    #[instrument(skip(self), err)]
    pub async fn get_state(&self, session_id: &str, key: &str) -> Result<Option<Value>> {
        if session_id.is_empty() {
            return Err(anyhow!("Session ID cannot be empty"));
        }
        if key.is_empty() {
            return Err(anyhow!("State key cannot be empty"));
        }

        let session = self.get(session_id).await?;

        Ok(session.and_then(|s| s.get_state(key).cloned()))
    }

    /// Sets a specific state value in session
    ///
    /// # Arguments
    /// - `session_id`: Session identifier
    /// - `key`: State key (with namespace prefix)
    /// - `value`: State value (JSON-serializable)
    ///
    /// # Errors
    /// Returns error if:
    /// - Session not found
    /// - Repository operation fails
    #[instrument(skip(self, value), err)]
    pub async fn set_state(&self, session_id: &str, key: &str, value: Value) -> Result<()> {
        if session_id.is_empty() {
            return Err(anyhow!("Session ID cannot be empty"));
        }
        if key.is_empty() {
            return Err(anyhow!("State key cannot be empty"));
        }

        // Fetch session
        let mut session = self
            .get(session_id)
            .await?
            .ok_or_else(|| anyhow!("Session {} not found", session_id))?;

        // Apply business logic
        session.set_state(key.to_string(), value);

        // Persist changes
        self.repo
            .update(session)
            .await
            .context("Failed to update session state")?;

        Ok(())
    }

    /// Updates session status
    ///
    /// # Arguments
    /// - `session_id`: Session identifier
    /// - `status`: New status
    ///
    /// # Errors
    /// Returns error if:
    /// - Session not found
    /// - Repository operation fails
    #[instrument(skip(self), err)]
    pub async fn update_status(&self, session_id: &str, status: SessionStatus) -> Result<()> {
        if session_id.is_empty() {
            return Err(anyhow!("Session ID cannot be empty"));
        }

        // Fetch session
        let mut session = self
            .get(session_id)
            .await?
            .ok_or_else(|| anyhow!("Session {} not found", session_id))?;

        // Apply business logic
        session.update_status(status);

        // Persist changes
        self.repo
            .update(session)
            .await
            .context("Failed to update session status")?;

        Ok(())
    }

    /// Lists sessions with optional filters
    ///
    /// # Arguments
    /// - `project_id`: Optional project ID filter
    /// - `status`: Optional status filter
    /// - `limit`: Maximum number of results (default: 50, max: 1000)
    ///
    /// # Errors
    /// Returns error if repository operation fails
    #[instrument(skip(self), err)]
    pub async fn list(
        &self,
        project_id: Option<&str>,
        status: Option<SessionStatus>,
        limit: Option<usize>,
    ) -> Result<Vec<Session>> {
        let limit = limit.unwrap_or(50).min(1000);

        self.repo
            .list(project_id, status, limit)
            .await
            .context("Failed to list sessions")
    }

    /// Terminates a session (convenience method)
    ///
    /// # Errors
    /// Returns error if:
    /// - Session not found
    /// - Repository operation fails
    #[instrument(skip(self), err)]
    pub async fn terminate(&self, session_id: &str) -> Result<()> {
        self.update_status(session_id, SessionStatus::Terminated)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Mutex;

    /// Mock repository for testing
    struct MockSessionRepository {
        sessions: Mutex<HashMap<String, Session>>,
    }

    impl MockSessionRepository {
        fn new() -> Self {
            Self {
                sessions: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl SessionRepository for MockSessionRepository {
        async fn create(&self, session: Session) -> Result<()> {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.contains_key(&session.id) {
                return Err(anyhow!("Session already exists"));
            }
            sessions.insert(session.id.clone(), session);
            Ok(())
        }

        async fn get(&self, session_id: &str) -> Result<Option<Session>> {
            let sessions = self.sessions.lock().unwrap();
            Ok(sessions.get(session_id).cloned())
        }

        async fn update(&self, session: Session) -> Result<()> {
            let mut sessions = self.sessions.lock().unwrap();
            if !sessions.contains_key(&session.id) {
                return Err(anyhow!("Session not found"));
            }
            sessions.insert(session.id.clone(), session);
            Ok(())
        }

        async fn list(
            &self,
            project_id: Option<&str>,
            status: Option<SessionStatus>,
            limit: usize,
        ) -> Result<Vec<Session>> {
            let sessions = self.sessions.lock().unwrap();
            let mut results: Vec<Session> = sessions
                .values()
                .filter(|s| {
                    let project_match = project_id
                        .map(|pid| s.project_id.as_deref() == Some(pid))
                        .unwrap_or(true);
                    let status_match = status.map(|st| s.status == st).unwrap_or(true);
                    project_match && status_match
                })
                .cloned()
                .collect();

            results.sort_by(|a, b| b.last_update_time.cmp(&a.last_update_time));
            results.truncate(limit);
            Ok(results)
        }

        async fn delete(&self, session_id: &str) -> Result<()> {
            let mut sessions = self.sessions.lock().unwrap();
            sessions
                .remove(session_id)
                .ok_or_else(|| anyhow!("Session not found"))?;
            Ok(())
        }

        async fn exists(&self, session_id: &str) -> Result<bool> {
            let sessions = self.sessions.lock().unwrap();
            Ok(sessions.contains_key(session_id))
        }
    }

    fn create_test_service() -> SessionService {
        let repo = Arc::new(MockSessionRepository::new());
        SessionService::new(repo)
    }

    fn create_test_session() -> Session {
        Session::new(
            "test_session_123".to_string(),
            "abathur".to_string(),
            "alice".to_string(),
            Some("project1".to_string()),
        )
    }

    #[tokio::test]
    async fn test_create_session_success() {
        let service = create_test_service();
        let session = create_test_session();

        let result = service.create(session.clone()).await;
        assert!(result.is_ok());

        // Verify session was created
        let retrieved = service.get(&session.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, session.id);
    }

    #[tokio::test]
    async fn test_create_session_duplicate_fails() {
        let service = create_test_service();
        let session = create_test_session();

        service.create(session.clone()).await.unwrap();

        // Try to create again
        let result = service.create(session).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn test_create_session_validates_fields() {
        let service = create_test_service();

        // Empty session ID
        let mut session = create_test_session();
        session.id = String::new();
        assert!(service.create(session).await.is_err());

        // Empty app name
        let mut session = create_test_session();
        session.app_name = String::new();
        assert!(service.create(session).await.is_err());

        // Empty user ID
        let mut session = create_test_session();
        session.user_id = String::new();
        assert!(service.create(session).await.is_err());
    }

    #[tokio::test]
    async fn test_get_session() {
        let service = create_test_service();
        let session = create_test_session();

        service.create(session.clone()).await.unwrap();

        let retrieved = service.get(&session.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, session.id);

        // Non-existent session
        let result = service.get("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_append_event_success() {
        let service = create_test_service();
        let mut session = create_test_session();
        session.status = SessionStatus::Active;
        service.create(session.clone()).await.unwrap();

        let event = Event::new(
            "evt_001".to_string(),
            Utc::now(),
            "message".to_string(),
            "user".to_string(),
            HashMap::new(),
        );

        let result = service.append_event(&session.id, event, None).await;
        assert!(result.is_ok());

        // Verify event was appended
        let updated = service.get(&session.id).await.unwrap().unwrap();
        assert_eq!(updated.events.len(), 1);
        assert_eq!(updated.events[0].event_id, "evt_001");
    }

    #[tokio::test]
    async fn test_append_event_with_state_delta() {
        let service = create_test_service();
        let mut session = create_test_session();
        session.status = SessionStatus::Active;
        service.create(session.clone()).await.unwrap();

        let event = Event::new(
            "evt_001".to_string(),
            Utc::now(),
            "message".to_string(),
            "user".to_string(),
            HashMap::new(),
        );

        let mut state_delta = HashMap::new();
        state_delta.insert("session:test:task".to_string(), json!("design"));

        service
            .append_event(&session.id, event, Some(state_delta))
            .await
            .unwrap();

        // Verify state was updated
        let updated = service.get(&session.id).await.unwrap().unwrap();
        assert_eq!(updated.state.len(), 1);
        assert_eq!(
            updated.get_state("session:test:task"),
            Some(&json!("design"))
        );
    }

    #[tokio::test]
    async fn test_append_event_fails_on_terminated_session() {
        let service = create_test_service();
        let mut session = create_test_session();
        session.status = SessionStatus::Terminated;
        service.create(session.clone()).await.unwrap();

        let event = Event::new(
            "evt_001".to_string(),
            Utc::now(),
            "message".to_string(),
            "user".to_string(),
            HashMap::new(),
        );

        let result = service.append_event(&session.id, event, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot accept"));
    }

    #[tokio::test]
    async fn test_get_state() {
        let service = create_test_service();
        let mut session = create_test_session();
        session.set_state("user:alice:theme".to_string(), json!("dark"));
        service.create(session.clone()).await.unwrap();

        let value = service
            .get_state(&session.id, "user:alice:theme")
            .await
            .unwrap();
        assert_eq!(value, Some(json!("dark")));

        let missing = service.get_state(&session.id, "nonexistent").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_set_state() {
        let service = create_test_service();
        let session = create_test_session();
        service.create(session.clone()).await.unwrap();

        service
            .set_state(&session.id, "user:alice:theme", json!("dark"))
            .await
            .unwrap();

        let updated = service.get(&session.id).await.unwrap().unwrap();
        assert_eq!(updated.get_state("user:alice:theme"), Some(&json!("dark")));
    }

    #[tokio::test]
    async fn test_update_status() {
        let service = create_test_service();
        let session = create_test_session();
        service.create(session.clone()).await.unwrap();

        service
            .update_status(&session.id, SessionStatus::Active)
            .await
            .unwrap();

        let updated = service.get(&session.id).await.unwrap().unwrap();
        assert_eq!(updated.status, SessionStatus::Active);
    }

    #[tokio::test]
    async fn test_terminate() {
        let service = create_test_service();
        let session = create_test_session();
        service.create(session.clone()).await.unwrap();

        service.terminate(&session.id).await.unwrap();

        let updated = service.get(&session.id).await.unwrap().unwrap();
        assert_eq!(updated.status, SessionStatus::Terminated);
        assert!(updated.terminated_at.is_some());
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let service = create_test_service();

        // Create multiple sessions
        for i in 0..5 {
            let mut session = Session::new(
                format!("session_{}", i),
                "abathur".to_string(),
                "alice".to_string(),
                Some("project1".to_string()),
            );
            if i % 2 == 0 {
                session.status = SessionStatus::Active;
            }
            service.create(session).await.unwrap();
        }

        // List all
        let all = service.list(None, None, None).await.unwrap();
        assert_eq!(all.len(), 5);

        // Filter by status
        let active = service
            .list(None, Some(SessionStatus::Active), None)
            .await
            .unwrap();
        assert_eq!(active.len(), 3);

        // Filter by project
        let by_project = service.list(Some("project1"), None, None).await.unwrap();
        assert_eq!(by_project.len(), 5);

        // Test limit
        let limited = service.list(None, None, Some(3)).await.unwrap();
        assert_eq!(limited.len(), 3);
    }

    #[tokio::test]
    async fn test_validation_empty_session_id() {
        let service = create_test_service();

        assert!(service.get("").await.is_err());
        assert!(
            service
                .append_event(
                    "",
                    Event::new(
                        "evt".to_string(),
                        Utc::now(),
                        "msg".to_string(),
                        "user".to_string(),
                        HashMap::new()
                    ),
                    None
                )
                .await
                .is_err()
        );
        assert!(service.get_state("", "key").await.is_err());
        assert!(service.set_state("", "key", json!("val")).await.is_err());
    }
}

//! Mock substrate for testing.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    SubstrateOutput, SubstrateRequest, SubstrateSession,
};
use crate::domain::ports::Substrate;

/// Mock response configuration.
#[derive(Debug, Clone)]
pub struct MockResponse {
    /// Output text
    pub output: String,
    /// Whether to simulate failure
    pub fail: bool,
    /// Error message if failing
    pub error_message: Option<String>,
    /// Turns to simulate
    pub turns: u32,
    /// Input tokens per turn
    pub input_tokens_per_turn: u64,
    /// Output tokens per turn
    pub output_tokens_per_turn: u64,
}

impl Default for MockResponse {
    fn default() -> Self {
        Self {
            output: "Mock task completed successfully.".to_string(),
            fail: false,
            error_message: None,
            turns: 1,
            input_tokens_per_turn: 100,
            output_tokens_per_turn: 50,
        }
    }
}

impl MockResponse {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            ..Default::default()
        }
    }

    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            fail: true,
            error_message: Some(error.into()),
            ..Default::default()
        }
    }
}

/// Mock substrate for testing.
pub struct MockSubstrate {
    sessions: Arc<RwLock<HashMap<Uuid, SubstrateSession>>>,
    default_response: MockResponse,
    response_overrides: Arc<RwLock<HashMap<Uuid, MockResponse>>>,
}

impl MockSubstrate {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            default_response: MockResponse::default(),
            response_overrides: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_default_response(response: MockResponse) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            default_response: response,
            response_overrides: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set a specific response for a task ID.
    pub async fn set_response_for_task(&self, task_id: Uuid, response: MockResponse) {
        let mut overrides = self.response_overrides.write().await;
        overrides.insert(task_id, response);
    }

    /// Get the response for a task.
    async fn get_response(&self, task_id: Uuid) -> MockResponse {
        let overrides = self.response_overrides.read().await;
        overrides.get(&task_id).cloned().unwrap_or_else(|| self.default_response.clone())
    }

    /// Get all completed sessions.
    pub async fn get_all_sessions(&self) -> Vec<SubstrateSession> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Clear all sessions.
    pub async fn clear(&self) {
        let mut sessions = self.sessions.write().await;
        sessions.clear();
    }
}

impl Default for MockSubstrate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Substrate for MockSubstrate {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn is_available(&self) -> DomainResult<bool> {
        Ok(true)
    }

    async fn execute(&self, request: SubstrateRequest) -> DomainResult<SubstrateSession> {
        let response = self.get_response(request.task_id).await;
        let mut session = SubstrateSession::new(request.task_id, &request.agent_template, request.config);

        session.start(None);

        // Simulate turns
        for _ in 0..response.turns {
            session.record_turn(response.input_tokens_per_turn, response.output_tokens_per_turn);
        }

        // Complete or fail
        if response.fail {
            session.fail(response.error_message.unwrap_or_else(|| "Mock failure".to_string()));
        } else {
            session.complete(&response.output);
        }

        // Store session
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id, session.clone());

        Ok(session)
    }

    async fn execute_streaming(
        &self,
        request: SubstrateRequest,
    ) -> DomainResult<(mpsc::Receiver<SubstrateOutput>, SubstrateSession)> {
        let response = self.get_response(request.task_id).await;
        let mut session = SubstrateSession::new(request.task_id, &request.agent_template, request.config);
        session.start(None);

        let (tx, rx) = mpsc::channel(100);

        // Send mock events
        let response_clone = response.clone();
        tokio::spawn(async move {
            // Simulate turns
            for turn in 0..response_clone.turns {
                let _ = tx.send(SubstrateOutput::TurnComplete {
                    turn_number: turn + 1,
                    input_tokens: response_clone.input_tokens_per_turn,
                    output_tokens: response_clone.output_tokens_per_turn,
                }).await;
            }

            // Send result
            if response_clone.fail {
                let _ = tx.send(SubstrateOutput::Error {
                    message: response_clone.error_message.unwrap_or_else(|| "Mock failure".to_string()),
                }).await;
            } else {
                let _ = tx.send(SubstrateOutput::AssistantText {
                    content: response_clone.output.clone(),
                }).await;
                let _ = tx.send(SubstrateOutput::SessionComplete {
                    result: response_clone.output,
                }).await;
            }
        });

        // Store session
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id, session.clone());

        Ok((rx, session))
    }

    async fn resume(
        &self,
        session_id: Uuid,
        _additional_prompt: Option<String>,
    ) -> DomainResult<SubstrateSession> {
        let sessions = self.sessions.read().await;
        let original = sessions.get(&session_id)
            .ok_or_else(|| DomainError::ValidationFailed(format!("Session {} not found", session_id)))?;

        let mut session = SubstrateSession::new(original.task_id, &original.agent_template, original.config.clone());
        session.start(None);
        session.record_turn(100, 50);
        session.complete("Resumed and completed");

        drop(sessions);

        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id, session.clone());

        Ok(session)
    }

    async fn terminate(&self, session_id: Uuid) -> DomainResult<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.terminate();
        }
        Ok(())
    }

    async fn get_session(&self, session_id: Uuid) -> DomainResult<Option<SubstrateSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&session_id).cloned())
    }

    async fn is_running(&self, _session_id: Uuid) -> DomainResult<bool> {
        // Mock sessions complete immediately
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::SessionStatus;

    #[tokio::test]
    async fn test_mock_execute_success() {
        let substrate = MockSubstrate::new();
        let request = SubstrateRequest::new(
            Uuid::new_v4(),
            "test-agent",
            "System prompt",
            "User prompt",
        );

        let session = substrate.execute(request).await.unwrap();

        assert_eq!(session.status, SessionStatus::Completed);
        assert!(session.result.is_some());
        assert_eq!(session.turns_completed, 1);
    }

    #[tokio::test]
    async fn test_mock_execute_failure() {
        let substrate = MockSubstrate::with_default_response(MockResponse::failure("Test error"));
        let request = SubstrateRequest::new(
            Uuid::new_v4(),
            "test-agent",
            "System prompt",
            "User prompt",
        );

        let session = substrate.execute(request).await.unwrap();

        assert_eq!(session.status, SessionStatus::Failed);
        assert!(session.error.is_some());
    }

    #[tokio::test]
    async fn test_mock_custom_response() {
        let substrate = MockSubstrate::new();
        let task_id = Uuid::new_v4();

        substrate.set_response_for_task(task_id, MockResponse {
            output: "Custom output".to_string(),
            turns: 3,
            ..Default::default()
        }).await;

        let request = SubstrateRequest::new(
            task_id,
            "test-agent",
            "System prompt",
            "User prompt",
        );

        let session = substrate.execute(request).await.unwrap();

        assert_eq!(session.result, Some("Custom output".to_string()));
        assert_eq!(session.turns_completed, 3);
    }

    #[tokio::test]
    async fn test_mock_streaming() {
        let substrate = MockSubstrate::new();
        let request = SubstrateRequest::new(
            Uuid::new_v4(),
            "test-agent",
            "System prompt",
            "User prompt",
        );

        let (mut rx, _session) = substrate.execute_streaming(request).await.unwrap();

        let mut events = vec![];
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(!events.is_empty());
    }
}

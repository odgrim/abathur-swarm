//! Substrate port - interface for LLM backends.

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{SubstrateOutput, SubstrateRequest, SubstrateSession};

/// Trait for LLM substrate implementations.
///
/// A substrate is the underlying LLM backend that executes agent tasks.
/// Different substrates may use different APIs (Claude Code CLI, Anthropic API, etc.)
#[async_trait]
pub trait Substrate: Send + Sync {
    /// Get the substrate type name.
    fn name(&self) -> &'static str;

    /// Check if the substrate is available and properly configured.
    async fn is_available(&self) -> DomainResult<bool>;

    /// Execute a substrate request and return a session.
    ///
    /// This is a blocking call that runs the full session to completion.
    async fn execute(&self, request: SubstrateRequest) -> DomainResult<SubstrateSession>;

    /// Execute a substrate request with streaming output.
    ///
    /// Returns a channel receiver for streaming output events and
    /// a future that resolves to the final session state.
    async fn execute_streaming(
        &self,
        request: SubstrateRequest,
    ) -> DomainResult<(mpsc::Receiver<SubstrateOutput>, SubstrateSession)>;

    /// Resume an existing session.
    async fn resume(
        &self,
        session_id: Uuid,
        additional_prompt: Option<String>,
    ) -> DomainResult<SubstrateSession>;

    /// Terminate a running session.
    async fn terminate(&self, session_id: Uuid) -> DomainResult<()>;

    /// Terminate any running session associated with a task.
    ///
    /// Unlike `terminate()` which requires a session ID, this looks up the
    /// session by task ID. Used by timeout handlers that only know the task ID.
    /// Default implementation is a no-op for substrates that don't track sessions by task.
    async fn terminate_by_task_id(&self, _task_id: Uuid) -> DomainResult<()> {
        Ok(())
    }

    /// Get the status of a session.
    async fn get_session(&self, session_id: Uuid) -> DomainResult<Option<SubstrateSession>>;

    /// Check if a session is still running.
    async fn is_running(&self, session_id: Uuid) -> DomainResult<bool>;
}

/// Factory for creating substrate instances.
pub trait SubstrateFactory: Send + Sync {
    /// Create a substrate of the given type.
    fn create(&self, substrate_type: &str) -> Option<Box<dyn Substrate>>;

    /// List available substrate types.
    fn available_types(&self) -> Vec<&'static str>;
}

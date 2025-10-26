/// Session repository port (trait) for dependency injection.
///
/// Defines the contract for session storage operations that infrastructure
/// adapters must implement. Services depend on this trait, not concrete implementations.
use crate::domain::models::{Session, SessionStatus};
use anyhow::Result;
use async_trait::async_trait;

/// Repository trait for session persistence
///
/// Implementations should handle:
/// - JSON serialization/deserialization of events and state
/// - Concurrent access with appropriate locking
/// - Transaction management for atomic updates
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Creates a new session
    ///
    /// # Errors
    /// Returns error if:
    /// - Session ID already exists
    /// - Database connection fails
    /// - JSON serialization fails
    async fn create(&self, session: Session) -> Result<()>;

    /// Retrieves session by ID
    ///
    /// # Returns
    /// - `Some(Session)` if found
    /// - `None` if not found
    ///
    /// # Errors
    /// Returns error if:
    /// - Database connection fails
    /// - JSON deserialization fails
    async fn get(&self, session_id: &str) -> Result<Option<Session>>;

    /// Updates an existing session
    ///
    /// # Errors
    /// Returns error if:
    /// - Session not found
    /// - Database connection fails
    /// - JSON serialization fails
    async fn update(&self, session: Session) -> Result<()>;

    /// Lists sessions with optional filters
    ///
    /// # Arguments
    /// - `project_id`: Optional project ID filter
    /// - `status`: Optional status filter
    /// - `limit`: Maximum number of results
    ///
    /// # Errors
    /// Returns error if:
    /// - Database connection fails
    /// - JSON deserialization fails
    async fn list(
        &self,
        project_id: Option<&str>,
        status: Option<SessionStatus>,
        limit: usize,
    ) -> Result<Vec<Session>>;

    /// Deletes a session
    ///
    /// # Errors
    /// Returns error if:
    /// - Session not found
    /// - Database connection fails
    async fn delete(&self, session_id: &str) -> Result<()>;

    /// Checks if session exists
    ///
    /// # Errors
    /// Returns error if database connection fails
    async fn exists(&self, session_id: &str) -> Result<bool>;
}

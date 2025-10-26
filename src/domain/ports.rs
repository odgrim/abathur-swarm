use async_trait::async_trait;
use chrono::Duration;
use uuid::Uuid;

use super::models::{Agent, AgentStatus};

/// Error type for database operations
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Query failed: {0}")]
    QueryFailed(#[from] sqlx::Error),

    #[error("Agent not found: {0}")]
    NotFound(Uuid),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),
}

/// Repository interface for agent persistence operations
///
/// This trait defines the contract for agent data access following
/// the repository pattern and Clean Architecture principles.
#[async_trait]
pub trait AgentRepository: Send + Sync {
    /// Insert a new agent into the repository
    ///
    /// # Arguments
    /// * `agent` - The agent to insert
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(DatabaseError)` on failure (e.g., duplicate ID, constraint violation)
    async fn insert(&self, agent: Agent) -> Result<(), DatabaseError>;

    /// Get an agent by ID
    ///
    /// # Arguments
    /// * `id` - The agent UUID
    ///
    /// # Returns
    /// * `Ok(Some(agent))` if found
    /// * `Ok(None)` if not found
    /// * `Err(DatabaseError)` on query failure
    async fn get(&self, id: Uuid) -> Result<Option<Agent>, DatabaseError>;

    /// Update an existing agent
    ///
    /// # Arguments
    /// * `agent` - The agent with updated fields
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(DatabaseError)` on failure
    async fn update(&self, agent: Agent) -> Result<(), DatabaseError>;

    /// List agents, optionally filtered by status
    ///
    /// # Arguments
    /// * `status` - Optional status filter (None returns all agents)
    ///
    /// # Returns
    /// * `Ok(Vec<Agent>)` - List of matching agents
    /// * `Err(DatabaseError)` on query failure
    async fn list(&self, status: Option<AgentStatus>) -> Result<Vec<Agent>, DatabaseError>;

    /// Find stale agents based on heartbeat threshold
    ///
    /// Returns agents whose last heartbeat is older than the threshold.
    /// Used for detecting and cleaning up dead agents.
    ///
    /// # Arguments
    /// * `heartbeat_threshold` - Maximum age of heartbeat before considering agent stale
    ///
    /// # Returns
    /// * `Ok(Vec<Agent>)` - List of stale agents
    /// * `Err(DatabaseError)` on query failure
    async fn find_stale_agents(
        &self,
        heartbeat_threshold: Duration,
    ) -> Result<Vec<Agent>, DatabaseError>;

    /// Update an agent's heartbeat to current time
    ///
    /// Lightweight operation to update only the heartbeat timestamp
    /// without requiring a full agent update.
    ///
    /// # Arguments
    /// * `id` - The agent UUID
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(DatabaseError)` on failure (e.g., agent not found)
    async fn update_heartbeat(&self, id: Uuid) -> Result<(), DatabaseError>;
}

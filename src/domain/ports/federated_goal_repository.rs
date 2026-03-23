//! Federated goal repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::goal_federation::*;

/// Repository interface for federated goal persistence.
#[async_trait]
pub trait FederatedGoalRepository: Send + Sync {
    /// Save (insert or update) a federated goal.
    async fn save(&self, goal: &FederatedGoal) -> DomainResult<()>;

    /// Get a federated goal by its ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<FederatedGoal>>;

    /// Get all federated goals associated with a local goal.
    async fn get_by_local_goal(&self, local_goal_id: Uuid) -> DomainResult<Vec<FederatedGoal>>;

    /// Get all federated goals delegated to a specific cerebrate.
    async fn get_by_cerebrate(&self, cerebrate_id: &str) -> DomainResult<Vec<FederatedGoal>>;

    /// Get all federated goals in non-terminal states.
    async fn get_active(&self) -> DomainResult<Vec<FederatedGoal>>;

    /// Update only the state of a federated goal.
    async fn update_state(&self, id: Uuid, state: FederatedGoalState) -> DomainResult<()>;

    /// Update the convergence signals snapshot for a federated goal.
    async fn update_signals(&self, id: Uuid, signals: ConvergenceSignalSnapshot) -> DomainResult<()>;

    /// Delete a federated goal by ID.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;
}

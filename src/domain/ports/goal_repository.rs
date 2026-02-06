//! Goal repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{Goal, GoalStatus};

/// Filter criteria for listing goals.
#[derive(Debug, Clone, Default)]
pub struct GoalFilter {
    pub status: Option<GoalStatus>,
    pub priority: Option<crate::domain::models::GoalPriority>,
    pub parent_id: Option<Uuid>,
}

/// Repository interface for Goal persistence.
#[async_trait]
pub trait GoalRepository: Send + Sync {
    /// Create a new goal.
    async fn create(&self, goal: &Goal) -> DomainResult<()>;

    /// Get a goal by ID.
    async fn get(&self, id: Uuid) -> DomainResult<Option<Goal>>;

    /// Update an existing goal.
    async fn update(&self, goal: &Goal) -> DomainResult<()>;

    /// Delete a goal by ID.
    async fn delete(&self, id: Uuid) -> DomainResult<()>;

    /// List goals with optional filters.
    async fn list(&self, filter: GoalFilter) -> DomainResult<Vec<Goal>>;

    /// Get all child goals of a parent.
    async fn get_children(&self, parent_id: Uuid) -> DomainResult<Vec<Goal>>;

    /// Get active goals with their constraints.
    async fn get_active_with_constraints(&self) -> DomainResult<Vec<Goal>>;

    /// Count goals by status.
    async fn count_by_status(&self) -> DomainResult<std::collections::HashMap<GoalStatus, u64>>;

    /// Find active goals whose applicability_domains overlap with the given domains.
    async fn find_by_domains(&self, domains: &[String]) -> DomainResult<Vec<Goal>>;
}

//! Goal service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Goal, GoalConstraint, GoalPriority, GoalStatus};
use crate::domain::ports::{GoalFilter, GoalRepository};

pub struct GoalService<R: GoalRepository> {
    repository: Arc<R>,
}

impl<R: GoalRepository> GoalService<R> {
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    /// Create a new goal.
    pub async fn create_goal(
        &self,
        name: String,
        description: String,
        priority: GoalPriority,
        parent_id: Option<Uuid>,
        constraints: Vec<GoalConstraint>,
    ) -> DomainResult<Goal> {
        // Validate parent exists if specified
        if let Some(pid) = parent_id {
            let parent = self.repository.get(pid).await?;
            if parent.is_none() {
                return Err(DomainError::GoalNotFound(pid));
            }
        }

        let mut goal = Goal::new(name, description).with_priority(priority);

        if let Some(pid) = parent_id {
            goal = goal.with_parent(pid);
        }

        for constraint in constraints {
            goal = goal.with_constraint(constraint);
        }

        goal.validate().map_err(DomainError::ValidationFailed)?;
        self.repository.create(&goal).await?;

        Ok(goal)
    }

    /// Get a goal by ID.
    pub async fn get_goal(&self, id: Uuid) -> DomainResult<Option<Goal>> {
        self.repository.get(id).await
    }

    /// List goals with optional filters.
    pub async fn list_goals(&self, filter: GoalFilter) -> DomainResult<Vec<Goal>> {
        self.repository.list(filter).await
    }

    /// Transition a goal to a new status.
    pub async fn transition_status(&self, id: Uuid, new_status: GoalStatus) -> DomainResult<Goal> {
        let mut goal = self.repository.get(id).await?
            .ok_or(DomainError::GoalNotFound(id))?;

        goal.transition_to(new_status).map_err(|_| DomainError::InvalidStateTransition {
            from: goal.status.as_str().to_string(),
            to: new_status.as_str().to_string(),
        })?;

        self.repository.update(&goal).await?;
        Ok(goal)
    }

    /// Get effective constraints for a goal (including inherited from ancestors).
    pub async fn get_effective_constraints(&self, id: Uuid) -> DomainResult<Vec<GoalConstraint>> {
        let mut constraints = Vec::new();
        let mut current_id = Some(id);

        while let Some(gid) = current_id {
            let goal = self.repository.get(gid).await?
                .ok_or(DomainError::GoalNotFound(gid))?;

            // Add constraints (ancestors first, so child constraints can override)
            constraints.splice(0..0, goal.constraints.clone());
            current_id = goal.parent_id;
        }

        Ok(constraints)
    }

    /// Get active goals.
    pub async fn get_active_goals(&self) -> DomainResult<Vec<Goal>> {
        self.repository.get_active_with_constraints().await
    }

    /// Delete a goal.
    pub async fn delete_goal(&self, id: Uuid) -> DomainResult<()> {
        // Check for children
        let children = self.repository.get_children(id).await?;
        if !children.is_empty() {
            return Err(DomainError::ValidationFailed(
                "Cannot delete goal with children. Delete children first.".to_string()
            ));
        }

        self.repository.delete(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, goal_repository::SqliteGoalRepository, Migrator, all_embedded_migrations};

    async fn setup_service() -> GoalService<SqliteGoalRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
        let repo = Arc::new(SqliteGoalRepository::new(pool));
        GoalService::new(repo)
    }

    #[tokio::test]
    async fn test_create_goal() {
        let service = setup_service().await;
        let goal = service.create_goal(
            "Test".to_string(),
            "Description".to_string(),
            GoalPriority::High,
            None,
            vec![],
        ).await.unwrap();

        assert_eq!(goal.name, "Test");
        assert_eq!(goal.priority, GoalPriority::High);
    }

    #[tokio::test]
    async fn test_transition_status() {
        let service = setup_service().await;
        let goal = service.create_goal(
            "Test".to_string(),
            "Desc".to_string(),
            GoalPriority::Normal,
            None,
            vec![],
        ).await.unwrap();

        let updated = service.transition_status(goal.id, GoalStatus::Paused).await.unwrap();
        assert_eq!(updated.status, GoalStatus::Paused);
    }

    #[tokio::test]
    async fn test_effective_constraints() {
        let service = setup_service().await;

        let parent = service.create_goal(
            "Parent".to_string(),
            "Desc".to_string(),
            GoalPriority::Normal,
            None,
            vec![GoalConstraint::invariant("ParentConstraint", "From parent")],
        ).await.unwrap();

        let child = service.create_goal(
            "Child".to_string(),
            "Desc".to_string(),
            GoalPriority::Normal,
            Some(parent.id),
            vec![GoalConstraint::preference("ChildConstraint", "From child")],
        ).await.unwrap();

        let constraints = service.get_effective_constraints(child.id).await.unwrap();
        assert_eq!(constraints.len(), 2);
    }
}

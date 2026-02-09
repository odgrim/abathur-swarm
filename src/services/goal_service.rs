//! Goal service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Goal, GoalConstraint, GoalPriority, GoalStatus};
use crate::domain::ports::{GoalFilter, GoalRepository};
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};

pub struct GoalService<R: GoalRepository> {
    repository: Arc<R>,
    event_bus: Arc<EventBus>,
}

impl<R: GoalRepository> GoalService<R> {
    pub fn new(repository: Arc<R>, event_bus: Arc<EventBus>) -> Self {
        Self {
            repository,
            event_bus,
        }
    }

    async fn emit(&self, event: UnifiedEvent) {
        self.event_bus.publish(event).await;
    }

    /// Create a new goal.
    pub async fn create_goal(
        &self,
        name: String,
        description: String,
        priority: GoalPriority,
        parent_id: Option<Uuid>,
        constraints: Vec<GoalConstraint>,
        applicability_domains: Vec<String>,
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

        for domain in applicability_domains {
            goal = goal.with_applicability_domain(domain);
        }

        goal.validate().map_err(DomainError::ValidationFailed)?;
        self.repository.create(&goal).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Goal,
            goal_id: Some(goal.id),
            task_id: None,
            correlation_id: None,
            payload: EventPayload::GoalStarted {
                goal_id: goal.id,
                goal_name: goal.name.clone(),
            },
        }).await;

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

        let from_status = goal.status;
        goal.transition_to(new_status).map_err(|_| DomainError::InvalidStateTransition {
            from: from_status.as_str().to_string(),
            to: new_status.as_str().to_string(),
        })?;

        self.repository.update(&goal).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Goal,
            goal_id: Some(goal.id),
            task_id: None,
            correlation_id: None,
            payload: EventPayload::GoalStatusChanged {
                goal_id: goal.id,
                from_status: from_status.as_str().to_string(),
                to_status: new_status.as_str().to_string(),
            },
        }).await;

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

    /// Update the applicability domains of a goal.
    pub async fn update_domains(&self, id: Uuid, domains: Vec<String>) -> DomainResult<Goal> {
        let mut goal = self.repository.get(id).await?
            .ok_or(DomainError::GoalNotFound(id))?;

        let old_domains = goal.applicability_domains.clone();
        goal.applicability_domains = domains.clone();
        goal.updated_at = chrono::Utc::now();
        goal.version += 1;

        self.repository.update(&goal).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Goal,
            goal_id: Some(goal.id),
            task_id: None,
            correlation_id: None,
            payload: EventPayload::GoalDomainsUpdated {
                goal_id: goal.id,
                old_domains,
                new_domains: domains,
            },
        }).await;

        Ok(goal)
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

        let goal = self.repository.get(id).await?
            .ok_or(DomainError::GoalNotFound(id))?;
        let goal_name = goal.name.clone();

        self.repository.delete(id).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Goal,
            goal_id: Some(id),
            task_id: None,
            correlation_id: None,
            payload: EventPayload::GoalDeleted {
                goal_id: id,
                goal_name,
            },
        }).await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_migrated_test_pool, goal_repository::SqliteGoalRepository};
    use crate::services::event_bus::EventBusConfig;

    async fn setup_service() -> GoalService<SqliteGoalRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = Arc::new(SqliteGoalRepository::new(pool));
        let event_bus = Arc::new(EventBus::new(EventBusConfig::default()));
        GoalService::new(repo, event_bus)
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
            vec!["testing".to_string()],
        ).await.unwrap();

        let child = service.create_goal(
            "Child".to_string(),
            "Desc".to_string(),
            GoalPriority::Normal,
            Some(parent.id),
            vec![GoalConstraint::preference("ChildConstraint", "From child")],
            vec![],
        ).await.unwrap();

        let constraints = service.get_effective_constraints(child.id).await.unwrap();
        assert_eq!(constraints.len(), 2);
    }
}

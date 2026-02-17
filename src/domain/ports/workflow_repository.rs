//! Workflow repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::workflow::{
    PhaseInstance, WorkflowDefinition, WorkflowInstance, WorkflowStatus,
};

/// Repository interface for workflow persistence.
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    // -- Workflow Definitions --

    /// Save a workflow definition.
    async fn save_definition(&self, definition: &WorkflowDefinition) -> DomainResult<()>;

    /// Get a workflow definition by ID.
    async fn get_definition(&self, id: Uuid) -> DomainResult<Option<WorkflowDefinition>>;

    /// Get workflow definitions for a goal.
    async fn get_definitions_by_goal(&self, goal_id: Uuid) -> DomainResult<Vec<WorkflowDefinition>>;

    // -- Workflow Instances --

    /// Save a workflow instance.
    async fn save_instance(&self, instance: &WorkflowInstance) -> DomainResult<()>;

    /// Get a workflow instance by ID.
    async fn get_instance(&self, id: Uuid) -> DomainResult<Option<WorkflowInstance>>;

    /// Update a workflow instance.
    async fn update_instance(&self, instance: &WorkflowInstance) -> DomainResult<()>;

    /// Get workflow instances by status.
    async fn get_instances_by_status(
        &self,
        status: WorkflowStatus,
    ) -> DomainResult<Vec<WorkflowInstance>>;

    /// Get workflow instances for a goal.
    async fn get_instances_by_goal(&self, goal_id: Uuid) -> DomainResult<Vec<WorkflowInstance>>;

    // -- Phase Instances --

    /// Save a phase instance.
    async fn save_phase_instance(
        &self,
        workflow_instance_id: Uuid,
        phase_instance: &PhaseInstance,
    ) -> DomainResult<()>;

    /// Update a phase instance.
    async fn update_phase_instance(
        &self,
        workflow_instance_id: Uuid,
        phase_instance: &PhaseInstance,
    ) -> DomainResult<()>;

    /// Get all phase instances for a workflow instance.
    async fn get_phase_instances(
        &self,
        workflow_instance_id: Uuid,
    ) -> DomainResult<Vec<PhaseInstance>>;
}

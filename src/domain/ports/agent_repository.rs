//! Agent repository port.

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::{AgentInstance, AgentStatus, AgentTemplate, AgentTier, InstanceStatus};

/// Filter criteria for listing agents.
#[derive(Debug, Clone, Default)]
pub struct AgentFilter {
    pub tier: Option<AgentTier>,
    pub status: Option<AgentStatus>,
    pub name_pattern: Option<String>,
}

/// Repository interface for Agent persistence.
#[async_trait]
pub trait AgentRepository: Send + Sync {
    // Template operations

    /// Create a new agent template.
    async fn create_template(&self, template: &AgentTemplate) -> DomainResult<()>;

    /// Get an agent template by ID.
    async fn get_template(&self, id: Uuid) -> DomainResult<Option<AgentTemplate>>;

    /// Get an agent template by name (latest version).
    async fn get_template_by_name(&self, name: &str) -> DomainResult<Option<AgentTemplate>>;

    /// Get a specific version of an agent template.
    async fn get_template_version(&self, name: &str, version: u32) -> DomainResult<Option<AgentTemplate>>;

    /// Update an agent template.
    async fn update_template(&self, template: &AgentTemplate) -> DomainResult<()>;

    /// Delete an agent template.
    async fn delete_template(&self, id: Uuid) -> DomainResult<()>;

    /// List templates with optional filters.
    async fn list_templates(&self, filter: AgentFilter) -> DomainResult<Vec<AgentTemplate>>;

    /// List templates by tier.
    async fn list_by_tier(&self, tier: AgentTier) -> DomainResult<Vec<AgentTemplate>>;

    /// Get active templates.
    async fn get_active_templates(&self) -> DomainResult<Vec<AgentTemplate>>;

    // Instance operations

    /// Create a new agent instance.
    async fn create_instance(&self, instance: &AgentInstance) -> DomainResult<()>;

    /// Get an agent instance by ID.
    async fn get_instance(&self, id: Uuid) -> DomainResult<Option<AgentInstance>>;

    /// Update an agent instance.
    async fn update_instance(&self, instance: &AgentInstance) -> DomainResult<()>;

    /// Delete an agent instance.
    async fn delete_instance(&self, id: Uuid) -> DomainResult<()>;

    /// List instances by status.
    async fn list_instances_by_status(&self, status: InstanceStatus) -> DomainResult<Vec<AgentInstance>>;

    /// Get running instances for a template.
    async fn get_running_instances(&self, template_name: &str) -> DomainResult<Vec<AgentInstance>>;

    /// Count running instances by template.
    async fn count_running_by_template(&self) -> DomainResult<std::collections::HashMap<String, u32>>;
}

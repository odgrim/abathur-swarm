//! Agent service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    AgentConstraint, AgentInstance, AgentStatus, AgentTemplate, AgentTier,
    InstanceStatus, ToolCapability, specialist_templates,
};
use crate::domain::ports::{AgentFilter, AgentRepository};

pub struct AgentService<R: AgentRepository> {
    repository: Arc<R>,
}

impl<R: AgentRepository> AgentService<R> {
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    /// Register a new agent template.
    #[allow(clippy::too_many_arguments)]
    pub async fn register_template(
        &self,
        name: String,
        description: String,
        tier: AgentTier,
        system_prompt: String,
        tools: Vec<ToolCapability>,
        constraints: Vec<AgentConstraint>,
        max_turns: Option<u32>,
    ) -> DomainResult<AgentTemplate> {
        // Check if template with same name exists
        if let Some(existing) = self.repository.get_template_by_name(&name).await? {
            // Create a new version
            let mut template = AgentTemplate::new(&name, tier)
                .with_description(description)
                .with_prompt(system_prompt)
                .with_max_turns(max_turns.unwrap_or(25));

            template.version = existing.version + 1;

            for tool in tools {
                template = template.with_tool(tool);
            }
            for constraint in constraints {
                template = template.with_constraint(constraint);
            }

            template.validate().map_err(DomainError::ValidationFailed)?;
            self.repository.create_template(&template).await?;
            return Ok(template);
        }

        // Create first version
        let mut template = AgentTemplate::new(name, tier)
            .with_description(description)
            .with_prompt(system_prompt)
            .with_max_turns(max_turns.unwrap_or(25));

        for tool in tools {
            template = template.with_tool(tool);
        }
        for constraint in constraints {
            template = template.with_constraint(constraint);
        }

        template.validate().map_err(DomainError::ValidationFailed)?;
        self.repository.create_template(&template).await?;

        Ok(template)
    }

    /// Get a template by name (latest version).
    pub async fn get_template(&self, name: &str) -> DomainResult<Option<AgentTemplate>> {
        self.repository.get_template_by_name(name).await
    }

    /// Get a specific version of a template.
    pub async fn get_template_version(&self, name: &str, version: u32) -> DomainResult<Option<AgentTemplate>> {
        self.repository.get_template_version(name, version).await
    }

    /// List all templates.
    pub async fn list_templates(&self, filter: AgentFilter) -> DomainResult<Vec<AgentTemplate>> {
        self.repository.list_templates(filter).await
    }

    /// Get templates by tier.
    pub async fn list_by_tier(&self, tier: AgentTier) -> DomainResult<Vec<AgentTemplate>> {
        self.repository.list_by_tier(tier).await
    }

    /// Disable a template.
    pub async fn disable_template(&self, name: &str) -> DomainResult<AgentTemplate> {
        let mut template = self.repository.get_template_by_name(name).await?
            .ok_or_else(|| DomainError::AgentNotFound(name.to_string()))?;

        template.status = AgentStatus::Disabled;
        template.updated_at = chrono::Utc::now();
        self.repository.update_template(&template).await?;

        Ok(template)
    }

    /// Enable a template.
    pub async fn enable_template(&self, name: &str) -> DomainResult<AgentTemplate> {
        let mut template = self.repository.get_template_by_name(name).await?
            .ok_or_else(|| DomainError::AgentNotFound(name.to_string()))?;

        template.status = AgentStatus::Active;
        template.updated_at = chrono::Utc::now();
        self.repository.update_template(&template).await?;

        Ok(template)
    }

    /// Deprecate a template (no new tasks, existing can complete).
    pub async fn deprecate_template(&self, name: &str) -> DomainResult<AgentTemplate> {
        let mut template = self.repository.get_template_by_name(name).await?
            .ok_or_else(|| DomainError::AgentNotFound(name.to_string()))?;

        template.status = AgentStatus::Deprecated;
        template.updated_at = chrono::Utc::now();
        self.repository.update_template(&template).await?;

        Ok(template)
    }

    /// Spawn a new agent instance.
    pub async fn spawn_instance(&self, template_name: &str) -> DomainResult<AgentInstance> {
        let template = self.repository.get_template_by_name(template_name).await?
            .ok_or_else(|| DomainError::AgentNotFound(template_name.to_string()))?;

        if template.status != AgentStatus::Active {
            return Err(DomainError::ValidationFailed(
                format!("Template {} is not active", template_name)
            ));
        }

        // Check instance limit
        let running = self.repository.get_running_instances(template_name).await?;
        if running.len() as u32 >= template.tier.max_instances() {
            return Err(DomainError::ValidationFailed(
                format!("Max instances ({}) reached for {}", template.tier.max_instances(), template_name)
            ));
        }

        let instance = AgentInstance::from_template(&template);
        self.repository.create_instance(&instance).await?;

        Ok(instance)
    }

    /// Assign a task to an agent instance.
    pub async fn assign_task(&self, instance_id: Uuid, task_id: Uuid) -> DomainResult<AgentInstance> {
        let mut instance = self.repository.get_instance(instance_id).await?
            .ok_or_else(|| DomainError::AgentNotFound(instance_id.to_string()))?;

        if instance.status != InstanceStatus::Idle {
            return Err(DomainError::ValidationFailed(
                "Instance is not idle".to_string()
            ));
        }

        instance.assign_task(task_id);
        self.repository.update_instance(&instance).await?;

        Ok(instance)
    }

    /// Record a turn for an agent instance.
    pub async fn record_turn(&self, instance_id: Uuid) -> DomainResult<AgentInstance> {
        let mut instance = self.repository.get_instance(instance_id).await?
            .ok_or_else(|| DomainError::AgentNotFound(instance_id.to_string()))?;

        instance.record_turn();
        self.repository.update_instance(&instance).await?;

        Ok(instance)
    }

    /// Complete an agent instance's task.
    pub async fn complete_instance(&self, instance_id: Uuid) -> DomainResult<AgentInstance> {
        let mut instance = self.repository.get_instance(instance_id).await?
            .ok_or_else(|| DomainError::AgentNotFound(instance_id.to_string()))?;

        instance.complete();
        self.repository.update_instance(&instance).await?;

        Ok(instance)
    }

    /// Fail an agent instance.
    pub async fn fail_instance(&self, instance_id: Uuid) -> DomainResult<AgentInstance> {
        let mut instance = self.repository.get_instance(instance_id).await?
            .ok_or_else(|| DomainError::AgentNotFound(instance_id.to_string()))?;

        instance.fail();
        self.repository.update_instance(&instance).await?;

        Ok(instance)
    }

    /// Get running instances.
    pub async fn get_running_instances(&self) -> DomainResult<Vec<AgentInstance>> {
        self.repository.list_instances_by_status(InstanceStatus::Running).await
    }

    /// Get instance counts by template.
    pub async fn get_instance_counts(&self) -> DomainResult<std::collections::HashMap<String, u32>> {
        self.repository.count_running_by_template().await
    }

    /// Check if can spawn more instances of a template.
    pub async fn can_spawn(&self, template_name: &str) -> DomainResult<bool> {
        let template = self.repository.get_template_by_name(template_name).await?
            .ok_or_else(|| DomainError::AgentNotFound(template_name.to_string()))?;

        if template.status != AgentStatus::Active {
            return Ok(false);
        }

        let running = self.repository.get_running_instances(template_name).await?;
        Ok((running.len() as u32) < template.tier.max_instances())
    }

    /// Select best agent for a task based on capabilities.
    pub async fn select_agent_for_task(
        &self,
        required_tools: &[String],
        preferred_tier: Option<AgentTier>,
    ) -> DomainResult<Option<AgentTemplate>> {
        let filter = AgentFilter {
            tier: preferred_tier,
            status: Some(AgentStatus::Active),
            ..Default::default()
        };

        let templates = self.repository.list_templates(filter).await?;

        // Find template that has all required tools
        for template in templates {
            let has_all_tools = required_tools.iter().all(|tool| template.has_tool(tool));
            if has_all_tools && self.can_spawn(&template.name).await? {
                return Ok(Some(template));
            }
        }

        Ok(None)
    }

    /// Load baseline specialist templates.
    ///
    /// Returns the number of templates created (skips existing ones).
    /// Currently returns 0 since all specialists are created dynamically
    /// by the Overmind at runtime.
    pub async fn load_baseline_specialists(&self) -> DomainResult<usize> {
        let specialists = specialist_templates::create_baseline_specialists();
        let mut created = 0;

        for template in specialists {
            // Check if already exists
            if self.repository.get_template_by_name(&template.name).await?.is_none() {
                self.repository.create_template(&template).await?;
                created += 1;
                tracing::info!("Registered baseline specialist: {}", template.name);
            }
        }

        if created > 0 {
            tracing::info!("Loaded {} baseline specialist templates", created);
        }

        Ok(created)
    }

    /// Get a specialist by capability.
    pub async fn get_specialist_by_capability(
        &self,
        capability: &str,
    ) -> DomainResult<Option<AgentTemplate>> {
        let filter = AgentFilter {
            tier: Some(AgentTier::Specialist),
            status: Some(AgentStatus::Active),
            ..Default::default()
        };

        let templates = self.repository.list_templates(filter).await?;

        // Find specialist with matching capability
        for template in templates {
            if template.agent_card.capabilities.iter().any(|c| c == capability) {
                return Ok(Some(template));
            }
        }

        Ok(None)
    }

    /// Seed all baseline specialist templates if they don't exist.
    ///
    /// Currently a no-op since all specialists are created dynamically
    /// by the Overmind at runtime.
    pub async fn seed_baseline_specialists(&self) -> DomainResult<Vec<String>> {
        let baseline = specialist_templates::create_baseline_specialists();
        let mut seeded = Vec::new();

        for template in baseline {
            // Check if template already exists
            if self.repository.get_template_by_name(&template.name).await?.is_none() {
                // Create the template
                self.repository.create_template(&template).await?;
                seeded.push(template.name.clone());
            }
        }

        Ok(seeded)
    }

    /// Ensure a specific specialist template exists, creating if needed.
    ///
    /// With the overmind-only model, this will only find specialists that
    /// have been dynamically created at runtime (no baseline specialists exist).
    pub async fn ensure_specialist(&self, name: &str) -> DomainResult<AgentTemplate> {
        // Check if exists in database (dynamically created agents)
        if let Some(template) = self.repository.get_template_by_name(name).await? {
            return Ok(template);
        }

        Err(DomainError::AgentNotFound(format!("Agent '{}' not found. Create it via the Agents REST API.", name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_test_pool, SqliteAgentRepository, Migrator, all_embedded_migrations};

    async fn setup_service() -> AgentService<SqliteAgentRepository> {
        let pool = create_test_pool().await.unwrap();
        let migrator = Migrator::new(pool.clone());
        migrator.run_embedded_migrations(all_embedded_migrations()).await.unwrap();
        let repo = Arc::new(SqliteAgentRepository::new(pool));
        AgentService::new(repo)
    }

    #[tokio::test]
    async fn test_register_template() {
        let service = setup_service().await;

        let template = service.register_template(
            "test-agent".to_string(),
            "A test agent".to_string(),
            AgentTier::Worker,
            "You are a test agent.".to_string(),
            vec![ToolCapability::new("read", "Read files")],
            vec![],
            None,
        ).await.unwrap();

        assert_eq!(template.name, "test-agent");
        assert_eq!(template.version, 1);
    }

    #[tokio::test]
    async fn test_version_increment() {
        let service = setup_service().await;

        // Create first version
        service.register_template(
            "versioned".to_string(),
            "V1".to_string(),
            AgentTier::Worker,
            "Version 1".to_string(),
            vec![],
            vec![],
            None,
        ).await.unwrap();

        // Create second version
        let v2 = service.register_template(
            "versioned".to_string(),
            "V2".to_string(),
            AgentTier::Worker,
            "Version 2".to_string(),
            vec![],
            vec![],
            None,
        ).await.unwrap();

        assert_eq!(v2.version, 2);
        assert_eq!(v2.system_prompt, "Version 2");
    }

    #[tokio::test]
    async fn test_spawn_instance() {
        let service = setup_service().await;

        service.register_template(
            "spawnable".to_string(),
            "Test".to_string(),
            AgentTier::Worker,
            "Test prompt".to_string(),
            vec![],
            vec![],
            None,
        ).await.unwrap();

        let instance = service.spawn_instance("spawnable").await.unwrap();
        assert_eq!(instance.status, InstanceStatus::Idle);
        assert_eq!(instance.template_name, "spawnable");
    }

    #[tokio::test]
    async fn test_instance_lifecycle() {
        let service = setup_service().await;

        service.register_template(
            "lifecycle".to_string(),
            "Test".to_string(),
            AgentTier::Worker,
            "Test prompt".to_string(),
            vec![],
            vec![],
            None,
        ).await.unwrap();

        let instance = service.spawn_instance("lifecycle").await.unwrap();
        let task_id = Uuid::new_v4();

        let assigned = service.assign_task(instance.id, task_id).await.unwrap();
        assert_eq!(assigned.status, InstanceStatus::Running);
        assert_eq!(assigned.current_task_id, Some(task_id));

        service.record_turn(instance.id).await.unwrap();
        service.record_turn(instance.id).await.unwrap();

        let completed = service.complete_instance(instance.id).await.unwrap();
        assert_eq!(completed.status, InstanceStatus::Completed);
        assert_eq!(completed.turn_count, 2);
    }

    #[tokio::test]
    async fn test_disable_enable() {
        let service = setup_service().await;

        service.register_template(
            "toggleable".to_string(),
            "Test".to_string(),
            AgentTier::Worker,
            "Test prompt".to_string(),
            vec![],
            vec![],
            None,
        ).await.unwrap();

        let disabled = service.disable_template("toggleable").await.unwrap();
        assert_eq!(disabled.status, AgentStatus::Disabled);

        // Should fail to spawn when disabled
        let spawn_result = service.spawn_instance("toggleable").await;
        assert!(spawn_result.is_err());

        let enabled = service.enable_template("toggleable").await.unwrap();
        assert_eq!(enabled.status, AgentStatus::Active);

        // Should succeed now
        let instance = service.spawn_instance("toggleable").await;
        assert!(instance.is_ok());
    }
}

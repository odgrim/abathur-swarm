//! Agent service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    AgentConstraint, AgentInstance, AgentStatus, AgentTemplate, AgentTier,
    InstanceStatus, ToolCapability, specialist_templates,
    workflow_template::WorkflowTemplate,
};
use crate::domain::ports::{AgentFilter, AgentRepository};
use crate::services::event_bus::{
    EventBus, EventCategory, EventId, EventPayload, EventSeverity, SequenceNumber, UnifiedEvent,
};

pub struct AgentService<R: AgentRepository> {
    repository: Arc<R>,
    event_bus: Arc<EventBus>,
}

impl<R: AgentRepository> AgentService<R> {
    pub fn new(repository: Arc<R>, event_bus: Arc<EventBus>) -> Self {
        Self {
            repository,
            event_bus,
        }
    }

    async fn emit(&self, event: UnifiedEvent) {
        self.event_bus.publish(event).await;
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
        read_only: bool,
    ) -> DomainResult<AgentTemplate> {
        // Check if template with same name exists
        if let Some(existing) = self.repository.get_template_by_name(&name).await? {
            // Create a new version
            let floor = tier.max_turns();
            let effective_turns = max_turns.map_or(floor, |t| t.max(floor));
            let mut template = AgentTemplate::new(&name, tier)
                .with_description(description)
                .with_prompt(system_prompt)
                .with_max_turns(effective_turns)
                .with_read_only(read_only);

            template.version = existing.version + 1;

            for tool in tools {
                template = template.with_tool(tool);
            }
            for constraint in constraints {
                template = template.with_constraint(constraint);
            }

            template.validate().map_err(DomainError::ValidationFailed)?;
            self.repository.create_template(&template).await?;

            self.emit(UnifiedEvent {
                id: EventId::new(),
                sequence: SequenceNumber(0),
                timestamp: chrono::Utc::now(),
                severity: EventSeverity::Info,
                category: EventCategory::Agent,
                goal_id: None,
                task_id: None,
                correlation_id: None,
                source_process_id: None,
                payload: EventPayload::AgentTemplateRegistered {
                    template_name: template.name.clone(),
                    tier: format!("{:?}", template.tier),
                    version: template.version,
                },
            }).await;

            return Ok(template);
        }

        // Create first version
        let floor = tier.max_turns();
        let effective_turns = max_turns.map_or(floor, |t| t.max(floor));
        let mut template = AgentTemplate::new(name, tier)
            .with_description(description)
            .with_prompt(system_prompt)
            .with_max_turns(effective_turns)
            .with_read_only(read_only);

        for tool in tools {
            template = template.with_tool(tool);
        }
        for constraint in constraints {
            template = template.with_constraint(constraint);
        }

        template.validate().map_err(DomainError::ValidationFailed)?;
        self.repository.create_template(&template).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::AgentTemplateRegistered {
                template_name: template.name.clone(),
                tier: format!("{:?}", template.tier),
                version: template.version,
            },
        }).await;

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

    /// Set a template's status (active, disabled, deprecated).
    pub async fn set_template_status(&self, name: &str, status: AgentStatus) -> DomainResult<AgentTemplate> {
        let mut template = self.repository.get_template_by_name(name).await?
            .ok_or_else(|| DomainError::AgentNotFound(name.to_string()))?;

        let from_status = format!("{:?}", template.status);
        template.status = status;
        template.updated_at = chrono::Utc::now();
        self.repository.update_template(&template).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::AgentTemplateStatusChanged {
                template_name: name.to_string(),
                from_status,
                to_status: format!("{:?}", status),
            },
        }).await;

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

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: None,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::AgentInstanceSpawned {
                instance_id: instance.id,
                template_name: template.name.clone(),
                tier: format!("{:?}", template.tier),
            },
        }).await;

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

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Debug,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: Some(task_id),
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::AgentInstanceAssigned {
                instance_id: instance.id,
                task_id,
                template_name: instance.template_name.clone(),
            },
        }).await;

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

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Info,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: instance.current_task_id,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::AgentInstanceCompleted {
                instance_id: instance.id,
                task_id: instance.current_task_id.unwrap_or(Uuid::nil()),
                tokens_used: 0,
            },
        }).await;

        Ok(instance)
    }

    /// Fail an agent instance.
    pub async fn fail_instance(&self, instance_id: Uuid) -> DomainResult<AgentInstance> {
        let mut instance = self.repository.get_instance(instance_id).await?
            .ok_or_else(|| DomainError::AgentNotFound(instance_id.to_string()))?;

        instance.fail();
        self.repository.update_instance(&instance).await?;

        self.emit(UnifiedEvent {
            id: EventId::new(),
            sequence: SequenceNumber(0),
            timestamp: chrono::Utc::now(),
            severity: EventSeverity::Error,
            category: EventCategory::Agent,
            goal_id: None,
            task_id: instance.current_task_id,
            correlation_id: None,
            source_process_id: None,
            payload: EventPayload::AgentInstanceFailed {
                instance_id: instance.id,
                task_id: instance.current_task_id,
                template_name: instance.template_name.clone(),
            },
        }).await;

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

    /// Seed baseline agent templates from hardcoded definitions.
    ///
    /// The DB is the sole source of agent definitions at runtime.
    /// Hardcoded templates in `specialist_templates` serve as bootstrap.
    ///
    /// - If a template is missing from the DB: insert it.
    /// - If the hardcoded version is newer than the DB version: update it.
    /// - If the DB version >= hardcoded version: skip (no downgrade).
    pub async fn seed_baseline_agents(&self) -> DomainResult<Vec<String>> {
        let baseline = specialist_templates::create_baseline_agents();
        let mut seeded = Vec::new();

        for template in baseline {
            match self.repository.get_template_by_name(&template.name).await? {
                None => {
                    self.repository.create_template(&template).await?;
                    seeded.push(template.name.clone());
                    tracing::info!("Seeded baseline agent '{}'", template.name);
                }
                Some(existing) if template.version > existing.version => {
                    // Upgrade: hardcoded version is newer
                    let mut upgraded = template.clone();
                    upgraded.id = existing.id;
                    self.repository.update_template(&upgraded).await?;
                    seeded.push(upgraded.name.clone());
                    tracing::info!(
                        "Upgraded baseline agent '{}' from v{} to v{}",
                        upgraded.name, existing.version, upgraded.version
                    );
                }
                Some(_) => {
                    // DB version is current or newer, skip
                }
            }
        }

        Ok(seeded)
    }

    /// Seed baseline agent templates with an optional workflow template.
    ///
    /// Like [`seed_baseline_agents`], but passes the workflow template to the
    /// overmind prompt generator so it knows the phase sequence.
    pub async fn seed_baseline_agents_with_workflow(
        &self,
        workflow: Option<&WorkflowTemplate>,
    ) -> DomainResult<Vec<String>> {
        let baseline = specialist_templates::create_baseline_agents_with_workflow(workflow);
        let mut seeded = Vec::new();

        for template in baseline {
            match self.repository.get_template_by_name(&template.name).await? {
                None => {
                    self.repository.create_template(&template).await?;
                    seeded.push(template.name.clone());
                    tracing::info!("Seeded baseline agent '{}'", template.name);
                }
                Some(existing) if template.version > existing.version => {
                    // Upgrade: hardcoded version is newer
                    let mut upgraded = template.clone();
                    upgraded.id = existing.id;
                    self.repository.update_template(&upgraded).await?;
                    seeded.push(upgraded.name.clone());
                    tracing::info!(
                        "Upgraded baseline agent '{}' from v{} to v{}",
                        upgraded.name, existing.version, upgraded.version
                    );
                }
                Some(_) => {
                    // DB version is current or newer, skip
                }
            }
        }

        Ok(seeded)
    }

    /// Seed baseline agent templates with awareness of all configured workflow spines.
    ///
    /// Like [`seed_baseline_agents_with_workflow`], but generates a routing-aware Overmind
    /// prompt that describes every provided workflow so the Overmind can select the
    /// appropriate spine at runtime based on task content.
    pub async fn seed_baseline_agents_with_workflows(
        &self,
        workflows: &[WorkflowTemplate],
    ) -> DomainResult<Vec<String>> {
        let baseline = specialist_templates::create_baseline_agents_with_workflows(workflows);
        let mut seeded = Vec::new();

        for template in baseline {
            match self.repository.get_template_by_name(&template.name).await? {
                None => {
                    self.repository.create_template(&template).await?;
                    seeded.push(template.name.clone());
                    tracing::info!("Seeded baseline agent '{}'", template.name);
                }
                Some(existing) if template.version > existing.version => {
                    let mut upgraded = template.clone();
                    upgraded.id = existing.id;
                    self.repository.update_template(&upgraded).await?;
                    seeded.push(upgraded.name.clone());
                    tracing::info!(
                        "Upgraded baseline agent '{}' from v{} to v{}",
                        upgraded.name, existing.version, upgraded.version
                    );
                }
                Some(_) => {
                    // DB version is current or newer, skip
                }
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
    use crate::adapters::sqlite::{create_migrated_test_pool, SqliteAgentRepository};

    async fn setup_service() -> AgentService<SqliteAgentRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        let repo = Arc::new(SqliteAgentRepository::new(pool));
        let event_bus = Arc::new(EventBus::new(crate::services::event_bus::EventBusConfig::default()));
        AgentService::new(repo, event_bus)
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
            false,
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
            false,
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
            false,
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
            false,
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
            false,
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
    async fn test_seed_baseline_agents_inserts_missing() {
        let service = setup_service().await;

        let seeded = service.seed_baseline_agents().await.unwrap();
        assert!(seeded.contains(&"overmind".to_string()));

        // Verify it's in the DB
        let template = service.get_template("overmind").await.unwrap().unwrap();
        assert_eq!(template.name, "overmind");
        assert!(template.has_tool("agents"));
    }

    #[tokio::test]
    async fn test_seed_baseline_agents_upgrades_older_version() {
        let service = setup_service().await;

        // Insert a v1 overmind manually
        let mut old = specialist_templates::create_overmind();
        old.version = 1;
        service.repository.create_template(&old).await.unwrap();

        // Seed should upgrade to v3
        let seeded = service.seed_baseline_agents().await.unwrap();
        assert!(seeded.contains(&"overmind".to_string()));

        let template = service.get_template("overmind").await.unwrap().unwrap();
        assert_eq!(template.version, 3);
    }

    #[tokio::test]
    async fn test_seed_baseline_agents_no_downgrade() {
        let service = setup_service().await;

        // Insert a v99 overmind (future version)
        let mut future = specialist_templates::create_overmind();
        future.version = 99;
        service.repository.create_template(&future).await.unwrap();

        // Seed should NOT downgrade
        let seeded = service.seed_baseline_agents().await.unwrap();
        assert!(seeded.is_empty());

        let template = service.get_template("overmind").await.unwrap().unwrap();
        assert_eq!(template.version, 99);
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
            false,
        ).await.unwrap();

        let disabled = service.set_template_status("toggleable", AgentStatus::Disabled).await.unwrap();
        assert_eq!(disabled.status, AgentStatus::Disabled);

        // Should fail to spawn when disabled
        let spawn_result = service.spawn_instance("toggleable").await;
        assert!(spawn_result.is_err());

        let enabled = service.set_template_status("toggleable", AgentStatus::Active).await.unwrap();
        assert_eq!(enabled.status, AgentStatus::Active);

        // Should succeed now
        let instance = service.spawn_instance("toggleable").await;
        assert!(instance.is_ok());
    }
}

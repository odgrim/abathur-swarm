//! SQLite implementation of the AgentRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    AgentCard, AgentConstraint, AgentInstance, AgentStatus, AgentTemplate, AgentTier,
    InstanceStatus, ToolCapability,
};
use crate::domain::ports::{AgentFilter, AgentRepository};

#[derive(Clone)]
pub struct SqliteAgentRepository {
    pool: SqlitePool,
}

impl SqliteAgentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentRepository for SqliteAgentRepository {
    async fn create_template(&self, template: &AgentTemplate) -> DomainResult<()> {
        let tools_json = serde_json::to_string(&template.tools)?;
        let constraints_json = serde_json::to_string(&template.constraints)?;
        let handoff_json = serde_json::to_string(&template.agent_card.handoff_targets)?;

        sqlx::query(
            r#"INSERT INTO agent_templates (id, name, description, tier, version, system_prompt,
               tools, constraints, handoff_targets, max_turns, is_active, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#
        )
        .bind(template.id.to_string())
        .bind(&template.name)
        .bind(&template.description)
        .bind(template.tier.as_str())
        .bind(template.version as i32)
        .bind(&template.system_prompt)
        .bind(&tools_json)
        .bind(&constraints_json)
        .bind(&handoff_json)
        .bind(template.max_turns as i32)
        .bind(if template.status == AgentStatus::Active { 1i32 } else { 0i32 })
        .bind(template.created_at.to_rfc3339())
        .bind(template.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_template(&self, id: Uuid) -> DomainResult<Option<AgentTemplate>> {
        let row: Option<TemplateRow> = sqlx::query_as(
            "SELECT * FROM agent_templates WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_template_by_name(&self, name: &str) -> DomainResult<Option<AgentTemplate>> {
        let row: Option<TemplateRow> = sqlx::query_as(
            "SELECT * FROM agent_templates WHERE name = ? ORDER BY version DESC LIMIT 1"
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn get_template_version(&self, name: &str, version: u32) -> DomainResult<Option<AgentTemplate>> {
        let row: Option<TemplateRow> = sqlx::query_as(
            "SELECT * FROM agent_templates WHERE name = ? AND version = ?"
        )
        .bind(name)
        .bind(version as i32)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn update_template(&self, template: &AgentTemplate) -> DomainResult<()> {
        let tools_json = serde_json::to_string(&template.tools)?;
        let constraints_json = serde_json::to_string(&template.constraints)?;
        let handoff_json = serde_json::to_string(&template.agent_card.handoff_targets)?;

        let result = sqlx::query(
            r#"UPDATE agent_templates SET name = ?, description = ?, tier = ?, version = ?,
               system_prompt = ?, tools = ?, constraints = ?, handoff_targets = ?,
               max_turns = ?, is_active = ?, updated_at = ?
               WHERE id = ?"#
        )
        .bind(&template.name)
        .bind(&template.description)
        .bind(template.tier.as_str())
        .bind(template.version as i32)
        .bind(&template.system_prompt)
        .bind(&tools_json)
        .bind(&constraints_json)
        .bind(&handoff_json)
        .bind(template.max_turns as i32)
        .bind(if template.status == AgentStatus::Active { 1i32 } else { 0i32 })
        .bind(template.updated_at.to_rfc3339())
        .bind(template.id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::AgentNotFound(template.name.clone()));
        }

        Ok(())
    }

    async fn delete_template(&self, id: Uuid) -> DomainResult<()> {
        let result = sqlx::query("DELETE FROM agent_templates WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::AgentNotFound(id.to_string()));
        }

        Ok(())
    }

    async fn list_templates(&self, filter: AgentFilter) -> DomainResult<Vec<AgentTemplate>> {
        let mut sql = String::from("SELECT * FROM agent_templates WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(tier) = &filter.tier {
            sql.push_str(" AND tier = ?");
            bindings.push(tier.as_str().to_string());
        }
        if let Some(status) = &filter.status {
            sql.push_str(" AND is_active = ?");
            bindings.push(if *status == AgentStatus::Active { "1" } else { "0" }.to_string());
        }
        if let Some(pattern) = &filter.name_pattern {
            sql.push_str(" AND name LIKE ?");
            bindings.push(pattern.replace('*', "%"));
        }

        sql.push_str(" ORDER BY name, version DESC");

        let mut q = sqlx::query_as::<_, TemplateRow>(&sql);
        for binding in &bindings {
            q = q.bind(binding);
        }

        let rows: Vec<TemplateRow> = q.fetch_all(&self.pool).await?;
        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn list_by_tier(&self, tier: AgentTier) -> DomainResult<Vec<AgentTemplate>> {
        self.list_templates(AgentFilter { tier: Some(tier), ..Default::default() }).await
    }

    async fn get_active_templates(&self) -> DomainResult<Vec<AgentTemplate>> {
        self.list_templates(AgentFilter { status: Some(AgentStatus::Active), ..Default::default() }).await
    }

    // Instance operations

    async fn create_instance(&self, instance: &AgentInstance) -> DomainResult<()> {
        sqlx::query(
            r#"INSERT INTO agent_instances (id, template_id, template_name, current_task_id,
               turn_count, status, started_at, completed_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#
        )
        .bind(instance.id.to_string())
        .bind(instance.template_id.to_string())
        .bind(&instance.template_name)
        .bind(instance.current_task_id.map(|id| id.to_string()))
        .bind(instance.turn_count as i32)
        .bind(instance.status.as_str())
        .bind(instance.started_at.to_rfc3339())
        .bind(instance.completed_at.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_instance(&self, id: Uuid) -> DomainResult<Option<AgentInstance>> {
        let row: Option<InstanceRow> = sqlx::query_as(
            "SELECT * FROM agent_instances WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.try_into()).transpose()
    }

    async fn update_instance(&self, instance: &AgentInstance) -> DomainResult<()> {
        let result = sqlx::query(
            r#"UPDATE agent_instances SET current_task_id = ?, turn_count = ?,
               status = ?, completed_at = ?
               WHERE id = ?"#
        )
        .bind(instance.current_task_id.map(|id| id.to_string()))
        .bind(instance.turn_count as i32)
        .bind(instance.status.as_str())
        .bind(instance.completed_at.map(|t| t.to_rfc3339()))
        .bind(instance.id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::AgentNotFound(instance.id.to_string()));
        }

        Ok(())
    }

    async fn delete_instance(&self, id: Uuid) -> DomainResult<()> {
        let result = sqlx::query("DELETE FROM agent_instances WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DomainError::AgentNotFound(id.to_string()));
        }

        Ok(())
    }

    async fn list_instances_by_status(&self, status: InstanceStatus) -> DomainResult<Vec<AgentInstance>> {
        let rows: Vec<InstanceRow> = sqlx::query_as(
            "SELECT * FROM agent_instances WHERE status = ? ORDER BY started_at DESC"
        )
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn get_running_instances(&self, template_name: &str) -> DomainResult<Vec<AgentInstance>> {
        let rows: Vec<InstanceRow> = sqlx::query_as(
            "SELECT * FROM agent_instances WHERE template_name = ? AND status = 'running'"
        )
        .bind(template_name)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into()).collect()
    }

    async fn count_running_by_template(&self) -> DomainResult<HashMap<String, u32>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT template_name, COUNT(*) FROM agent_instances WHERE status = 'running' GROUP BY template_name"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(name, count)| (name, count as u32)).collect())
    }
}

#[derive(sqlx::FromRow)]
struct TemplateRow {
    id: String,
    name: String,
    description: Option<String>,
    tier: String,
    version: i32,
    system_prompt: String,
    tools: Option<String>,
    constraints: Option<String>,
    handoff_targets: Option<String>,
    max_turns: i32,
    is_active: i32,
    created_at: String,
    updated_at: String,
}

impl TryFrom<TemplateRow> for AgentTemplate {
    type Error = DomainError;

    fn try_from(row: TemplateRow) -> Result<Self, Self::Error> {
        let id = super::parse_uuid(&row.id)?;

        let tier = AgentTier::parse_str(&row.tier)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid tier: {}", row.tier)))?;

        let tools: Vec<ToolCapability> = super::parse_json_or_default(row.tools)?;
        let constraints: Vec<AgentConstraint> = super::parse_json_or_default(row.constraints)?;
        let handoff_targets: Vec<String> = super::parse_json_or_default(row.handoff_targets)?;

        let created_at = super::parse_datetime(&row.created_at)?;
        let updated_at = super::parse_datetime(&row.updated_at)?;

        let status = if row.is_active != 0 {
            AgentStatus::Active
        } else {
            AgentStatus::Disabled
        };

        Ok(AgentTemplate {
            id,
            name: row.name,
            description: row.description.unwrap_or_default(),
            tier,
            version: row.version as u32,
            system_prompt: row.system_prompt,
            tools,
            constraints,
            agent_card: AgentCard {
                handoff_targets,
                ..Default::default()
            },
            max_turns: row.max_turns as u32,
            status,
            created_at,
            updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct InstanceRow {
    id: String,
    template_id: String,
    template_name: String,
    current_task_id: Option<String>,
    turn_count: i32,
    status: String,
    started_at: String,
    completed_at: Option<String>,
}

impl TryFrom<InstanceRow> for AgentInstance {
    type Error = DomainError;

    fn try_from(row: InstanceRow) -> Result<Self, Self::Error> {
        let id = super::parse_uuid(&row.id)?;
        let template_id = super::parse_uuid(&row.template_id)?;
        let current_task_id = super::parse_optional_uuid(row.current_task_id)?;

        let status = InstanceStatus::parse_str(&row.status)
            .ok_or_else(|| DomainError::SerializationError(format!("Invalid status: {}", row.status)))?;

        let started_at = super::parse_datetime(&row.started_at)?;
        let completed_at = super::parse_optional_datetime(row.completed_at)?;

        Ok(AgentInstance {
            id,
            template_id,
            template_name: row.template_name,
            current_task_id,
            turn_count: row.turn_count as u32,
            status,
            started_at,
            completed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::create_migrated_test_pool;

    async fn setup_test_repo() -> SqliteAgentRepository {
        let pool = create_migrated_test_pool().await.unwrap();
        SqliteAgentRepository::new(pool)
    }

    #[tokio::test]
    async fn test_create_and_get_template() {
        let repo = setup_test_repo().await;

        let template = AgentTemplate::new("test-agent", AgentTier::Worker)
            .with_description("A test agent")
            .with_prompt("You are a test agent.");

        repo.create_template(&template).await.unwrap();

        let retrieved = repo.get_template(template.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-agent");
    }

    #[tokio::test]
    async fn test_get_by_name() {
        let repo = setup_test_repo().await;

        let template = AgentTemplate::new("named-agent", AgentTier::Specialist)
            .with_prompt("Specialist agent");

        repo.create_template(&template).await.unwrap();

        let found = repo.get_template_by_name("named-agent").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().tier, AgentTier::Specialist);
    }

    #[tokio::test]
    async fn test_list_by_tier() {
        let repo = setup_test_repo().await;

        let worker = AgentTemplate::new("worker-1", AgentTier::Worker)
            .with_prompt("Worker");
        let architect = AgentTemplate::new("architect-1", AgentTier::Architect)
            .with_prompt("Architect");

        repo.create_template(&worker).await.unwrap();
        repo.create_template(&architect).await.unwrap();

        let workers = repo.list_by_tier(AgentTier::Worker).await.unwrap();
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].name, "worker-1");

        let architects = repo.list_by_tier(AgentTier::Architect).await.unwrap();
        assert_eq!(architects.len(), 1);
    }

    #[tokio::test]
    async fn test_instance_lifecycle() {
        let repo = setup_test_repo().await;

        let template = AgentTemplate::new("instance-test", AgentTier::Worker)
            .with_prompt("Test");
        repo.create_template(&template).await.unwrap();

        let mut instance = AgentInstance::from_template(&template);
        repo.create_instance(&instance).await.unwrap();

        let task_id = Uuid::new_v4();
        instance.assign_task(task_id);
        repo.update_instance(&instance).await.unwrap();

        let running = repo.get_running_instances("instance-test").await.unwrap();
        assert_eq!(running.len(), 1);

        instance.complete();
        repo.update_instance(&instance).await.unwrap();

        let running = repo.get_running_instances("instance-test").await.unwrap();
        assert_eq!(running.len(), 0);
    }
}

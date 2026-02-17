//! SQLite implementation of the WorkflowRepository.

use async_trait::async_trait;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::workflow::{
    PhaseInstance, PhaseStatus, WorkflowDefinition, WorkflowInstance, WorkflowStatus,
};
use crate::domain::ports::workflow_repository::WorkflowRepository;

#[derive(Clone)]
pub struct SqliteWorkflowRepository {
    pool: SqlitePool,
}

impl SqliteWorkflowRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowRepository for SqliteWorkflowRepository {
    // -- Workflow Definitions --

    async fn save_definition(&self, definition: &WorkflowDefinition) -> DomainResult<()> {
        let definition_json = serde_json::to_string(definition)?;

        sqlx::query(
            "INSERT INTO workflow_definitions (id, name, goal_id, definition_json, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(definition.id.to_string())
        .bind(&definition.name)
        .bind(definition.goal_id.to_string())
        .bind(&definition_json)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_definition(&self, id: Uuid) -> DomainResult<Option<WorkflowDefinition>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT definition_json FROM workflow_definitions WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    async fn get_definitions_by_goal(
        &self,
        goal_id: Uuid,
    ) -> DomainResult<Vec<WorkflowDefinition>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT definition_json FROM workflow_definitions WHERE goal_id = ?",
        )
        .bind(goal_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(json,)| serde_json::from_str(&json).map_err(DomainError::from))
            .collect()
    }

    // -- Workflow Instances --

    async fn save_instance(&self, instance: &WorkflowInstance) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO workflow_instances (id, workflow_id, goal_id, status, tokens_consumed, created_at, updated_at, completed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(instance.id.to_string())
        .bind(instance.workflow_id.to_string())
        .bind(instance.goal_id.to_string())
        .bind(instance.status.to_string())
        .bind(instance.tokens_consumed as i64)
        .bind(instance.created_at.to_rfc3339())
        .bind(instance.updated_at.to_rfc3339())
        .bind(instance.completed_at.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await?;

        // Save all phase instances
        for phase_instance in instance.phase_instances.values() {
            self.save_phase_instance(instance.id, phase_instance).await?;
        }

        Ok(())
    }

    async fn get_instance(&self, id: Uuid) -> DomainResult<Option<WorkflowInstance>> {
        let row: Option<WorkflowInstanceRow> = sqlx::query_as(
            "SELECT id, workflow_id, goal_id, status, tokens_consumed, created_at, updated_at, completed_at
             FROM workflow_instances WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let mut instance = row.try_into_instance()?;
                let phases = self.get_phase_instances(instance.id).await?;
                for phase in phases {
                    instance.phase_instances.insert(phase.phase_id, phase);
                }
                Ok(Some(instance))
            }
            None => Ok(None),
        }
    }

    async fn update_instance(&self, instance: &WorkflowInstance) -> DomainResult<()> {
        sqlx::query(
            "UPDATE workflow_instances SET status = ?, tokens_consumed = ?, updated_at = ?, completed_at = ?
             WHERE id = ?",
        )
        .bind(instance.status.to_string())
        .bind(instance.tokens_consumed as i64)
        .bind(instance.updated_at.to_rfc3339())
        .bind(instance.completed_at.map(|t| t.to_rfc3339()))
        .bind(instance.id.to_string())
        .execute(&self.pool)
        .await?;

        // Update all phase instances
        for phase_instance in instance.phase_instances.values() {
            self.update_phase_instance(instance.id, phase_instance).await?;
        }

        Ok(())
    }

    async fn get_instances_by_status(
        &self,
        status: WorkflowStatus,
    ) -> DomainResult<Vec<WorkflowInstance>> {
        let rows: Vec<WorkflowInstanceRow> = sqlx::query_as(
            "SELECT id, workflow_id, goal_id, status, tokens_consumed, created_at, updated_at, completed_at
             FROM workflow_instances WHERE status = ?",
        )
        .bind(status.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut instances = Vec::new();
        for row in rows {
            let mut instance = row.try_into_instance()?;
            let phases = self.get_phase_instances(instance.id).await?;
            for phase in phases {
                instance.phase_instances.insert(phase.phase_id, phase);
            }
            instances.push(instance);
        }
        Ok(instances)
    }

    async fn get_instances_by_goal(&self, goal_id: Uuid) -> DomainResult<Vec<WorkflowInstance>> {
        let rows: Vec<WorkflowInstanceRow> = sqlx::query_as(
            "SELECT id, workflow_id, goal_id, status, tokens_consumed, created_at, updated_at, completed_at
             FROM workflow_instances WHERE goal_id = ?",
        )
        .bind(goal_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut instances = Vec::new();
        for row in rows {
            let mut instance = row.try_into_instance()?;
            let phases = self.get_phase_instances(instance.id).await?;
            for phase in phases {
                instance.phase_instances.insert(phase.phase_id, phase);
            }
            instances.push(instance);
        }
        Ok(instances)
    }

    // -- Phase Instances --

    async fn save_phase_instance(
        &self,
        workflow_instance_id: Uuid,
        phase_instance: &PhaseInstance,
    ) -> DomainResult<()> {
        let id = format!("{}:{}", workflow_instance_id, phase_instance.phase_id);
        let task_ids_json = serde_json::to_string(&phase_instance.task_ids)?;

        sqlx::query(
            "INSERT OR REPLACE INTO phase_instances (id, workflow_instance_id, phase_id, status, task_ids_json, retry_count, verification_result, iteration_count, started_at, completed_at, error)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workflow_instance_id.to_string())
        .bind(phase_instance.phase_id.to_string())
        .bind(phase_instance.status.to_string())
        .bind(&task_ids_json)
        .bind(phase_instance.retry_count as i32)
        .bind(phase_instance.verification_result.map(|b| b as i32))
        .bind(phase_instance.iteration_count as i32)
        .bind(phase_instance.started_at.map(|t| t.to_rfc3339()))
        .bind(phase_instance.completed_at.map(|t| t.to_rfc3339()))
        .bind(&phase_instance.error)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn update_phase_instance(
        &self,
        workflow_instance_id: Uuid,
        phase_instance: &PhaseInstance,
    ) -> DomainResult<()> {
        // Use save_phase_instance with INSERT OR REPLACE
        self.save_phase_instance(workflow_instance_id, phase_instance)
            .await
    }

    async fn get_phase_instances(
        &self,
        workflow_instance_id: Uuid,
    ) -> DomainResult<Vec<PhaseInstance>> {
        let rows: Vec<PhaseInstanceRow> = sqlx::query_as(
            "SELECT phase_id, status, task_ids_json, retry_count, verification_result, iteration_count, started_at, completed_at, error
             FROM phase_instances WHERE workflow_instance_id = ?",
        )
        .bind(workflow_instance_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.try_into_phase_instance()).collect()
    }
}

// ============================================================================
// Row types for sqlx
// ============================================================================

#[derive(sqlx::FromRow)]
struct WorkflowInstanceRow {
    id: String,
    workflow_id: String,
    goal_id: String,
    status: String,
    tokens_consumed: i64,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl WorkflowInstanceRow {
    fn try_into_instance(self) -> DomainResult<WorkflowInstance> {
        use crate::adapters::sqlite::{parse_datetime, parse_optional_datetime, parse_uuid};

        let status = match self.status.as_str() {
            "pending" => WorkflowStatus::Pending,
            "running" => WorkflowStatus::Running,
            "completed" => WorkflowStatus::Completed,
            "failed" => WorkflowStatus::Failed,
            "canceled" => WorkflowStatus::Canceled,
            other => {
                return Err(DomainError::SerializationError(format!(
                    "Unknown workflow status: {}",
                    other
                )))
            }
        };

        Ok(WorkflowInstance {
            id: parse_uuid(&self.id)?,
            workflow_id: parse_uuid(&self.workflow_id)?,
            goal_id: parse_uuid(&self.goal_id)?,
            status,
            phase_instances: std::collections::HashMap::new(), // populated separately
            tokens_consumed: self.tokens_consumed as u64,
            created_at: parse_datetime(&self.created_at)?,
            updated_at: parse_datetime(&self.updated_at)?,
            completed_at: parse_optional_datetime(self.completed_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PhaseInstanceRow {
    phase_id: String,
    status: String,
    task_ids_json: String,
    retry_count: i32,
    verification_result: Option<i32>,
    iteration_count: i32,
    started_at: Option<String>,
    completed_at: Option<String>,
    error: Option<String>,
}

impl PhaseInstanceRow {
    fn try_into_phase_instance(self) -> DomainResult<PhaseInstance> {
        use crate::adapters::sqlite::{parse_optional_datetime, parse_uuid};

        let status = match self.status.as_str() {
            "pending" => PhaseStatus::Pending,
            "ready" => PhaseStatus::Ready,
            "running" => PhaseStatus::Running,
            "verifying" => PhaseStatus::Verifying,
            "completed" => PhaseStatus::Completed,
            "failed" => PhaseStatus::Failed,
            "skipped" => PhaseStatus::Skipped,
            "awaiting_decision" => PhaseStatus::AwaitingDecision,
            other => {
                return Err(DomainError::SerializationError(format!(
                    "Unknown phase status: {}",
                    other
                )))
            }
        };

        let task_ids: Vec<Uuid> = serde_json::from_str(&self.task_ids_json)?;

        Ok(PhaseInstance {
            phase_id: parse_uuid(&self.phase_id)?,
            status,
            task_ids,
            retry_count: self.retry_count as u32,
            verification_result: self.verification_result.map(|v| v != 0),
            iteration_count: self.iteration_count as u32,
            started_at: parse_optional_datetime(self.started_at)?,
            completed_at: parse_optional_datetime(self.completed_at)?,
            error: self.error,
        })
    }
}

///! Workflow Verification Module
///!
///! Provides memory-based verification to detect orphaned workflows and validate
///! that tasks properly stored their workflow state in MCP memory.

use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::{AgentContractRegistry, Task, TaskStatus};
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Workflow health monitor for detecting orphaned workflows
pub struct WorkflowHealthMonitor {
    task_coordinator: Arc<TaskCoordinator>,
}

impl WorkflowHealthMonitor {
    /// Create a new workflow health monitor
    ///
    /// # Arguments
    ///
    /// * `task_coordinator` - Task coordinator for querying tasks
    pub fn new(task_coordinator: Arc<TaskCoordinator>) -> Self {
        Self { task_coordinator }
    }

    /// Detect tasks that completed but failed to spawn required children
    ///
    /// Scans completed tasks and checks if they met their agent contract requirements.
    ///
    /// # Returns
    ///
    /// Vector of task IDs that are orphaned (completed without required children)
    pub async fn detect_orphaned_workflows(&self) -> Result<Vec<Uuid>> {
        let completed_tasks = self
            .task_coordinator
            .get_tasks_by_status(TaskStatus::Completed)
            .await
            .context("Failed to get completed tasks")?;

        let mut orphaned = Vec::new();

        for task in completed_tasks {
            // Check if this agent type has contract requirements
            if let Some(contract) = Self::get_contract_requirements(&task.agent_type) {
                if contract.must_spawn_children {
                    // Check if children were spawned
                    let children = self
                        .task_coordinator
                        .get_child_tasks(task.id)
                        .await
                        .unwrap_or_default();

                    if children.len() < contract.min_children {
                        warn!(
                            task_id = %task.id,
                            agent_type = %task.agent_type,
                            expected = contract.min_children,
                            found = children.len(),
                            "Detected orphaned workflow"
                        );
                        orphaned.push(task.id);
                    }
                }
            }
        }

        Ok(orphaned)
    }

    /// Get contract requirements from registry
    fn get_contract_requirements(agent_type: &str) -> Option<ContractRequirements> {
        use crate::domain::models::ValidationRequirement;

        let req = AgentContractRegistry::get_validation_requirement(agent_type);

        if let ValidationRequirement::Contract {
            must_spawn_children,
            expected_child_types,
            min_children,
        } = req
        {
            Some(ContractRequirements {
                must_spawn_children,
                expected_child_types,
                min_children,
            })
        } else {
            None
        }
    }
}

/// Contract requirements extracted from validation requirement
#[derive(Debug)]
#[allow(dead_code)] // Used for future validation logic
struct ContractRequirements {
    must_spawn_children: bool,
    expected_child_types: Vec<String>,
    min_children: usize,
}

/// Verify that workflow state was properly stored in memory
///
/// Checks MCP memory for workflow tracking data. This would require MCP client integration.
///
/// # Arguments
///
/// * `task_id` - UUID of the task to verify
/// * `memory_client` - MCP memory client (if available)
///
/// # Returns
///
/// true if workflow state found in memory, false otherwise
///
/// # Note
///
/// This is a placeholder for future MCP memory integration
pub async fn verify_workflow_in_memory(
    task_id: Uuid,
    _memory_client: Option<&Value>, // Placeholder for MCP client
) -> Result<bool> {
    // TODO: Implement MCP memory verification
    // Expected memory keys:
    // - task:{task_id}:workflow (workflow state)
    // - task:{task_id}:requirements (for requirements-gatherer)
    // - task:{task_id}:architecture (for technical-architect)

    info!(
        task_id = %task_id,
        "Memory verification not yet implemented - requires MCP client integration"
    );

    // Placeholder: assume memory exists
    Ok(true)
}

/// Remediate an orphaned workflow
///
/// Determines the best remediation strategy for a task that completed without
/// spawning required children.
///
/// # Arguments
///
/// * `task_id` - UUID of the orphaned task
/// * `task_coordinator` - Task coordinator
///
/// # Returns
///
/// Remediation action taken
pub async fn remediate_orphaned_workflow(
    task_id: Uuid,
    task_coordinator: &TaskCoordinator,
) -> Result<RemediationAction> {
    let task = task_coordinator
        .get_task(task_id)
        .await
        .context("Failed to get orphaned task")?;

    info!(
        task_id = %task_id,
        agent_type = %task.agent_type,
        "Remediating orphaned workflow"
    );

    // For now, spawn task-salvage-specialist to analyze and remediate
    let salvage_task = Task::new(
        format!("Salvage: {}", task.summary),
        format!(
            r#"# Salvage Orphaned Workflow

## Failed Task
- Task ID: {}
- Agent Type: {}
- Summary: {}
- Status: {}

## Failure Mode
Task completed but failed to spawn required child tasks.

## Your Mission
Analyze the task memory and determine best remediation:
1. **Option A**: Salvage work and spawn missing children
2. **Option B**: Requeue entire workflow
3. **Option C**: Mark as failed (unsalvageable)

## Next Steps
1. Query task memory: task:{}:*
2. Assess work quality
3. Make remediation decision
4. Execute chosen strategy

Follow the task-salvage-specialist agent guidelines.
"#,
            task_id,
            task.agent_type,
            task.summary,
            task.status,
            task_id
        ),
    );

    let mut salvage_task_mut = salvage_task;
    salvage_task_mut.agent_type = "task-salvage-specialist".to_string();
    salvage_task_mut.priority = 9; // High priority
    salvage_task_mut.parent_task_id = task.parent_task_id;
    salvage_task_mut.dependencies = Some(vec![task_id]);

    let salvage_task_id = task_coordinator.submit_task(salvage_task_mut).await?;

    info!(
        task_id = %task_id,
        salvage_task_id = %salvage_task_id,
        "Spawned salvage specialist for orphaned workflow"
    );

    Ok(RemediationAction::SalvageSpecialistSpawned {
        salvage_task_id,
    })
}

/// Action taken during remediation
#[derive(Debug)]
pub enum RemediationAction {
    /// Spawned task-salvage-specialist to analyze and remediate
    SalvageSpecialistSpawned { salvage_task_id: Uuid },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remediation_action_variants() {
        let action = RemediationAction::SalvageSpecialistSpawned {
            salvage_task_id: Uuid::new_v4(),
        };
        assert!(matches!(
            action,
            RemediationAction::SalvageSpecialistSpawned { .. }
        ));
    }
}

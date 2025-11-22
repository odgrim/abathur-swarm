///! Task Validation Logic
///!
///! Provides validation functions for enforcing agent contracts and spawning
///! validation/remediation tasks based on validation requirements.
use crate::application::task_coordinator::TaskCoordinator;
use crate::domain::models::{AgentContractRegistry, Task, ValidationRequirement, WorkflowState};
use anyhow::{Context, Result};
use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Result of task validation
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Validation passed
    Passed,
    /// Validation failed with reason
    Failed { reason: String },
}

/// Validate that a task met its contract requirements
///
/// This function checks if a task spawned the expected child tasks
/// as defined by its agent contract.
///
/// If validation fails, automatically spawns task-salvage-specialist to remediate.
///
/// # Arguments
///
/// * `task_id` - UUID of the task to validate
/// * `task_coordinator` - Task coordinator for querying child tasks
///
/// # Returns
///
/// ValidationResult indicating if contract was met
pub async fn validate_contract(
    task_id: Uuid,
    task_coordinator: &TaskCoordinator,
) -> Result<ValidationResult> {
    // Get the task
    let task = task_coordinator
        .get_task(task_id)
        .await
        .context("Failed to get task for contract validation")?;

    // Get validation requirement from registry
    let validation_req = AgentContractRegistry::get_validation_requirement(&task.agent_type);

    let ValidationRequirement::Contract {
        must_spawn_children,
        expected_child_types,
        min_children,
    } = validation_req
    else {
        // No contract validation required
        return Ok(ValidationResult::Passed);
    };

    if !must_spawn_children {
        return Ok(ValidationResult::Passed);
    }

    // Get child tasks
    let children = task_coordinator.get_child_tasks(task_id).await?;

    // Check minimum children requirement
    if children.len() < min_children {
        let failure_reason = format!(
            "Agent '{}' must spawn at least {} children, found {}",
            task.agent_type, min_children, children.len()
        );

        warn!(
            task_id = %task_id,
            reason = %failure_reason,
            "Contract validation failed - spawning salvage specialist"
        );

        // Spawn salvage specialist to remediate
        spawn_salvage_specialist(task_id, &task, &failure_reason, task_coordinator).await?;

        return Ok(ValidationResult::Failed {
            reason: failure_reason,
        });
    }

    // Check child types if specified
    if !expected_child_types.is_empty() {
        for child in &children {
            if !expected_child_types.contains(&child.agent_type) {
                let failure_reason = format!(
                    "Agent '{}' spawned unexpected child type '{}'. Expected one of: {:?}",
                    task.agent_type, child.agent_type, expected_child_types
                );

                warn!(
                    task_id = %task_id,
                    reason = %failure_reason,
                    "Contract validation failed - spawning salvage specialist"
                );

                // Spawn salvage specialist to remediate
                spawn_salvage_specialist(task_id, &task, &failure_reason, task_coordinator).await?;

                return Ok(ValidationResult::Failed {
                    reason: failure_reason,
                });
            }
        }
    }

    // Update workflow state
    let workflow_state = WorkflowState {
        children_spawned: children.iter().map(|c| c.id).collect(),
        spawned_agent_types: children.iter().map(|c| c.agent_type.clone()).collect(),
        expectations_met: true,
        last_updated: Some(Utc::now()),
    };

    task_coordinator
        .update_workflow_state(task_id, workflow_state)
        .await?;

    info!(
        task_id = %task_id,
        children = children.len(),
        "Contract validation passed"
    );

    Ok(ValidationResult::Passed)
}

/// Spawn task-salvage-specialist to recover from contract validation failure
///
/// # Arguments
///
/// * `failed_task_id` - UUID of the task that failed validation
/// * `failed_task` - The task that failed validation
/// * `failure_reason` - Description of why validation failed
/// * `task_coordinator` - Task coordinator
///
/// # Returns
///
/// UUID of the spawned salvage task
async fn spawn_salvage_specialist(
    failed_task_id: Uuid,
    failed_task: &Task,
    failure_reason: &str,
    task_coordinator: &TaskCoordinator,
) -> Result<Uuid> {
    let description = format!(
        r#"# Salvage Orphaned Workflow

## Failed Task
- Task ID: {}
- Agent Type: {}
- Summary: {}
- Status: Contract validation failed

## Failure Reason
{}

## Your Mission
Analyze the task memory and determine best remediation strategy:

### Option A: Salvage & Spawn Missing Tasks
If the task completed its work but just forgot to spawn children,
retrieve the work from memory and manually spawn the required downstream tasks.

### Option B: Requeue Entire Workflow
If the work is incomplete or would be faster to redo,
create a new task for the original agent with enhanced instructions.

### Option C: Mark as Failed
If the work is fundamentally flawed or cannot be recovered,
mark the task as failed with detailed explanation.

## Investigation Steps
1. Query task memory: task:{}:*
2. Check what was actually stored
3. Assess work quality and completeness
4. Determine expected child tasks for agent type: {}
5. Make remediation decision
6. Execute chosen strategy

Follow the task-salvage-specialist agent guidelines for detailed procedures.

## Context
- Expected to spawn: {} children
- Expected types: {:?}
- Parent Task: {:?}
- Feature Branch: {:?}
"#,
        failed_task_id,
        failed_task.agent_type,
        failed_task.summary,
        failure_reason,
        failed_task_id,
        failed_task.agent_type,
        match AgentContractRegistry::get_validation_requirement(&failed_task.agent_type) {
            ValidationRequirement::Contract {
                min_children,
                expected_child_types,
                ..
            } => {
                format!(
                    "min: {}, types: {:?}",
                    min_children, expected_child_types
                )
            }
            _ => "unknown".to_string(),
        },
        match AgentContractRegistry::get_validation_requirement(&failed_task.agent_type) {
            ValidationRequirement::Contract {
                expected_child_types,
                ..
            } => format!("{:?}", expected_child_types),
            _ => "unknown".to_string(),
        },
        failed_task.parent_task_id,
        failed_task.feature_branch,
    );

    let mut salvage_task = Task::new(
        Task::create_summary_with_prefix("Salvage: ", &failed_task.summary),
        description,
    );

    salvage_task.agent_type = "task-salvage-specialist".to_string();
    salvage_task.priority = 9; // High priority - blocks workflow
    salvage_task.parent_task_id = Some(failed_task_id); // Set failed task as parent, not dependency
    salvage_task.validation_requirement = ValidationRequirement::None; // Don't validate the salvager

    let salvage_task_id = task_coordinator.submit_task(salvage_task).await?;

    info!(
        failed_task_id = %failed_task_id,
        salvage_task_id = %salvage_task_id,
        "Spawned salvage specialist for contract validation failure"
    );

    Ok(salvage_task_id)
}

/// Main entry point for validating task completion
///
/// This function determines what type of validation is required and
/// either performs inline validation or spawns a validation task.
///
/// # Arguments
///
/// * `task_id` - UUID of the task to validate
/// * `task` - The task being validated
/// * `task_coordinator` - Task coordinator
///
/// # Returns
///
/// ValidationResult or spawns validation task
pub async fn validate_task_completion(
    task_id: Uuid,
    task: &Task,
    task_coordinator: &TaskCoordinator,
) -> Result<ValidationResult> {
    let validation_req = AgentContractRegistry::get_validation_requirement(&task.agent_type);

    match validation_req {
        ValidationRequirement::None => {
            // No validation required
            Ok(ValidationResult::Passed)
        }

        ValidationRequirement::Contract { .. } => {
            // Inline contract validation
            validate_contract(task_id, task_coordinator).await
        }

        ValidationRequirement::Testing { .. } => {
            // Testing validation requires spawning a validator agent
            // This is handled by spawn_validation_task
            Ok(ValidationResult::Passed) // Assume will be validated externally
        }
    }
}

/// Spawn a validation task for test-based validation
///
/// Creates a validator agent task that will run tests in the worktree
/// and route to either merge or remediation based on results.
///
/// # Arguments
///
/// * `original_task_id` - UUID of the task being validated
/// * `original_task` - The task being validated
/// * `task_coordinator` - Task coordinator
///
/// # Returns
///
/// UUID of the spawned validation task
pub async fn spawn_validation_task(
    original_task_id: Uuid,
    original_task: &Task,
    task_coordinator: &TaskCoordinator,
) -> Result<Uuid> {
    let validation_req = AgentContractRegistry::get_validation_requirement(&original_task.agent_type);

    let ValidationRequirement::Testing {
        validator_agent,
        test_commands,
        max_remediation_cycles,
        ..
    } = validation_req
    else {
        return Err(anyhow::anyhow!(
            "Cannot spawn validation task for non-Testing validation requirement"
        ));
    };

    // Build validation task description
    let description = format!(
        r#"# Validation Task for: {}

## Original Task
- Task ID: {}
- Agent Type: {}
- Summary: {}

## Validation Instructions
Run the following validation checks in the worktree:

{}

## Worktree Context
- Worktree Path: {}
- Task Branch: {}
- Feature Branch: {}

## Expected Actions
1. Navigate to worktree
2. Verify git status is clean (all changes committed)
3. Run cargo build (must succeed)
4. Run all test commands above
5. If ALL tests pass:
   - Mark original task as Completed
   - Enqueue merge task to merge into feature branch
6. If ANY tests fail:
   - Enqueue remediation task back to {} agent
   - Mark original task as ValidationFailed
   - Include detailed test failure output in remediation task

## Remediation Tracking
- Current Cycle: {}/{}
- If max cycles exceeded, mark task as Failed (not fixable)

## Memory Storage
Store validation results in:
- Namespace: task:{}:validation
- Keys: test_results, validation_status, failure_details
"#,
        original_task.summary,
        original_task_id,
        original_task.agent_type,
        original_task.summary,
        test_commands.join("\n"),
        original_task
            .worktree_path
            .as_ref()
            .unwrap_or(&"N/A".to_string()),
        original_task
            .branch
            .as_ref()
            .unwrap_or(&"N/A".to_string()),
        original_task
            .feature_branch
            .as_ref()
            .unwrap_or(&"N/A".to_string()),
        original_task.agent_type,
        original_task.remediation_count + 1,
        max_remediation_cycles,
        original_task_id,
    );

    let mut validation_task = Task::new(
        Task::create_summary_with_prefix("Validate: ", &original_task.summary),
        description,
    );

    validation_task.agent_type = validator_agent;
    validation_task.priority = original_task.priority;
    validation_task.parent_task_id = Some(original_task_id);
    validation_task.validating_task_id = Some(original_task_id);
    validation_task.dependencies = Some(vec![original_task_id]);
    validation_task.worktree_path = original_task.worktree_path.clone();
    validation_task.branch = original_task.branch.clone();
    validation_task.feature_branch = original_task.feature_branch.clone();
    validation_task.validation_requirement = ValidationRequirement::None; // Don't validate the validator

    let validation_task_id = task_coordinator.submit_task(validation_task).await?;

    // Link validation task to original
    task_coordinator
        .link_validation_task(original_task_id, validation_task_id)
        .await?;

    info!(
        original_task_id = %original_task_id,
        validation_task_id = %validation_task_id,
        "Spawned validation task"
    );

    Ok(validation_task_id)
}

/// Spawn a remediation task after validation failure
///
/// Creates a task for the original agent to fix issues found by validation.
///
/// # Arguments
///
/// * `original_task_id` - UUID of the task that failed validation
/// * `original_task` - The task that failed validation
/// * `test_failures` - Description of test failures
/// * `task_coordinator` - Task coordinator
///
/// # Returns
///
/// UUID of the spawned remediation task
pub async fn spawn_remediation_task(
    original_task_id: Uuid,
    original_task: &Task,
    test_failures: &str,
    task_coordinator: &TaskCoordinator,
) -> Result<Uuid> {
    // Check if max remediation cycles exceeded
    let validation_req = AgentContractRegistry::get_validation_requirement(&original_task.agent_type);

    let max_cycles = if let ValidationRequirement::Testing {
        max_remediation_cycles,
        ..
    } = validation_req
    {
        max_remediation_cycles
    } else {
        3 // Default
    };

    if original_task.remediation_count >= max_cycles as u32 {
        error!(
            task_id = %original_task_id,
            cycles = original_task.remediation_count,
            "Max remediation cycles exceeded"
        );
        return Err(anyhow::anyhow!(
            "Max remediation cycles ({}) exceeded",
            max_cycles
        ));
    }

    let description = format!(
        r#"# Remediation Task: Fix Validation Failures

## Original Task
- Task ID: {}
- Summary: {}
- Remediation Cycle: {}/{}

## Validation Failures
{}

## Required Actions
1. Navigate to worktree: {}
2. Review test failure output above
3. Fix all issues causing tests to fail
4. Commit fixes with descriptive message
5. Validation will automatically re-run after completion

## Context
- Task Branch: {}
- Feature Branch: {}
- Original Agent: {}

## Important Notes
- This is remediation attempt {}/{}
- If validation fails again and max cycles is reached, task will be marked as Failed
- Focus on fixing the root cause, not just making tests pass
"#,
        original_task_id,
        original_task.summary,
        original_task.remediation_count + 1,
        max_cycles,
        test_failures,
        original_task
            .worktree_path
            .as_ref()
            .unwrap_or(&"N/A".to_string()),
        original_task
            .branch
            .as_ref()
            .unwrap_or(&"N/A".to_string()),
        original_task
            .feature_branch
            .as_ref()
            .unwrap_or(&"N/A".to_string()),
        original_task.agent_type,
        original_task.remediation_count + 1,
        max_cycles,
    );

    let mut remediation_task = Task::new(
        Task::create_summary_with_prefix("Fix: ", &original_task.summary),
        description,
    );

    remediation_task.agent_type = original_task.agent_type.clone();
    remediation_task.priority = original_task.priority;
    remediation_task.parent_task_id = original_task.parent_task_id;
    remediation_task.dependencies = Some(vec![original_task_id]);
    remediation_task.worktree_path = original_task.worktree_path.clone();
    remediation_task.branch = original_task.branch.clone();
    remediation_task.feature_branch = original_task.feature_branch.clone();
    remediation_task.remediation_count = original_task.remediation_count + 1;
    remediation_task.is_remediation = true;
    remediation_task.validation_requirement = original_task.validation_requirement.clone();

    let remediation_task_id = task_coordinator.submit_task(remediation_task).await?;

    warn!(
        original_task_id = %original_task_id,
        remediation_task_id = %remediation_task_id,
        cycle = original_task.remediation_count + 1,
        "Spawned remediation task"
    );

    Ok(remediation_task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_result_variants() {
        let passed = ValidationResult::Passed;
        assert!(matches!(passed, ValidationResult::Passed));

        let failed = ValidationResult::Failed {
            reason: "test failure".to_string(),
        };
        assert!(matches!(failed, ValidationResult::Failed { .. }));
    }
}

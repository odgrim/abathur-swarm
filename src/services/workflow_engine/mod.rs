//! Deterministic workflow state machine.
//!
//! The `WorkflowEngine` drives phase ordering and tracks completion for tasks
//! enrolled in a workflow. It is purely deterministic — no LLM calls. The
//! Overmind provides gate decisions and creates agents for each phase.
//!
//! Workflow state is stored as a JSON blob in `task.context.custom["workflow_state"]`.

use std::sync::Arc;

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::task::Task;
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::workflow_template::WorkflowTemplate;
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::EventBus;
use crate::services::task_service::TaskService;

mod fan_out;
mod lifecycle;
mod validators;

#[cfg(test)]
mod tests;

use validators::validate_state_consistency;

/// Result of an `advance` call, giving the overmind enough info to fan out.
#[derive(Debug, Clone)]
pub enum AdvanceResult {
    /// The next phase is ready; the overmind should call `fan_out` to create subtasks.
    PhaseReady {
        phase_index: usize,
        phase_name: String,
    },
    /// All phases are done; the workflow completed successfully.
    Completed,
}

/// Result of a `fan_out` call.
#[derive(Debug, Clone)]
pub struct FanOutResult {
    pub subtask_ids: Vec<Uuid>,
    pub phase_index: usize,
    pub phase_name: String,
}

/// Workflow status summary returned by `get_state`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkflowStatus {
    pub task_id: Uuid,
    pub workflow_name: String,
    pub state: WorkflowState,
    pub total_phases: usize,
    pub current_phase_index: Option<usize>,
    pub current_phase_name: Option<String>,
    pub is_verifying: bool,
    pub verification_retry_count: Option<u32>,
}

/// The deterministic workflow engine.
pub struct WorkflowEngine<T: TaskRepository> {
    pub(super) task_repo: Arc<T>,
    pub(super) task_service: TaskService<T>,
    pub(super) event_bus: Arc<EventBus>,
    pub(super) templates: std::collections::HashMap<String, WorkflowTemplate>,
    pub(super) verification_enabled: bool,
}

impl<T: TaskRepository + 'static> WorkflowEngine<T> {
    pub fn new(
        task_repo: Arc<T>,
        task_service: TaskService<T>,
        event_bus: Arc<EventBus>,
        verification_enabled: bool,
    ) -> Self {
        Self {
            task_repo,
            task_service,
            event_bus,
            templates: std::collections::HashMap::new(),
            verification_enabled,
        }
    }

    /// Merge additional workflow templates into this engine.
    ///
    /// Supplied templates take precedence over any already loaded with the
    /// same name.
    pub fn with_templates(
        mut self,
        extra: std::collections::HashMap<String, WorkflowTemplate>,
    ) -> Self {
        self.templates.extend(extra);
        self
    }

    /// Create a workflow engine seeded with YAML and inline workflow templates
    /// loaded from the current configuration.
    ///
    /// Resolution mirrors `Config::resolve_workflow`: inline workflows override
    /// YAML workflows from `workflows_dir`. If no matching workflows are found,
    /// the engine will reject tasks whose template name cannot be resolved;
    /// run `abathur init` to scaffold the default YAML workflows.
    pub fn new_with_config(
        task_repo: Arc<T>,
        task_service: TaskService<T>,
        event_bus: Arc<EventBus>,
        verification_enabled: bool,
    ) -> Self {
        let config = crate::services::config::Config::load().unwrap_or_default();
        let mut templates = config.load_yaml_workflows();
        for wf in &config.workflows {
            templates.insert(wf.name.clone(), wf.clone());
        }
        Self {
            task_repo,
            task_service,
            event_bus,
            templates,
            verification_enabled,
        }
    }

    /// Returns the names of all workflow templates known to this engine.
    pub fn available_workflow_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.templates.keys().cloned().collect();
        names.sort();
        names
    }

    /// Look up the workflow template by name.
    pub(super) fn get_template(&self, name: &str) -> DomainResult<&WorkflowTemplate> {
        self.templates.get(name).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Unknown workflow template: {}", name))
        })
    }

    /// Read workflow state from task context.
    pub(super) fn read_state(task: &Task) -> Option<WorkflowState> {
        task.workflow_state()
    }

    /// Public accessor for reading workflow state from a task (used by handlers).
    pub fn read_state_from_task(task: &Task) -> Option<WorkflowState> {
        Self::read_state(task)
    }

    /// Write workflow state to task context and persist via TaskService
    /// (with retry-on-conflict).
    pub(super) async fn write_state(
        &self,
        task_id: Uuid,
        state: &WorkflowState,
    ) -> DomainResult<()> {
        self.task_service
            .update_workflow_state(task_id, state)
            .await?;

        // Post-condition: validate state consistency
        self.check_state_consistency(task_id, state).await;

        Ok(())
    }

    /// Load the task's current TaskStatus and validate it against the given WorkflowState.
    /// Logs a warning on inconsistency but never errors.
    async fn check_state_consistency(&self, task_id: Uuid, workflow_state: &WorkflowState) {
        match self.task_repo.get(task_id).await {
            Ok(Some(task)) => {
                if let Err(msg) = validate_state_consistency(task.status, workflow_state) {
                    tracing::warn!(
                        task_id = %task_id,
                        task_status = %task.status,
                        workflow_state = ?workflow_state,
                        "State consistency invariant violated: {}",
                        msg
                    );
                }
            }
            Ok(None) => {
                tracing::warn!(
                    task_id = %task_id,
                    "State consistency check: task not found"
                );
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id,
                    error = %e,
                    "State consistency check: failed to load task"
                );
            }
        }
    }

    // ========================================================================
    // Public API
    // ========================================================================

    // enroll() — REMOVED
    // Auto-enrollment happens in TaskService::submit_task() with template
    // validation (fix C). The MCP enroll tool was also removed. Tests use
    // submit_task() or write workflow state directly.

    /// Change the workflow spine for a task that is still in `Pending` state.
    ///
    /// This allows the overmind to override the auto-selected workflow before
    /// the first phase starts. Validates the template exists, verifies the
    /// task is in `Pending` state, and updates both the workflow state and
    /// routing hints.
    pub async fn select_workflow(
        &self,
        task_id: Uuid,
        workflow_name: &str,
    ) -> DomainResult<WorkflowStatus> {
        // Validate the target template exists
        let _template = self.get_template(workflow_name)?;

        let mut task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        // Only allow spine change while still in Pending
        match &state {
            WorkflowState::Pending { .. } => {}
            other => {
                return Err(DomainError::ValidationFailed(format!(
                    "Cannot change workflow spine for task {} — must be in Pending state (current: {:?})",
                    task_id, other
                )));
            }
        }

        // Overwrite workflow state and routing hints atomically.
        // We already loaded `task` fresh above (with version), so set both
        // fields and persist in one write. On conflict, the caller can retry.
        let new_state = WorkflowState::Pending {
            workflow_name: workflow_name.to_string(),
        };
        let value = serde_json::to_value(&new_state)
            .map_err(|e| DomainError::SerializationError(e.to_string()))?;
        task.context
            .custom
            .insert("workflow_state".to_string(), value);
        task.routing_hints.workflow_name = Some(workflow_name.to_string());
        task.updated_at = chrono::Utc::now();
        self.task_repo.update(&task).await?;

        tracing::info!(
            task_id = %task_id,
            workflow_name = %workflow_name,
            "Workflow spine changed via select_workflow"
        );

        self.get_state(task_id).await
    }

    /// Get the current workflow state for a task.
    pub async fn get_state(&self, task_id: Uuid) -> DomainResult<WorkflowStatus> {
        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let workflow_name = state.workflow_name().to_string();
        let total_phases = self
            .get_template(&workflow_name)
            .map(|t| t.phases.len())
            .unwrap_or(0);

        let (current_phase_index, current_phase_name, is_verifying, verification_retry_count) =
            match &state {
                WorkflowState::PhaseRunning {
                    phase_index,
                    phase_name,
                    ..
                }
                | WorkflowState::FanningOut {
                    phase_index,
                    phase_name,
                    ..
                }
                | WorkflowState::Aggregating {
                    phase_index,
                    phase_name,
                    ..
                } => (Some(*phase_index), Some(phase_name.clone()), false, None),
                WorkflowState::Verifying {
                    phase_index,
                    phase_name,
                    retry_count,
                    ..
                } => (
                    Some(*phase_index),
                    Some(phase_name.clone()),
                    true,
                    Some(*retry_count),
                ),
                WorkflowState::PhaseGate {
                    phase_index,
                    phase_name,
                    ..
                }
                | WorkflowState::PhaseReady {
                    phase_index,
                    phase_name,
                    ..
                } => (Some(*phase_index), Some(phase_name.clone()), false, None),
                _ => (None, None, false, None),
            };

        Ok(WorkflowStatus {
            task_id,
            workflow_name,
            state,
            total_phases,
            current_phase_index,
            current_phase_name,
            is_verifying,
            verification_retry_count,
        })
    }
}

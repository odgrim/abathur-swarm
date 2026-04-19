//! Fan-out/fan-in logic for workflow phases.
//!
//! Handles subtask creation for parallel slices, aggregation task creation
//! when all slices complete, and subtask state queries used by the state
//! machine.

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::task::{ExecutionMode, Task, TaskSource, TaskStatus, TaskType};
use crate::domain::models::workflow_state::{FanOutSlice, WorkflowState};
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity};
use crate::services::event_factory;

use super::{FanOutResult, WorkflowEngine};

impl<T: TaskRepository + 'static> WorkflowEngine<T> {
    /// Fan out the current phase into N parallel subtasks.
    pub async fn fan_out(
        &self,
        task_id: Uuid,
        slices: Vec<FanOutSlice>,
    ) -> DomainResult<FanOutResult> {
        if slices.is_empty() {
            return Err(DomainError::ValidationFailed(
                "fan_out requires at least one slice".to_string(),
            ));
        }

        // Every slice must specify an agent — the Overmind is responsible for
        // creating/selecting the right specialist before fanning out.
        for (i, slice) in slices.iter().enumerate() {
            if slice.agent.is_none() {
                return Err(DomainError::ValidationFailed(format!(
                    "fan_out slice {} is missing required `agent` field — \
                         create or select an agent template and set `agent` on every slice",
                    i
                )));
            }
        }

        let task = self
            .task_repo
            .get(task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", task_id))
        })?;

        let (workflow_name, phase_index, phase_name) = match &state {
            // fan_out only accepts PhaseReady — overmind must call advance() first
            WorkflowState::PhaseReady {
                workflow_name,
                phase_index,
                phase_name,
            } => (workflow_name.clone(), *phase_index, phase_name.clone()),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Cannot fan_out task {} from state {:?} — call advance() first to reach PhaseReady",
                    task_id, state
                )));
            }
        };

        let template = self.get_template(&workflow_name)?;
        let phase = &template.phases[phase_index];

        let mut subtask_ids = Vec::new();
        for (i, slice) in slices.iter().enumerate() {
            let title = format!(
                "[{}/{}:{}] {} (slice {}/{})",
                workflow_name,
                phase_index,
                phase.name,
                slice.description.chars().take(50).collect::<String>(),
                i + 1,
                slices.len()
            );
            let description = format!(
                "Workflow: {}\nPhase: {} ({}/{})\nSlice {}/{}:\n\n{}\n\nParent task: {}",
                workflow_name,
                phase.name,
                phase_index + 1,
                template.phases.len(),
                i + 1,
                slices.len(),
                slice.description,
                task.description
            );

            let mut subtask = Task::with_title(&title, &description);
            subtask.parent_id = Some(task_id);
            subtask.source = TaskSource::SubtaskOf(task_id);
            subtask
                .transition_to(TaskStatus::Ready)
                .expect("Pending → Ready transition must succeed for freshly-created subtask");

            // Assign agent_type inline if the slice specifies one
            if let Some(ref agent) = slice.agent {
                subtask.agent_type = Some(agent.clone());
            }

            // Inherit worktree from parent
            subtask.worktree_path = task.worktree_path.clone();

            // Copy slice context into subtask
            for (k, v) in &slice.context {
                subtask.context.custom.insert(k.clone(), v.clone());
            }
            subtask.set_workflow_phase_value(serde_json::json!({
                "workflow_name": workflow_name,
                "phase_index": phase_index,
                "phase_name": phase_name,
                "slice_index": i,
                "total_slices": slices.len(),
            }));

            self.task_repo.create(&subtask).await?;
            subtask_ids.push(subtask.id);
        }

        let slice_count = slices.len();
        let new_state = WorkflowState::FanningOut {
            workflow_name: workflow_name.clone(),
            phase_index,
            phase_name: phase_name.clone(),
            subtask_ids: subtask_ids.clone(),
            slice_count,
        };
        self.write_state(task_id, &new_state).await?;

        self.event_bus
            .publish(event_factory::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                None,
                Some(task_id),
                EventPayload::WorkflowPhaseStarted {
                    task_id,
                    phase_index,
                    phase_name: phase_name.clone(),
                    subtask_ids: subtask_ids.clone(),
                },
            ))
            .await;

        Ok(FanOutResult {
            subtask_ids,
            phase_index,
            phase_name,
        })
    }

    /// Handle fan-in: all fan-out subtasks are done, create aggregation task.
    ///
    /// Transitions from FanningOut → Aggregating and creates a read-only
    /// aggregation subtask that synthesizes results from parallel slices.
    pub(super) async fn handle_fan_in(&self, parent_task_id: Uuid) -> DomainResult<()> {
        let task = self
            .task_repo
            .get(parent_task_id)
            .await?
            .ok_or(DomainError::TaskNotFound(parent_task_id))?;

        let state = Self::read_state(&task).ok_or_else(|| {
            DomainError::ValidationFailed(format!("Task {} has no workflow state", parent_task_id))
        })?;

        let (workflow_name, phase_index, phase_name, fan_out_subtask_ids) = match &state {
            WorkflowState::FanningOut {
                workflow_name,
                phase_index,
                phase_name,
                subtask_ids,
                ..
            } => (
                workflow_name.clone(),
                *phase_index,
                phase_name.clone(),
                subtask_ids.clone(),
            ),
            _ => {
                return Err(DomainError::ValidationFailed(format!(
                    "Task {} is not in FanningOut state",
                    parent_task_id
                )));
            }
        };

        // Collect summaries from completed fan-out subtasks
        let mut slice_summaries = Vec::new();
        for (i, id) in fan_out_subtask_ids.iter().enumerate() {
            if let Ok(Some(subtask)) = self.task_repo.get(*id).await {
                let artifact_refs: Vec<String> =
                    subtask.artifacts.iter().map(|a| a.uri.clone()).collect();
                slice_summaries.push(format!(
                    "Slice {} (id: {}): {} (status: {}, artifacts: {})",
                    i + 1,
                    id,
                    subtask.title,
                    subtask.status.as_str(),
                    if artifact_refs.is_empty() {
                        "none".to_string()
                    } else {
                        artifact_refs.join(", ")
                    }
                ));
            }
        }

        let subtask_id_list: Vec<String> = fan_out_subtask_ids
            .iter()
            .map(|id| id.to_string())
            .collect();

        let aggregation_desc = format!(
            "Aggregate results from {} parallel slices of phase '{}'.\n\n\
             ## Subtask IDs\n{}\n\n\
             ## Slice Results\n{}\n\n\
             ## Your Role\n\
             1. Use `task_get` for each subtask ID above to read full results.\n\
             2. Use `memory_search` to find memories stored by these subtasks \
             (search for the subtask titles and phase name '{}').\n\
             3. Synthesize findings into a unified summary.\n\
             4. Decide whether the NEXT phase should fan out to multiple parallel \
             tasks or collapse to a single task. Base this on:\n\
                - Whether subtask results are independent or need integration\n\
                - Whether the work ahead is naturally parallelizable\n\
                - Whether conflicts/gaps require a single coordinating agent\n\
             5. Store your summary AND fan-out/collapse recommendation via `memory_store`.\n\n\
             Parent task: {}",
            fan_out_subtask_ids.len(),
            phase_name,
            subtask_id_list.join("\n"),
            slice_summaries.join("\n"),
            phase_name,
            task.description
        );

        let mut agg_subtask = Task::with_title(
            format!(
                "[{}/{}:{}] Aggregate fan-out results",
                workflow_name, phase_index, phase_name
            ),
            &aggregation_desc,
        );
        agg_subtask.parent_id = Some(parent_task_id);
        agg_subtask.source = TaskSource::SubtaskOf(parent_task_id);
        agg_subtask.task_type = TaskType::Standard;
        agg_subtask.execution_mode = ExecutionMode::Direct; // Aggregation is always read-only
        agg_subtask.worktree_path = task.worktree_path.clone();
        agg_subtask.transition_to(TaskStatus::Ready).expect(
            "Pending → Ready transition must succeed for freshly-created aggregation subtask",
        );
        agg_subtask.agent_type = Some("aggregator".to_string());

        agg_subtask.set_workflow_phase_value(serde_json::json!({
            "workflow_name": workflow_name,
            "phase_index": phase_index,
            "phase_name": phase_name,
            "is_aggregation": true,
        }));

        self.task_repo.create(&agg_subtask).await?;

        // Include all original subtask_ids plus the aggregation subtask
        let mut all_ids = fan_out_subtask_ids;
        all_ids.push(agg_subtask.id);

        let aggregating_state = WorkflowState::Aggregating {
            workflow_name: workflow_name.clone(),
            phase_index,
            phase_name: phase_name.clone(),
            subtask_ids: all_ids,
        };
        self.write_state(parent_task_id, &aggregating_state).await?;

        tracing::info!(
            task_id = %parent_task_id,
            aggregation_subtask = %agg_subtask.id,
            "Fan-in: created aggregation subtask"
        );

        Ok(())
    }

    /// Check if all subtask IDs are in a terminal state.
    pub(super) async fn all_subtasks_done(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) if t.status.is_terminal() => continue,
                Some(_) => return Ok(false),
                None => {
                    tracing::warn!(subtask_id = %id, "Workflow subtask missing — treating as terminal");
                    continue;
                }
            }
        }
        Ok(true)
    }

    /// Check if any subtask failed or was canceled.
    pub(super) async fn any_subtask_failed(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) if t.status == TaskStatus::Failed || t.status == TaskStatus::Canceled => {
                    return Ok(true);
                }
                _ => continue,
            }
        }
        Ok(false)
    }

    /// Check if all subtasks have convergence_outcome == "converged".
    ///
    /// Used to skip redundant workflow verification when convergent execution
    /// already verified each subtask.
    pub(super) async fn all_subtasks_converged(&self, subtask_ids: &[Uuid]) -> DomainResult<bool> {
        for id in subtask_ids {
            match self.task_repo.get(*id).await? {
                Some(t) => {
                    let converged = t
                        .context
                        .custom
                        .get("convergence_outcome")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "converged" || s == "partial_accepted")
                        .unwrap_or(false);
                    if !converged {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }
}

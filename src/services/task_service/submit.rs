//! Task submission path: classification heuristic, submit_task, and readiness helpers.

use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::models::{
    Complexity, ExecutionMode, Task, TaskContext, TaskPriority, TaskSource, TaskStatus, TaskType,
};
use crate::domain::ports::TaskRepository;
use crate::services::event_bus::{EventCategory, EventPayload, EventSeverity, UnifiedEvent};

use super::TaskService;

impl<T: TaskRepository> TaskService<T> {
    // --- Scoring weights for classify_execution_mode ---
    //
    // | Signal                        | Weight | Direction   |
    // |-------------------------------|--------|-------------|
    // | Agent role (impl/dev/coder)   |   +2   | Convergent  |
    // | Agent role (research/plan)    |   −2   | Direct      |
    // | Complexity::Complex           |   +3   | Convergent  |
    // | Complexity::Trivial / Simple  |   −3   | Direct      |
    // | Moderate + long description   |   +2   | Convergent  |
    // | Acceptance keywords           |   +2   | Convergent  |
    // | Anti-pattern / constraint     |   +2   | Convergent  |
    // | Parent is convergent          |   +3   | Convergent  |
    // | Low priority                  |   −2   | Direct      |
    // | **Threshold**                 | **≥3** | Convergent  |

    const AGENT_ROLE_WEIGHT: i32 = 2;
    const COMPLEXITY_COMPLEX_WEIGHT: i32 = 3;
    const COMPLEXITY_TRIVIAL_SIMPLE_WEIGHT: i32 = 3;
    const MODERATE_LONG_DESC_WEIGHT: i32 = 2;
    const ACCEPTANCE_KEYWORD_WEIGHT: i32 = 2;
    const ANTIPATTERN_HINT_WEIGHT: i32 = 2;
    const PARENT_CONVERGENT_WEIGHT: i32 = 3;
    const LOW_PRIORITY_WEIGHT: i32 = 2;
    const CONVERGENT_THRESHOLD: i32 = 3;

    /// Classify whether a task should use Direct or Convergent execution mode.
    ///
    /// Uses a scoring heuristic based on task complexity, description content,
    /// context hints, source lineage, agent role, and priority. A score >=
    /// [`CONVERGENT_THRESHOLD`] recommends Convergent mode; below that, Direct
    /// mode is used. See the weight table above for the full scoring breakdown.
    ///
    /// When `default_mode` is `Some(...)`, the operator override takes precedence
    /// and the heuristic is skipped entirely (the operator's mode is returned).
    pub(super) fn classify_execution_mode(
        task: &Task,
        parent_mode: Option<&ExecutionMode>,
        default_mode: &Option<ExecutionMode>,
    ) -> ExecutionMode {
        // If operator set a default, use it as the baseline for tasks that
        // did not explicitly request a mode.
        if let Some(mode) = default_mode {
            return mode.clone();
        }

        let mut convergent_score: i32 = 0;

        // --- Agent-role signal ---
        // Execution-focused agents strongly favor convergent mode;
        // orchestration/research agents favor direct mode.
        if let Some(ref agent) = task.agent_type {
            let lower = agent.to_lowercase();
            if lower == "overmind"
                || lower.contains("researcher")
                || lower.contains("planner")
                || lower.contains("analyst")
                || lower.contains("architect")
            {
                convergent_score -= Self::AGENT_ROLE_WEIGHT;
            } else if lower.contains("implement")
                || lower.contains("develop")
                || lower.contains("coder")
                || lower.contains("fixer")
            {
                convergent_score += Self::AGENT_ROLE_WEIGHT;
            }
        }

        // --- Complexity signals ---
        match task.routing_hints.complexity {
            Complexity::Complex => convergent_score += Self::COMPLEXITY_COMPLEX_WEIGHT,
            Complexity::Moderate => {
                // Moderate complexity with a lengthy description suggests
                // requirements that benefit from iterative refinement.
                if task.description.split_whitespace().count() > 200 {
                    convergent_score += Self::MODERATE_LONG_DESC_WEIGHT;
                }
            }
            Complexity::Trivial => convergent_score -= Self::COMPLEXITY_TRIVIAL_SIMPLE_WEIGHT,
            Complexity::Simple => convergent_score -= Self::COMPLEXITY_TRIVIAL_SIMPLE_WEIGHT,
        }

        // --- Description content signals ---
        let desc_lower = task.description.to_lowercase();

        // Presence of test expectations or acceptance criteria implies
        // measurable success conditions — a strong fit for convergence.
        let acceptance_keywords = [
            "acceptance criteria",
            "should pass",
            "must pass",
            "expected output",
            "test case",
            "assert",
            "verify that",
            "ensure that",
        ];
        if acceptance_keywords.iter().any(|kw| desc_lower.contains(kw)) {
            convergent_score += Self::ACCEPTANCE_KEYWORD_WEIGHT;
        }

        // --- Context hints signals ---
        // Anti-patterns and constraints in hints suggest the task needs
        // guardrails that convergence provides.
        let has_anti_patterns = task
            .context
            .hints
            .iter()
            .any(|h| h.starts_with("anti-pattern:") || h.starts_with("constraint:"));
        if has_anti_patterns {
            convergent_score += Self::ANTIPATTERN_HINT_WEIGHT;
        }

        // --- Parent inheritance ---
        // Subtasks of convergent parents inherit the convergent mode unless
        // other signals strongly push toward Direct.
        if let TaskSource::SubtaskOf(_) = &task.source
            && let Some(parent_exec_mode) = parent_mode
            && parent_exec_mode.is_convergent()
        {
            convergent_score += Self::PARENT_CONVERGENT_WEIGHT;
        }

        // --- Priority signal ---
        // Low priority tasks are "fast-lane": favor Direct execution to
        // minimize latency and token cost.
        if task.priority == TaskPriority::Low {
            convergent_score -= Self::LOW_PRIORITY_WEIGHT;
        }

        // --- Threshold decision ---
        if convergent_score >= Self::CONVERGENT_THRESHOLD {
            ExecutionMode::Convergent {
                parallel_samples: None,
            }
        } else {
            ExecutionMode::Direct
        }
    }

    /// Look up the parent task's execution mode, if the task has a parent.
    async fn resolve_parent_execution_mode(
        &self,
        parent_id: Option<Uuid>,
    ) -> DomainResult<Option<ExecutionMode>> {
        match parent_id {
            Some(pid) => {
                let parent = self.task_repo.get(pid).await?;
                Ok(parent.map(|p| p.execution_mode))
            }
            None => Ok(None),
        }
    }

    /// Submit a new task. Returns the task and events to be journaled.
    // reason: TaskService::submit_task is a load-bearing public API with 30+
    // call sites in tests and the dispatcher. Each caller already constructs
    // the args inline alongside in-context locals; a parameter struct here
    // would force every caller to either build a builder (lots of churn for
    // no clarity win) or use a struct-literal (no better than named args).
    // Revisiting once the dispatcher path consolidates.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_task(
        &self,
        title: Option<String>,
        description: String,
        parent_id: Option<Uuid>,
        priority: TaskPriority,
        agent_type: Option<String>,
        depends_on: Vec<Uuid>,
        context: Option<TaskContext>,
        idempotency_key: Option<String>,
        source: TaskSource,
        deadline: Option<chrono::DateTime<chrono::Utc>>,
        task_type: Option<TaskType>,
        execution_mode: Option<ExecutionMode>,
    ) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut events = Vec::new();

        tracing::info!(
            parent_id = ?parent_id,
            priority = ?priority,
            agent_type = ?agent_type,
            source = ?source,
            execution_mode = ?execution_mode,
            has_idempotency_key = idempotency_key.is_some(),
            "submitting new task"
        );

        // Check for duplicate by idempotency key
        if let Some(ref key) = idempotency_key
            && let Some(existing) = self.task_repo.get_by_idempotency_key(key).await?
        {
            tracing::debug!(task_id = %existing.id, "returning existing task (idempotency dedup)");
            return Ok((existing, events));
        }

        // Validate parent exists if specified, and reject subtask creation
        // under workflow-enrolled tasks that are actively tracking phase subtasks.
        // Tasks in Pending/PhaseReady/PhaseGate/terminal states don't have active
        // subtask tracking, so creating children under them is safe.
        if let Some(pid) = parent_id {
            let parent = self.task_repo.get(pid).await?;
            match parent {
                None => return Err(DomainError::TaskNotFound(pid)),
                Some(ref p) => {
                    if let Some(wf_state) = p.workflow_state()
                        && wf_state.has_tracked_subtasks()
                    {
                        return Err(DomainError::ValidationFailed(format!(
                            "Cannot create subtask under workflow-enrolled task {}. \
                             Use workflow_fan_out() to create workflow subtasks, or \
                             submit this task without a parent_id / under a different parent.",
                            pid
                        )));
                    }
                }
            }
        }

        // Validate dependencies exist
        for dep_id in &depends_on {
            let dep = self.task_repo.get(*dep_id).await?;
            if dep.is_none() {
                return Err(DomainError::TaskNotFound(*dep_id));
            }
        }

        let mut task = match title {
            Some(t) => Task::with_title(t, description),
            None => Task::new(description),
        };
        task = task.with_priority(priority).with_source(source);

        if let Some(pid) = parent_id {
            task = task.with_parent(pid);
        }
        if let Some(agent) = agent_type {
            task = task.with_agent(agent);
        }
        if let Some(key) = idempotency_key {
            task = task.with_idempotency_key(key);
        }
        task.deadline = deadline;
        if let Some(tt) = task_type {
            task = task.with_task_type(tt);
        }

        // Cycle detection for the initial dependency set is enforced inside
        // `TaskRepository::create()`, which calls `add_dependency()` for each edge.
        // `add_dependency()` runs a transitive reachability check (recursive CTE)
        // and returns `DomainError::DependencyCycle` if the new edge would form a cycle.
        for dep in depends_on {
            task = task.with_dependency(dep);
        }

        if let Some(ctx) = context {
            task.context = ctx;
        }

        // --- Execution mode classification heuristic (Part 1.2) ---
        // If the caller explicitly requested an execution mode, use it directly.
        // Otherwise, if the task has the default Direct mode, run the heuristic to
        // determine whether it should be upgraded to Convergent.
        if let Some(explicit_mode) = execution_mode {
            task.execution_mode = explicit_mode;
        } else if task.execution_mode.is_direct() {
            let parent_mode = self.resolve_parent_execution_mode(parent_id).await?;
            let inferred_mode = Self::classify_execution_mode(
                &task,
                parent_mode.as_ref(),
                &self.default_execution_mode,
            );
            task.execution_mode = inferred_mode;
        }

        // --- Auto-enroll in workflow ---
        // Resolve via Config: inline workflows take precedence over YAML workflows
        // loaded from `workflows_dir`. There are no hardcoded fallbacks — if the
        // default workflow can't be resolved, reject the task with a remediation
        // hint rather than creating it unenrolled.
        let config = crate::services::config::Config::load().unwrap_or_default();
        if let Some(wf_name) = Self::infer_workflow_name(&task, &config.default_workflow) {
            if config.resolve_workflow(&wf_name).is_none() {
                return Err(DomainError::ValidationFailed(format!(
                    "Workflow '{}' not found. Run `abathur init` to scaffold the \
                     default workflow YAMLs, or set `workflows_dir` in abathur.toml \
                     to point at a directory that contains them (currently: '{}').",
                    wf_name, config.workflows_dir,
                )));
            }
            task.routing_hints.workflow_name = Some(wf_name.clone());
            let wf_state = WorkflowState::Pending {
                workflow_name: wf_name.clone(),
            };
            let _ = task.set_workflow_state(&wf_state);
        }

        task.validate().map_err(DomainError::ValidationFailed)?;
        self.task_repo.create(&task).await?;
        tracing::info!(task_id = %task.id, status = ?task.status, execution_mode = ?task.execution_mode, "task submitted successfully");

        // Check if task is ready
        self.check_and_update_readiness(&mut task).await?;
        self.task_repo.update(&task).await?;
        // Sync loaded_version so the returned task can be updated again
        // without a re-fetch (optimistic locking bookkeeping).
        task.loaded_version.set(task.version);

        // Collect TaskSubmitted event
        let goal_id = Self::extract_goal_id(&task);
        events.push(Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            goal_id,
            Some(task.id),
            EventPayload::TaskSubmitted {
                task_id: task.id,
                task_title: task.title.clone(),
                goal_id,
            },
        ));

        // Emit WorkflowEnrolled if auto-enrolled
        if task.has_workflow_state()
            && let Some(ref wf_name) = task.routing_hints.workflow_name
        {
            events.push(Self::make_event(
                EventSeverity::Info,
                EventCategory::Workflow,
                goal_id,
                Some(task.id),
                EventPayload::WorkflowEnrolled {
                    task_id: task.id,
                    workflow_name: wf_name.clone(),
                },
            ));
        }

        // If the task is immediately ready (no deps), collect TaskReady event
        if task.status == TaskStatus::Ready {
            events.push(Self::make_event(
                EventSeverity::Debug,
                EventCategory::Task,
                goal_id,
                Some(task.id),
                EventPayload::TaskReady {
                    task_id: task.id,
                    task_title: task.title.clone(),
                },
            ));
        }

        self.publish_events(&events).await;

        // Metrics: task submission — label bounded by TaskType enum variants.
        metrics::counter!(
            "abathur_tasks_submitted_total",
            "template" => task.task_type.as_str()
        )
        .increment(1);

        Ok((task, events))
    }

    /// Check if a task's dependencies are all complete.
    async fn are_dependencies_complete(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(true);
        }

        let deps = self.task_repo.get_dependencies(task.id).await?;
        Ok(deps.iter().all(|d| d.status == TaskStatus::Complete))
    }

    /// Check if any dependency has failed.
    async fn has_failed_dependency(&self, task: &Task) -> DomainResult<bool> {
        if task.depends_on.is_empty() {
            return Ok(false);
        }

        let deps = self.task_repo.get_dependencies(task.id).await?;
        Ok(deps
            .iter()
            .any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled))
    }

    /// Check and update task readiness.
    async fn check_and_update_readiness(&self, task: &mut Task) -> DomainResult<()> {
        if task.status != TaskStatus::Pending {
            return Ok(());
        }

        if self.has_failed_dependency(task).await? {
            if let Err(e) = task.transition_to(TaskStatus::Blocked) {
                tracing::warn!(task_id = %task.id, error = %e, "Failed to transition task to Blocked");
                return Err(DomainError::InvalidStateTransition {
                    from: task.status.as_str().to_string(),
                    to: "blocked".to_string(),
                    reason: e,
                });
            }
        } else if self.are_dependencies_complete(task).await?
            && let Err(e) = task.transition_to(TaskStatus::Ready)
        {
            tracing::warn!(task_id = %task.id, error = %e, "Failed to transition task to Ready");
            return Err(DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "ready".to_string(),
                reason: e,
            });
        }

        Ok(())
    }
}

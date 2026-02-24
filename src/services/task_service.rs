//! Task service implementing business logic.

use std::sync::Arc;
use uuid::Uuid;

use async_trait::async_trait;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{Complexity, ExecutionMode, Task, TaskContext, TaskPriority, TaskSource, TaskStatus, TaskType};
use crate::domain::models::workflow_state::WorkflowState;
use crate::domain::ports::{TaskFilter, TaskRepository};
use crate::services::command_bus::{CommandError, CommandOutcome, CommandResult, TaskCommand, TaskCommandHandler};
use crate::services::event_bus::{
    EventCategory, EventPayload, EventSeverity, UnifiedEvent,
};
use crate::services::event_factory;
use tracing::warn;

/// Configuration for spawn limits.
#[derive(Debug, Clone)]
pub struct SpawnLimitConfig {
    /// Maximum depth of subtask nesting.
    pub max_subtask_depth: u32,
    /// Maximum number of direct subtasks per task.
    pub max_subtasks_per_task: u32,
    /// Maximum total descendants from a root task.
    pub max_total_descendants: u32,
    /// Whether to allow extension requests when limits are reached.
    pub allow_limit_extensions: bool,
}

impl Default for SpawnLimitConfig {
    fn default() -> Self {
        Self {
            max_subtask_depth: 5,
            max_subtasks_per_task: 10,
            max_total_descendants: 100,
            allow_limit_extensions: true,
        }
    }
}

/// Result of spawn limit checking.
#[derive(Debug, Clone)]
pub enum SpawnLimitResult {
    /// Task creation is allowed.
    Allowed,
    /// Limit exceeded but extension may be granted.
    LimitExceeded {
        limit_type: SpawnLimitType,
        current_value: u32,
        limit_value: u32,
        can_request_extension: bool,
    },
    /// Hard limit - cannot create task.
    HardLimit {
        limit_type: SpawnLimitType,
        reason: String,
    },
}

impl SpawnLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn requires_specialist(&self) -> bool {
        matches!(self, Self::LimitExceeded { can_request_extension: true, .. })
    }
}

/// Type of spawn limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnLimitType {
    SubtaskDepth,
    SubtasksPerTask,
    TotalDescendants,
}

impl SpawnLimitType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SubtaskDepth => "subtask_depth",
            Self::SubtasksPerTask => "subtasks_per_task",
            Self::TotalDescendants => "total_descendants",
        }
    }
}

#[derive(Clone)]
pub struct TaskService<T: TaskRepository> {
    task_repo: Arc<T>,
    spawn_limits: SpawnLimitConfig,
    /// Default execution mode override. When `Some`, all tasks without an
    /// explicit execution mode use this value and the classification heuristic
    /// is skipped. When `None`, the heuristic decides. This gives operators a
    /// kill switch (set to `Some(ExecutionMode::Direct)` to disable convergence
    /// inference globally).
    default_execution_mode: Option<ExecutionMode>,
}

impl<T: TaskRepository> TaskService<T> {
    pub fn new(task_repo: Arc<T>) -> Self {
        Self {
            task_repo,
            spawn_limits: SpawnLimitConfig::default(),
            default_execution_mode: None,
        }
    }

    /// Create with custom spawn limits.
    pub fn with_spawn_limits(mut self, limits: SpawnLimitConfig) -> Self {
        self.spawn_limits = limits;
        self
    }

    /// Set the default execution mode override.
    ///
    /// When set to `Some(ExecutionMode::Direct)`, the classification heuristic
    /// is bypassed and all tasks default to direct execution unless they were
    /// explicitly submitted with a convergent mode. When `None`, the heuristic
    /// runs for tasks that don't have an explicit mode set.
    pub fn with_default_execution_mode(mut self, mode: Option<ExecutionMode>) -> Self {
        self.default_execution_mode = mode;
        self
    }

    /// Access the underlying task repository.
    pub fn repo(&self) -> &Arc<T> {
        &self.task_repo
    }

    /// Determine the workflow name for a task based on its characteristics.
    ///
    /// Returns `None` for tasks that should NOT be enrolled (verification tasks,
    /// review tasks, tasks already part of a workflow phase, non-root subtasks).
    fn infer_workflow_name(task: &Task) -> Option<String> {
        // Adapter-sourced tasks -> "external" (triage-first)
        if let TaskSource::Adapter(_) = &task.source {
            return Some("external".to_string());
        }

        // Verification and Review tasks are never enrolled
        if matches!(task.task_type, TaskType::Verification | TaskType::Review) {
            return None;
        }

        // Tasks that are already a workflow phase subtask are never enrolled
        if task.context.custom.contains_key("workflow_phase") {
            return None;
        }

        // Explicit workflow_name hint takes priority
        if let Some(ref name) = task.routing_hints.workflow_name {
            return Some(name.clone());
        }

        // Root tasks (no parent) default to "code"
        if task.parent_id.is_none() {
            return Some("code".to_string());
        }

        // Other subtasks are not enrolled
        None
    }

    /// Helper to build a UnifiedEvent with standard fields.
    fn make_event(
        severity: EventSeverity,
        category: EventCategory,
        goal_id: Option<uuid::Uuid>,
        task_id: Option<uuid::Uuid>,
        payload: EventPayload,
    ) -> UnifiedEvent {
        event_factory::make_event(severity, category, goal_id, task_id, payload)
    }

    /// Check spawn limits for creating a subtask under a parent.
    ///
    /// Returns `SpawnLimitResult` indicating whether the task can be created,
    /// and if not, whether a limit evaluation specialist should be triggered.
    pub async fn check_spawn_limits(&self, parent_id: Option<Uuid>) -> DomainResult<SpawnLimitResult> {
        let Some(parent_id) = parent_id else {
            // No parent = root task, no spawn limits apply
            return Ok(SpawnLimitResult::Allowed);
        };

        let parent = self.task_repo.get(parent_id).await?
            .ok_or(DomainError::TaskNotFound(parent_id))?;

        // Check subtask depth
        let depth = self.calculate_depth(&parent).await?;
        if depth >= self.spawn_limits.max_subtask_depth {
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::SubtaskDepth,
                current_value: depth,
                limit_value: self.spawn_limits.max_subtask_depth,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        // Check direct subtasks count
        let direct_subtasks = self.count_direct_subtasks(parent_id).await?;
        if direct_subtasks >= self.spawn_limits.max_subtasks_per_task {
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::SubtasksPerTask,
                current_value: direct_subtasks,
                limit_value: self.spawn_limits.max_subtasks_per_task,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        // Check total descendants from root
        let root_id = self.find_root_task(&parent).await?;
        let total_descendants = self.count_all_descendants(root_id).await?;
        if total_descendants >= self.spawn_limits.max_total_descendants {
            return Ok(SpawnLimitResult::LimitExceeded {
                limit_type: SpawnLimitType::TotalDescendants,
                current_value: total_descendants,
                limit_value: self.spawn_limits.max_total_descendants,
                can_request_extension: self.spawn_limits.allow_limit_extensions,
            });
        }

        Ok(SpawnLimitResult::Allowed)
    }

    /// Calculate the depth of a task in the hierarchy (0 = root).
    async fn calculate_depth(&self, task: &Task) -> DomainResult<u32> {
        let mut depth = 0;
        let mut current = task.clone();

        while let Some(parent_id) = current.parent_id {
            depth += 1;
            if depth > 100 {
                // Safety limit to prevent infinite loops
                break;
            }
            match self.task_repo.get(parent_id).await? {
                Some(parent) => current = parent,
                None => break,
            }
        }

        Ok(depth)
    }

    /// Count direct subtasks of a task.
    async fn count_direct_subtasks(&self, parent_id: Uuid) -> DomainResult<u32> {
        let filter = TaskFilter {
            parent_id: Some(parent_id),
            ..Default::default()
        };
        let subtasks = self.task_repo.list(filter).await?;
        Ok(subtasks.len() as u32)
    }

    /// Find the root task (task with no parent).
    async fn find_root_task(&self, task: &Task) -> DomainResult<Uuid> {
        let mut current = task.clone();

        while let Some(parent_id) = current.parent_id {
            match self.task_repo.get(parent_id).await? {
                Some(parent) => current = parent,
                None => break,
            }
        }

        Ok(current.id)
    }

    /// Count all descendants of a task using iterative BFS.
    async fn count_all_descendants(&self, task_id: Uuid) -> DomainResult<u32> {
        let mut count = 0u32;
        let mut queue = vec![task_id];

        while let Some(current_id) = queue.pop() {
            let filter = TaskFilter {
                parent_id: Some(current_id),
                ..Default::default()
            };
            let children = self.task_repo.list(filter).await?;

            count += children.len() as u32;
            for child in children {
                queue.push(child.id);
            }

            // Safety limit
            if count > 10000 {
                break;
            }
        }

        Ok(count)
    }

    /// Classify whether a task should use Direct or Convergent execution mode.
    ///
    /// Uses a scoring heuristic based on task complexity, description content,
    /// context hints, source lineage, and priority. A score >= 3 recommends
    /// Convergent mode; below that, Direct mode is used.
    ///
    /// When `default_mode` is `Some(...)`, the operator override takes precedence
    /// and the heuristic is skipped entirely (the operator's mode is returned).
    fn classify_execution_mode(
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
                convergent_score -= 5;
            } else if lower.contains("implement")
                || lower.contains("develop")
                || lower.contains("coder")
                || lower.contains("fixer")
            {
                convergent_score += 5;
            }
        }

        // --- Complexity signals ---
        match task.routing_hints.complexity {
            Complexity::Complex => convergent_score += 3,
            Complexity::Moderate => {
                // Moderate complexity with a lengthy description suggests
                // requirements that benefit from iterative refinement.
                if task.description.split_whitespace().count() > 200 {
                    convergent_score += 2;
                }
            }
            Complexity::Trivial => convergent_score -= 3,
            Complexity::Simple => convergent_score -= 3,
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
            convergent_score += 2;
        }

        // --- Context hints signals ---
        // Anti-patterns and constraints in hints suggest the task needs
        // guardrails that convergence provides.
        let has_anti_patterns = task.context.hints.iter().any(|h| {
            h.starts_with("anti-pattern:") || h.starts_with("constraint:")
        });
        if has_anti_patterns {
            convergent_score += 2;
        }

        // --- Parent inheritance ---
        // Subtasks of convergent parents inherit the convergent mode unless
        // other signals strongly push toward Direct.
        if let TaskSource::SubtaskOf(_) = &task.source {
            if let Some(parent_exec_mode) = parent_mode {
                if parent_exec_mode.is_convergent() {
                    convergent_score += 3;
                }
            }
        }

        // --- Priority signal ---
        // Low priority tasks are "fast-lane": favor Direct execution to
        // minimize latency and token cost.
        if task.priority == TaskPriority::Low {
            convergent_score -= 2;
        }

        // --- Threshold decision ---
        if convergent_score >= 3 {
            ExecutionMode::Convergent { parallel_samples: None }
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

        // Check for duplicate by idempotency key
        if let Some(ref key) = idempotency_key {
            if let Some(existing) = self.task_repo.get_by_idempotency_key(key).await? {
                return Ok((existing, events));
            }
        }

        // Validate parent exists if specified
        if let Some(pid) = parent_id {
            let parent = self.task_repo.get(pid).await?;
            if parent.is_none() {
                return Err(DomainError::TaskNotFound(pid));
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
        task = task.with_priority(priority)
            .with_source(source);

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
        if let Some(wf_name) = Self::infer_workflow_name(&task) {
            // Validate that the inferred workflow name corresponds to a real template.
            // If not, skip enrollment and log a warning rather than creating a task
            // that will fail later when advance() calls get_template().
            let templates = crate::domain::models::workflow_template::WorkflowTemplate::builtin_templates();
            if templates.contains_key(&wf_name) {
                task.routing_hints.workflow_name = Some(wf_name.clone());
                let wf_state = WorkflowState::Pending {
                    workflow_name: wf_name.clone(),
                };
                if let Ok(val) = serde_json::to_value(&wf_state) {
                    task.context.custom.insert("workflow_state".to_string(), val);
                }
            } else {
                warn!(
                    task_id = %task.id,
                    workflow_name = %wf_name,
                    "Skipping workflow auto-enrollment: unknown template"
                );
            }
        }

        task.validate().map_err(DomainError::ValidationFailed)?;
        self.task_repo.create(&task).await?;

        // Check if task is ready
        self.check_and_update_readiness(&mut task).await?;
        self.task_repo.update(&task).await?;

        // Collect TaskSubmitted event
        let goal_id = task.parent_id.unwrap_or_else(Uuid::new_v4);
        events.push(Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            Some(goal_id),
            Some(task.id),
            EventPayload::TaskSubmitted {
                task_id: task.id,
                task_title: task.title.clone(),
                goal_id,
            },
        ));

        // Emit WorkflowEnrolled if auto-enrolled
        if task.context.custom.contains_key("workflow_state") {
            if let Some(ref wf_name) = task.routing_hints.workflow_name {
                events.push(Self::make_event(
                    EventSeverity::Info,
                    EventCategory::Workflow,
                    Some(goal_id),
                    Some(task.id),
                    EventPayload::WorkflowEnrolled {
                        task_id: task.id,
                        workflow_name: wf_name.clone(),
                    },
                ));
            }
        }

        // If the task is immediately ready (no deps), collect TaskReady event
        if task.status == TaskStatus::Ready {
            events.push(Self::make_event(
                EventSeverity::Debug,
                EventCategory::Task,
                Some(goal_id),
                Some(task.id),
                EventPayload::TaskReady {
                    task_id: task.id,
                    task_title: task.title.clone(),
                },
            ));
        }

        Ok((task, events))
    }

    /// Get a task by ID.
    pub async fn get_task(&self, id: Uuid) -> DomainResult<Option<Task>> {
        self.task_repo.get(id).await
    }

    /// List tasks with optional filters.
    pub async fn list_tasks(&self, filter: TaskFilter) -> DomainResult<Vec<Task>> {
        self.task_repo.list(filter).await
    }

    /// Get ready tasks ordered by priority.
    pub async fn get_ready_tasks(&self, limit: usize) -> DomainResult<Vec<Task>> {
        self.task_repo.get_ready_tasks(limit).await
    }

    /// Transition task to Running state (claim it).
    pub async fn claim_task(&self, task_id: Uuid, agent_type: &str) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.status != TaskStatus::Ready {
            return Err(DomainError::InvalidStateTransition {
                from: task.status.as_str().to_string(),
                to: "running".to_string(),
                reason: "task must be in Ready state to be claimed".to_string(),
            });
        }

        task.agent_type = Some(agent_type.to_string());
        task.transition_to(TaskStatus::Running).map_err(|e| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "running".to_string(),
            reason: e,
        })?;

        self.task_repo.update(&task).await?;

        let events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskClaimed {
                task_id,
                agent_type: agent_type.to_string(),
            },
        )];

        Ok((task, events))
    }

    /// Mark task as complete.
    ///
    /// In addition to the standard TaskCompleted event, emits a
    /// `TaskExecutionRecorded` event for opportunistic convergence memory
    /// recording (spec Part 10.3). This lightweight event captures the task's
    /// complexity, execution mode, and success/failure signal. An event handler
    /// downstream persists this data to build the dataset used by the
    /// classification heuristic to learn which complexity levels benefit from
    /// convergence.
    pub async fn complete_task(&self, task_id: Uuid) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Complete).map_err(|e| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "complete".to_string(),
            reason: e,
        })?;

        self.task_repo.update(&task).await?;

        let mut events = vec![Self::make_event(
            EventSeverity::Info,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskCompleted {
                task_id,
                tokens_used: 0,
            },
        )];

        // Opportunistic convergence memory recording (Part 10.3).
        // Emit a lightweight event so that a downstream handler can persist
        // execution metrics. This builds the dataset that informs the
        // classification heuristic over time.
        let execution_mode_str = if task.execution_mode.is_convergent() {
            "convergent".to_string()
        } else {
            "direct".to_string()
        };
        let complexity_str = format!("{:?}", task.routing_hints.complexity).to_lowercase();

        events.push(Self::make_event(
            EventSeverity::Debug,
            EventCategory::Memory,
            None,
            Some(task_id),
            EventPayload::TaskExecutionRecorded {
                task_id,
                execution_mode: execution_mode_str,
                complexity: complexity_str,
                succeeded: true,
                tokens_used: 0, // Actual token count filled by orchestrator-level event
            },
        ));

        Ok((task, events))
    }

    /// Mark task as failed.
    ///
    /// Also emits a `TaskExecutionRecorded` event for opportunistic convergence
    /// memory recording, mirroring the event emitted on success (Part 10.3).
    pub async fn fail_task(&self, task_id: Uuid, error_message: Option<String>) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        task.transition_to(TaskStatus::Failed).map_err(|e| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "failed".to_string(),
            reason: e,
        })?;

        let error_str = error_message.clone().unwrap_or_default();
        if let Some(msg) = error_message {
            task.context.push_hint_bounded(format!("Error: {}", msg));
        }

        self.task_repo.update(&task).await?;

        let execution_mode_str = if task.execution_mode.is_convergent() {
            "convergent".to_string()
        } else {
            "direct".to_string()
        };
        let complexity_str = format!("{:?}", task.routing_hints.complexity).to_lowercase();

        let events = vec![
            Self::make_event(
                EventSeverity::Error,
                EventCategory::Task,
                None,
                Some(task_id),
                EventPayload::TaskFailed {
                    task_id,
                    error: error_str,
                    retry_count: task.retry_count,
                },
            ),
            // Opportunistic convergence memory recording (Part 10.3).
            Self::make_event(
                EventSeverity::Debug,
                EventCategory::Memory,
                None,
                Some(task_id),
                EventPayload::TaskExecutionRecorded {
                    task_id,
                    execution_mode: execution_mode_str,
                    complexity: complexity_str,
                    succeeded: false,
                    tokens_used: 0,
                },
            ),
        ];

        Ok((task, events))
    }

    /// Retry a failed task.
    ///
    /// For convergent tasks (`trajectory_id.is_some()`), the retry intentionally
    /// preserves the trajectory_id. The convergent execution path in the
    /// orchestrator detects `task.trajectory_id.is_some()` and resumes the
    /// existing trajectory (loading accumulated observations, attractor state,
    /// and bandit learning) rather than creating a new one from scratch. This
    /// ensures retry attempts build on previous convergence progress rather
    /// than discarding it. See spec Part 4.2 for full details.
    ///
    /// When a convergent task failed due to being trapped in an attractor
    /// (indicated by an `Error: trapped` hint in context), a `convergence:fresh_start`
    /// hint is added to signal the convergent execution path to force a FreshStart
    /// strategy on the next iteration. This helps escape the attractor by
    /// resetting the working state while carrying forward learned context.
    pub async fn retry_task(&self, task_id: Uuid) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if !task.can_retry() {
            return Err(DomainError::ValidationFailed(
                "Task cannot be retried: either not failed or max retries exceeded".to_string()
            ));
        }

        // For convergent tasks that failed due to being trapped, signal
        // the convergent execution path to force a FreshStart strategy.
        // The trap detection looks for "Error: trapped" hints added by
        // fail_task() when the convergence loop reports a Trapped outcome.
        if task.execution_mode.is_convergent() && task.trajectory_id.is_some() {
            let is_trapped = task.context.hints.iter().any(|h| {
                h.to_lowercase().contains("trapped")
            });
            if is_trapped {
                task.context.push_hint_bounded("convergence:fresh_start".to_string());
            }
        }

        task.retry().map_err(DomainError::ValidationFailed)?;
        self.task_repo.update(&task).await?;

        let events = vec![Self::make_event(
            EventSeverity::Warning,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskRetrying {
                task_id,
                attempt: task.retry_count,
                max_attempts: task.max_retries,
            },
        )];

        Ok((task, events))
    }

    /// Cancel a task.
    pub async fn cancel_task(&self, task_id: Uuid, reason: &str) -> DomainResult<(Task, Vec<UnifiedEvent>)> {
        let mut task = self.task_repo.get(task_id).await?
            .ok_or(DomainError::TaskNotFound(task_id))?;

        if task.is_terminal() {
            return Err(DomainError::ValidationFailed(
                "Cannot cancel a terminal task".to_string()
            ));
        }

        task.transition_to(TaskStatus::Canceled).map_err(|e| DomainError::InvalidStateTransition {
            from: task.status.as_str().to_string(),
            to: "canceled".to_string(),
            reason: e,
        })?;

        self.task_repo.update(&task).await?;

        let events = vec![Self::make_event(
            EventSeverity::Warning,
            EventCategory::Task,
            None,
            Some(task_id),
            EventPayload::TaskCanceled {
                task_id,
                reason: reason.to_string(),
            },
        )];

        Ok((task, events))
    }

    /// Get task status counts.
    pub async fn get_status_counts(&self) -> DomainResult<std::collections::HashMap<TaskStatus, u64>> {
        self.task_repo.count_by_status().await
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
        Ok(deps.iter().any(|d| d.status == TaskStatus::Failed || d.status == TaskStatus::Canceled))
    }

    /// Check and update task readiness.
    async fn check_and_update_readiness(&self, task: &mut Task) -> DomainResult<()> {
        if task.status != TaskStatus::Pending {
            return Ok(());
        }

        if self.has_failed_dependency(task).await? {
            if let Err(e) = task.transition_to(TaskStatus::Blocked) {
                warn!(task_id = %task.id, error = %e, "Failed to transition task to Blocked");
                return Err(DomainError::InvalidStateTransition {
                    from: task.status.as_str().to_string(),
                    to: "blocked".to_string(),
                    reason: e,
                });
            }
        } else if self.are_dependencies_complete(task).await? {
            if let Err(e) = task.transition_to(TaskStatus::Ready) {
                warn!(task_id = %task.id, error = %e, "Failed to transition task to Ready");
                return Err(DomainError::InvalidStateTransition {
                    from: task.status.as_str().to_string(),
                    to: "ready".to_string(),
                    reason: e,
                });
            }
        }

        Ok(())
    }

}

#[async_trait]
impl<T: TaskRepository + 'static> TaskCommandHandler for TaskService<T> {
    async fn handle(&self, cmd: TaskCommand) -> Result<CommandOutcome, CommandError> {
        match cmd {
            TaskCommand::Submit {
                title,
                description,
                parent_id,
                priority,
                agent_type,
                depends_on,
                context,
                idempotency_key,
                source,
                deadline,
                task_type,
                execution_mode,
            } => {
                let (task, events) = self
                    .submit_task(
                        title,
                        description,
                        parent_id,
                        priority,
                        agent_type,
                        depends_on,
                        *context,
                        idempotency_key,
                        source,
                        deadline,
                        task_type,
                        execution_mode,
                    )
                    .await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Claim {
                task_id,
                agent_type,
            } => {
                let (task, events) = self.claim_task(task_id, &agent_type).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Complete { task_id, .. } => {
                let (task, events) = self.complete_task(task_id).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Fail { task_id, error } => {
                let (task, events) = self.fail_task(task_id, error).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Retry { task_id } => {
                let (task, events) = self.retry_task(task_id).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Cancel { task_id, reason } => {
                let (task, events) = self.cancel_task(task_id, &reason).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
            TaskCommand::Assign {
                task_id,
                agent_type,
            } => {
                // Set agent_type on a Ready task without claiming it.
                // The scheduler will pick it up on the next cycle.
                let mut task = self
                    .task_repo
                    .get(task_id)
                    .await?
                    .ok_or(DomainError::TaskNotFound(task_id))?;
                if task.status != TaskStatus::Ready {
                    return Err(DomainError::InvalidStateTransition {
                        from: task.status.as_str().to_string(),
                        to: "ready (assign)".to_string(),
                        reason: "task must be in Ready state to assign an agent_type".to_string(),
                    }.into());
                }
                task.agent_type = Some(agent_type);
                task.updated_at = chrono::Utc::now();
                self.task_repo.update(&task).await?;
                Ok(CommandOutcome { result: CommandResult::Task(task), events: vec![] })
            }
            TaskCommand::Transition {
                task_id,
                new_status,
            } => {
                // Direct transition for reconciliation — load, transition, save.
                let mut task = self
                    .task_repo
                    .get(task_id)
                    .await?
                    .ok_or(DomainError::TaskNotFound(task_id))?;
                task.transition_to(new_status).map_err(|e| {
                    DomainError::InvalidStateTransition {
                        from: task.status.as_str().to_string(),
                        to: new_status.as_str().to_string(),
                        reason: e,
                    }
                })?;
                self.task_repo.update(&task).await?;

                // Collect event for the transition so handlers can react
                let mut events = Vec::new();
                let payload = match new_status {
                    TaskStatus::Ready => Some(EventPayload::TaskReady {
                        task_id,
                        task_title: task.title.clone(),
                    }),
                    TaskStatus::Complete => Some(EventPayload::TaskCompleted {
                        task_id,
                        tokens_used: 0,
                    }),
                    TaskStatus::Failed => Some(EventPayload::TaskFailed {
                        task_id,
                        error: "reconciliation-transition".into(),
                        retry_count: task.retry_count,
                    }),
                    TaskStatus::Canceled => Some(EventPayload::TaskCanceled {
                        task_id,
                        reason: "reconciliation-transition".into(),
                    }),
                    _ => None,
                };
                if let Some(payload) = payload {
                    events.push(Self::make_event(
                        EventSeverity::Info,
                        EventCategory::Task,
                        None,
                        Some(task_id),
                        payload,
                    ));
                }

                Ok(CommandOutcome { result: CommandResult::Task(task), events })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::sqlite::{create_migrated_test_pool, SqliteTaskRepository};

    async fn setup_service() -> TaskService<SqliteTaskRepository> {
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        TaskService::new(task_repo)
    }

    #[tokio::test]
    async fn test_submit_task() {
        let service = setup_service().await;

        let (task, events) = service.submit_task(
            Some("Test Task".to_string()),
            "Description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        assert_eq!(task.title, "Test Task");
        assert_eq!(task.status, TaskStatus::Ready); // No deps, should be ready
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn test_task_dependencies_block_ready() {
        let service = setup_service().await;

        // Create a dependency task
        let (dep, _) = service.submit_task(
            Some("Dependency".to_string()),
            "Must complete first".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        // Create main task that depends on it
        let (main, _) = service.submit_task(
            Some("Main Task".to_string()),
            "Depends on first".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![dep.id],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        // Main should be pending (dependency not complete)
        assert_eq!(main.status, TaskStatus::Pending);

        // Complete the dependency
        service.claim_task(dep.id, "test-agent").await.unwrap();
        service.complete_task(dep.id).await.unwrap();

        // TaskService emits a TaskCompleted event; readiness cascading is handled
        // by the TaskCompletedReadinessHandler in the event reactor, not by
        // TaskService directly. In this unit test (no reactor), the dependent
        // task stays Pending. Full cascade is tested in integration tests.
        let main_updated = service.get_task(main.id).await.unwrap().unwrap();
        assert_eq!(main_updated.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn test_idempotency() {
        let service = setup_service().await;

        let (task1, _) = service.submit_task(
            Some("Task".to_string()),
            "Description".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        let (task2, _) = service.submit_task(
            Some("Different Task".to_string()),
            "Different Description".to_string(),
            None,
            TaskPriority::High,
            None,
            vec![],
            None,
            Some("unique-key".to_string()),
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        // Should return same task
        assert_eq!(task1.id, task2.id);
        assert_eq!(task2.title, "Task"); // Original title
    }

    #[tokio::test]
    async fn test_claim_and_complete() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        let (claimed, _) = service.claim_task(task.id, "test-agent").await.unwrap();
        assert_eq!(claimed.status, TaskStatus::Running);
        assert_eq!(claimed.agent_type, Some("test-agent".to_string()));

        let (completed, _) = service.complete_task(task.id).await.unwrap();
        assert_eq!(completed.status, TaskStatus::Complete);
        assert!(completed.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_fail_and_retry() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        let (failed, _) = service.fail_task(task.id, Some("Test error".to_string())).await.unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);

        let (retried, _) = service.retry_task(task.id).await.unwrap();
        assert_eq!(retried.status, TaskStatus::Ready);
        assert_eq!(retried.retry_count, 1);
    }

    // --- Execution mode classification heuristic tests ---

    #[test]
    fn test_classify_complex_task_as_convergent() {
        let mut task = Task::new("Implement a complex feature with many moving parts");
        task.routing_hints.complexity = Complexity::Complex;

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        assert!(mode.is_convergent(), "Complex tasks should classify as Convergent");
    }

    #[test]
    fn test_classify_trivial_task_as_direct() {
        let mut task = Task::new("Rename a variable");
        task.routing_hints.complexity = Complexity::Trivial;

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        assert!(mode.is_direct(), "Trivial tasks should classify as Direct");
    }

    #[test]
    fn test_classify_simple_task_as_direct() {
        let mut task = Task::new("Add a config field");
        task.routing_hints.complexity = Complexity::Simple;

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        assert!(mode.is_direct(), "Simple tasks should classify as Direct");
    }

    #[test]
    fn test_classify_moderate_short_description_as_direct() {
        let mut task = Task::new("Short description of a moderate task");
        task.routing_hints.complexity = Complexity::Moderate;

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        assert!(mode.is_direct(), "Moderate tasks with short descriptions should be Direct");
    }

    #[test]
    fn test_classify_moderate_long_description_as_convergent() {
        // Build a description with > 200 words and acceptance criteria keywords
        let words: String = (0..210).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
        let desc = format!("{} acceptance criteria: must pass all tests", words);
        let mut task = Task::new(desc);
        task.routing_hints.complexity = Complexity::Moderate;

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        // 2 (long moderate) + 2 (acceptance criteria) = 4 >= 3
        assert!(mode.is_convergent(), "Moderate task with long desc + acceptance criteria should be Convergent");
    }

    #[test]
    fn test_classify_with_anti_pattern_hints() {
        let mut task = Task::new("Fix something with constraints");
        task.routing_hints.complexity = Complexity::Moderate;
        task.context.hints.push("anti-pattern: do not use unwrap".to_string());
        task.context.hints.push("constraint: must preserve backwards compat".to_string());

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        // 0 (moderate, short desc) + 2 (has anti-pattern/constraint) = 2 < 3
        assert!(mode.is_direct(), "Moderate with hints but no other signals stays Direct");

        // Now add acceptance criteria to push over threshold
        task.description = "Fix something. Verify that all tests pass.".to_string();
        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        // 0 + 2 (hints) + 2 (acceptance keyword) = 4 >= 3
        assert!(mode.is_convergent(), "Moderate + hints + acceptance keywords should be Convergent");
    }

    #[test]
    fn test_classify_subtask_inherits_convergent_parent() {
        let parent_id = Uuid::new_v4();
        let mut task = Task::new("Child task of convergent parent");
        task.source = TaskSource::SubtaskOf(parent_id);
        // Default complexity is Moderate, which alone gives 0 points
        // Parent inheritance adds +3

        let parent_mode = ExecutionMode::Convergent { parallel_samples: None };
        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, Some(&parent_mode), &None,
        );
        assert!(mode.is_convergent(), "Subtasks of convergent parents should inherit Convergent");
    }

    #[test]
    fn test_classify_low_priority_pushes_toward_direct() {
        let mut task = Task::new("Something that needs to verify that tests pass");
        task.routing_hints.complexity = Complexity::Moderate;
        task.priority = TaskPriority::Low;
        // acceptance keyword: +2, low priority: -2 = 0 < 3

        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &None,
        );
        assert!(mode.is_direct(), "Low priority should push toward Direct");
    }

    #[test]
    fn test_classify_operator_default_overrides_heuristic() {
        let mut task = Task::new("Complex task that would normally be convergent");
        task.routing_hints.complexity = Complexity::Complex;

        let default_mode = Some(ExecutionMode::Direct);
        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &default_mode,
        );
        assert!(mode.is_direct(), "Operator default_execution_mode should override heuristic");
    }

    #[test]
    fn test_classify_operator_default_convergent() {
        let mut task = Task::new("Simple task");
        task.routing_hints.complexity = Complexity::Simple;

        let default_mode = Some(ExecutionMode::Convergent { parallel_samples: None });
        let mode = TaskService::<SqliteTaskRepository>::classify_execution_mode(
            &task, None, &default_mode,
        );
        assert!(mode.is_convergent(), "Operator default Convergent should override even for simple tasks");
    }

    // --- Trajectory-aware retry tests ---

    #[tokio::test]
    async fn test_retry_convergent_preserves_trajectory_id() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Convergent Task".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        // Manually set convergent mode and trajectory_id (normally done by orchestrator)
        let mut task_updated = service.get_task(task.id).await.unwrap().unwrap();
        task_updated.execution_mode = ExecutionMode::Convergent { parallel_samples: None };
        task_updated.trajectory_id = Some(Uuid::new_v4());
        // Transition to Ready -> Running -> Failed so we can retry
        task_updated.status = TaskStatus::Ready;
        service.task_repo.update(&task_updated).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        service.fail_task(task.id, Some("convergence exhausted".to_string())).await.unwrap();

        let trajectory_before = service.get_task(task.id).await.unwrap().unwrap().trajectory_id;
        let (retried, _) = service.retry_task(task.id).await.unwrap();

        assert_eq!(retried.status, TaskStatus::Ready);
        assert_eq!(retried.trajectory_id, trajectory_before,
            "trajectory_id must be preserved on retry for convergent tasks");
    }

    #[tokio::test]
    async fn test_retry_trapped_convergent_adds_fresh_start_hint() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Trapped Task".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        // Set up as convergent with trajectory
        let mut task_updated = service.get_task(task.id).await.unwrap().unwrap();
        task_updated.execution_mode = ExecutionMode::Convergent { parallel_samples: None };
        task_updated.trajectory_id = Some(Uuid::new_v4());
        task_updated.status = TaskStatus::Ready;
        service.task_repo.update(&task_updated).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        // Fail with "trapped" in the error message — this is what the convergence
        // loop does when LoopControl::Trapped fires.
        service.fail_task(task.id, Some("trapped in FixedPoint attractor".to_string())).await.unwrap();

        let (retried, _) = service.retry_task(task.id).await.unwrap();
        assert!(
            retried.context.hints.iter().any(|h| h == "convergence:fresh_start"),
            "Retrying a trapped convergent task should add convergence:fresh_start hint"
        );
    }

    // --- Opportunistic memory recording tests ---

    #[tokio::test]
    async fn test_complete_task_emits_execution_recorded_event() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        let (_, events) = service.complete_task(task.id).await.unwrap();

        // Should have TaskCompleted + TaskExecutionRecorded
        assert!(events.len() >= 2, "complete_task should emit at least 2 events");
        let recorded = events.iter().find(|e| {
            matches!(&e.payload, EventPayload::TaskExecutionRecorded { succeeded: true, .. })
        });
        assert!(recorded.is_some(), "Should emit TaskExecutionRecorded with succeeded=true");
    }

    #[tokio::test]
    async fn test_fail_task_emits_execution_recorded_event() {
        let service = setup_service().await;

        let (task, _) = service.submit_task(
            Some("Test".to_string()),
            "Desc".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            None,
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        service.claim_task(task.id, "test-agent").await.unwrap();
        let (_, events) = service.fail_task(task.id, Some("boom".to_string())).await.unwrap();

        // Should have TaskFailed + TaskExecutionRecorded
        assert!(events.len() >= 2, "fail_task should emit at least 2 events");
        let recorded = events.iter().find(|e| {
            matches!(&e.payload, EventPayload::TaskExecutionRecorded { succeeded: false, .. })
        });
        assert!(recorded.is_some(), "Should emit TaskExecutionRecorded with succeeded=false");
    }

    // --- with_default_execution_mode builder test ---

    #[tokio::test]
    async fn test_submit_task_respects_default_execution_mode() {
        let pool = create_migrated_test_pool().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(pool));
        let service = TaskService::new(task_repo)
            .with_default_execution_mode(Some(ExecutionMode::Direct));

        // Submit a complex task — normally would be classified as Convergent
        let mut ctx = TaskContext::default();
        ctx.hints.push("anti-pattern: avoid unsafe".to_string());
        let (task, _) = service.submit_task(
            Some("Complex Task".to_string()),
            "This is a complex task that should verify that all tests pass".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            Some(ctx),
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        assert!(task.execution_mode.is_direct(),
            "When default_execution_mode is Direct, heuristic should be skipped");
    }

    #[tokio::test]
    async fn test_submit_complex_task_infers_convergent() {
        let service = setup_service().await;

        // Submit a complex task — heuristic should classify as Convergent
        let mut ctx = TaskContext::default();
        ctx.hints.push("constraint: must preserve API compatibility".to_string());
        let (task, _) = service.submit_task(
            Some("Complex Feature".to_string()),
            "Implement the full OAuth2 flow. Ensure that all integration tests pass.".to_string(),
            None,
            TaskPriority::Normal,
            None,
            vec![],
            Some(ctx),
            None,
            TaskSource::Human,
            None,
            None,
            None,
        ).await.unwrap();

        // Default complexity is Moderate. "ensure that" keyword = +2, constraint hint = +2 => 4 >= 3
        assert!(task.execution_mode.is_convergent(),
            "Task with acceptance criteria + constraints should be inferred as Convergent");
    }
}

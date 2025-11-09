use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

use crate::domain::error::TaskError;

/// Task lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,            // Submitted, dependencies not yet checked
    Blocked,            // Waiting for dependencies
    Ready,              // Dependencies met, ready for execution
    Running,            // Currently executing
    AwaitingValidation, // Execution complete, validation task spawned
    ValidationRunning,  // Validation task is executing
    ValidationFailed,   // Validation found issues (triggers remediation)
    Completed,          // Fully complete (validated if required)
    Failed,             // Task execution failed
    Cancelled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Blocked => write!(f, "blocked"),
            Self::Ready => write!(f, "ready"),
            Self::Running => write!(f, "running"),
            Self::AwaitingValidation => write!(f, "awaiting_validation"),
            Self::ValidationRunning => write!(f, "validation_running"),
            Self::ValidationFailed => write!(f, "validation_failed"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "blocked" => Ok(Self::Blocked),
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "awaiting_validation" | "awaitingvalidation" => Ok(Self::AwaitingValidation),
            "validation_running" | "validationrunning" => Ok(Self::ValidationRunning),
            "validation_failed" | "validationfailed" => Ok(Self::ValidationFailed),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(anyhow::anyhow!("Invalid task status: {s}")),
        }
    }
}

/// Origin of task submission
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    Human,
    AgentRequirements,
    AgentPlanner,
    AgentImplementation,
}

impl fmt::Display for TaskSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Human => write!(f, "human"),
            Self::AgentRequirements => write!(f, "agent_requirements"),
            Self::AgentPlanner => write!(f, "agent_planner"),
            Self::AgentImplementation => write!(f, "agent_implementation"),
        }
    }
}

impl FromStr for TaskSource {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "human" => Ok(Self::Human),
            "agent_requirements" => Ok(Self::AgentRequirements),
            "agent_planner" => Ok(Self::AgentPlanner),
            "agent_implementation" => Ok(Self::AgentImplementation),
            _ => Err(anyhow::anyhow!("Invalid task source: {s}")),
        }
    }
}

/// Type of dependency relationship
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Sequential, // B depends on A completing
    Parallel,   // C depends on A AND B both completing (AND logic)
}

impl fmt::Display for DependencyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sequential => write!(f, "sequential"),
            Self::Parallel => write!(f, "parallel"),
        }
    }
}

impl FromStr for DependencyType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" => Ok(Self::Sequential),
            "parallel" => Ok(Self::Parallel),
            _ => Err(anyhow::anyhow!("Invalid dependency type: {s}")),
        }
    }
}

/// Types of validation requirements for tasks
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValidationRequirement {
    /// No validation needed (e.g., research, documentation)
    None,

    /// Contract validation (e.g., workflow agents spawning children)
    Contract {
        must_spawn_children: bool,
        expected_child_types: Vec<String>,
        min_children: usize,
    },

    /// Test-based validation (e.g., code implementation)
    Testing {
        validator_agent: String,
        test_commands: Vec<String>,
        worktree_required: bool,
        max_remediation_cycles: usize,
    },
}

impl Default for ValidationRequirement {
    fn default() -> Self {
        Self::None
    }
}

/// Workflow state tracking for parent-child task relationships
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowState {
    /// Child tasks spawned by this task
    #[schemars(with = "Vec<String>")]
    pub children_spawned: Vec<Uuid>,

    /// Agent types of spawned children
    pub spawned_agent_types: Vec<String>,

    /// Whether workflow expectations were met
    pub expectations_met: bool,

    /// Timestamp when workflow state was last updated
    pub last_updated: Option<DateTime<Utc>>,
}

/// Workflow expectations for tasks that must spawn children
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowExpectations {
    /// Whether this task must spawn at least one child
    pub must_spawn_child: bool,

    /// Expected agent types for children
    pub expected_child_types: Vec<String>,

    /// Minimum number of children required
    pub min_children: usize,

    /// Maximum number of children allowed (None = unlimited)
    pub max_children: Option<usize>,
}

/// Represents a unit of work in the task queue
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    #[schemars(with = "String")]
    pub id: Uuid,
    pub summary: String,
    pub description: String,
    pub agent_type: String,
    pub priority: u8,
    pub calculated_priority: f64,
    pub status: TaskStatus,
    #[schemars(with = "Option<Vec<String>>")]
    pub dependencies: Option<Vec<Uuid>>,
    pub dependency_type: DependencyType,
    pub dependency_depth: u32,
    pub input_data: Option<serde_json::Value>,
    pub result_data: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub max_execution_timeout_seconds: u32,
    pub submitted_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_updated_at: DateTime<Utc>,
    pub created_by: Option<String>,
    #[schemars(with = "Option<String>")]
    pub parent_task_id: Option<Uuid>,
    #[schemars(with = "Option<String>")]
    pub session_id: Option<Uuid>,
    pub source: TaskSource,
    pub deadline: Option<DateTime<Utc>>,
    pub estimated_duration_seconds: Option<u32>,
    pub feature_branch: Option<String>,
    pub task_branch: Option<String>,
    pub worktree_path: Option<String>,

    // Validation and workflow tracking fields
    /// Validation requirement for this task
    #[serde(default)]
    pub validation_requirement: ValidationRequirement,

    /// ID of the validation task (if spawned)
    #[schemars(with = "Option<String>")]
    pub validation_task_id: Option<Uuid>,

    /// ID of the task being validated (if this is a validation task)
    #[schemars(with = "Option<String>")]
    pub validating_task_id: Option<Uuid>,

    /// Number of remediation cycles so far
    #[serde(default)]
    pub remediation_count: u32,

    /// Whether this task is a remediation task
    #[serde(default)]
    pub is_remediation: bool,

    /// Workflow state (what children were spawned)
    pub workflow_state: Option<WorkflowState>,

    /// Expected workflow behavior
    pub workflow_expectations: Option<WorkflowExpectations>,

    /// Prompt chain ID (if task should execute through a multi-step chain)
    pub chain_id: Option<String>,

    /// Current step index in the prompt chain (0-based)
    #[serde(default)]
    pub chain_step_index: usize,
}

impl Task {
    /// Create a new task with default values
    pub fn new(summary: String, description: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            summary,
            description,
            agent_type: "requirements-gatherer".to_string(),
            priority: 5,
            calculated_priority: 5.0,
            status: TaskStatus::Pending,
            dependencies: None,
            dependency_type: DependencyType::Sequential,
            dependency_depth: 0,
            input_data: None,
            result_data: None,
            error_message: None,
            retry_count: 0,
            max_retries: 3,
            max_execution_timeout_seconds: 3600,
            submitted_at: now,
            started_at: None,
            completed_at: None,
            last_updated_at: now,
            created_by: None,
            parent_task_id: None,
            session_id: None,
            source: TaskSource::Human,
            deadline: None,
            estimated_duration_seconds: None,
            feature_branch: None,
            task_branch: None,
            worktree_path: None,
            validation_requirement: ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
            chain_step_index: 0,
        }
    }

    /// Create a summary with a prefix, ensuring total length doesn't exceed 140 characters
    ///
    /// # Arguments
    ///
    /// * `prefix` - The prefix to add (e.g., "Validate: ", "Fix: ")
    /// * `base_summary` - The base summary text
    ///
    /// # Returns
    ///
    /// A summary string that is guaranteed to be <= 140 characters
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::domain::models::Task;
    ///
    /// let summary = Task::create_summary_with_prefix("Fix: ", "Very long task summary that needs to be truncated");
    /// assert!(summary.len() <= 140);
    /// assert!(summary.starts_with("Fix: "));
    /// ```
    pub fn create_summary_with_prefix(prefix: &str, base_summary: &str) -> String {
        let max_summary_len = 140 - prefix.len();
        let truncated_summary = if base_summary.len() > max_summary_len {
            format!("{}...", &base_summary[..max_summary_len.saturating_sub(3)])
        } else {
            base_summary.to_string()
        };
        format!("{}{}", prefix, truncated_summary)
    }

    /// Validate summary length (max 140 chars)
    pub fn validate_summary(&self) -> Result<(), anyhow::Error> {
        if self.summary.len() > 140 {
            return Err(anyhow::anyhow!("Summary exceeds 140 characters"));
        }
        Ok(())
    }

    /// Validate priority range (0-10)
    pub fn validate_priority(&self) -> Result<(), anyhow::Error> {
        if self.priority > 10 {
            return Err(anyhow::anyhow!("Priority must be between 0 and 10"));
        }
        Ok(())
    }

    // ========================
    // State Transition Methods
    // ========================

    /// Mark task as ready if all dependencies are met
    pub fn mark_ready(&mut self) -> Result<(), TaskError> {
        if self.status != TaskStatus::Pending && self.status != TaskStatus::Blocked {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Ready,
            });
        }

        self.status = TaskStatus::Ready;
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Mark task as blocked due to unresolved dependencies
    pub fn block(&mut self, _unresolved_count: usize) -> Result<(), TaskError> {
        if self.status != TaskStatus::Pending && self.status != TaskStatus::Ready {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Blocked,
            });
        }

        self.status = TaskStatus::Blocked;
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Start task execution (transition to Running)
    pub fn start(&mut self) -> Result<(), TaskError> {
        if self.status != TaskStatus::Ready {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Running,
            });
        }

        self.status = TaskStatus::Running;
        self.started_at = Some(Utc::now());
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Complete task successfully
    pub fn complete(&mut self, result_data: Option<serde_json::Value>) -> Result<(), TaskError> {
        if self.status != TaskStatus::Running {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Completed,
            });
        }

        self.status = TaskStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.last_updated_at = Utc::now();
        self.result_data = result_data;
        Ok(())
    }

    /// Fail task with error message
    pub fn fail(&mut self, error_message: String) -> Result<(), TaskError> {
        if self.status != TaskStatus::Running {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Failed,
            });
        }

        self.status = TaskStatus::Failed;
        self.completed_at = Some(Utc::now());
        self.last_updated_at = Utc::now();
        self.error_message = Some(error_message);
        Ok(())
    }

    /// Cancel task (can be done from any non-terminal state)
    pub fn cancel(&mut self) -> Result<(), TaskError> {
        if self.is_terminal() {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Cancelled,
            });
        }

        self.status = TaskStatus::Cancelled;
        self.completed_at = Some(Utc::now());
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Retry a failed task (increments retry count and resets to Pending)
    pub fn retry(&mut self) -> Result<(), TaskError> {
        if self.status != TaskStatus::Failed {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Pending,
            });
        }

        if !self.can_retry() {
            return Err(TaskError::MaxRetriesExceeded {
                retry_count: self.retry_count,
                max_retries: self.max_retries,
            });
        }

        self.retry_count += 1;
        self.status = TaskStatus::Pending;
        self.started_at = None;
        self.completed_at = None;
        self.error_message = None;
        self.last_updated_at = Utc::now();
        Ok(())
    }

    // ========================
    // Query Methods
    // ========================

    /// Check if task is in a terminal state (cannot transition further)
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Completed | TaskStatus::Cancelled | TaskStatus::Failed
        )
    }

    /// Check if task is ready to execute
    pub fn is_ready(&self) -> bool {
        self.status == TaskStatus::Ready
    }

    /// Check if task is currently running
    pub fn is_running(&self) -> bool {
        self.status == TaskStatus::Running
    }

    /// Check if task can be retried
    pub fn can_retry(&self) -> bool {
        self.status == TaskStatus::Failed && self.retry_count < self.max_retries
    }

    /// Check if task is blocked by dependencies
    pub fn is_blocked(&self) -> bool {
        self.status == TaskStatus::Blocked
    }

    /// Check if task is completed successfully
    pub fn is_completed(&self) -> bool {
        self.status == TaskStatus::Completed
    }

    /// Check if task has failed
    pub fn is_failed(&self) -> bool {
        self.status == TaskStatus::Failed
    }

    /// Check if task is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.status == TaskStatus::Cancelled
    }

    // ========================
    // Business Logic Methods
    // ========================

    /// Calculate effective priority including dependency depth boost
    pub fn calculate_priority(&self) -> f64 {
        // Base priority (0-10) + depth boost (0.5 per level)
        f64::from(self.dependency_depth).mul_add(0.5, f64::from(self.priority))
    }

    /// Update the calculated priority field
    pub fn update_calculated_priority(&mut self) {
        self.calculated_priority = self.calculate_priority();
        self.last_updated_at = Utc::now();
    }

    /// Check if task has exceeded execution timeout
    pub fn is_timed_out(&self) -> bool {
        self.started_at.is_some_and(|started| {
            let elapsed = Utc::now().signed_duration_since(started);
            elapsed
                .num_seconds()
                .try_into()
                .is_ok_and(|secs: u32| secs > self.max_execution_timeout_seconds)
        })
    }

    /// Get elapsed execution time in seconds (None if not started)
    pub fn elapsed_time(&self) -> Option<i64> {
        self.started_at.map(|started| {
            let end = self.completed_at.unwrap_or_else(Utc::now);
            end.signed_duration_since(started).num_seconds()
        })
    }

    /// Check if task has dependencies
    pub fn has_dependencies(&self) -> bool {
        self.dependencies
            .as_ref()
            .is_some_and(|deps| !deps.is_empty())
    }

    /// Get dependency count
    pub fn dependency_count(&self) -> usize {
        self.dependencies.as_ref().map_or(0, std::vec::Vec::len)
    }

    /// Check if all dependencies are in the completed set
    pub fn dependencies_met(&self, completed_tasks: &[Uuid]) -> bool {
        self.dependencies
            .as_ref()
            .is_none_or(|deps| deps.iter().all(|dep_id| completed_tasks.contains(dep_id)))
    }

    /// Update task status based on dependency resolution
    pub fn update_status_for_dependencies(
        &mut self,
        completed_tasks: &[Uuid],
    ) -> Result<(), TaskError> {
        if self.is_terminal() || self.status == TaskStatus::Running {
            return Ok(()); // Don't modify terminal or running tasks
        }

        if self.dependencies_met(completed_tasks) {
            self.mark_ready()?;
        } else {
            let unresolved = self.dependency_count()
                - self
                    .dependencies
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|id| completed_tasks.contains(id))
                    .count();
            self.block(unresolved)?;
        }

        Ok(())
    }

    /// Check if task has exceeded its deadline
    pub fn is_past_deadline(&self) -> bool {
        self.deadline.is_some_and(|deadline| Utc::now() > deadline)
    }

    /// Set dependencies and update dependency depth
    pub fn set_dependencies(
        &mut self,
        dependencies: Vec<Uuid>,
        dependency_type: DependencyType,
        depth: u32,
    ) {
        self.dependencies = Some(dependencies);
        self.dependency_type = dependency_type;
        self.dependency_depth = depth;
        self.update_calculated_priority();
    }

    /// Get dependencies as a slice
    pub fn get_dependencies(&self) -> &[Uuid] {
        self.dependencies.as_deref().unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================
    // Helper Functions
    // ========================

    fn create_test_task() -> Task {
        Task::new("Test task".to_string(), "Test description".to_string())
    }

    // ========================
    // Summary Tests
    // ========================

    #[test]
    fn test_create_summary_with_prefix_short_summary() {
        let summary = Task::create_summary_with_prefix("Fix: ", "Short summary");
        assert_eq!(summary, "Fix: Short summary");
        assert!(summary.len() <= 140);
    }

    #[test]
    fn test_create_summary_with_prefix_long_summary() {
        // Create a summary that's 135 chars (close to limit)
        let long_summary = "a".repeat(135);
        let summary = Task::create_summary_with_prefix("Validate: ", &long_summary);

        // "Validate: " is 10 chars, so max base summary is 130 chars
        // Should truncate to 127 chars + "..." = 130 chars
        // Total: 10 + 130 = 140 chars
        assert_eq!(summary.len(), 140);
        assert!(summary.starts_with("Validate: "));
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn test_create_summary_with_prefix_exactly_at_limit() {
        // Create a summary that when combined with prefix is exactly 140 chars
        let base_summary = "a".repeat(130); // "Validate: " (10) + 130 = 140
        let summary = Task::create_summary_with_prefix("Validate: ", &base_summary);

        assert_eq!(summary.len(), 140);
        assert!(summary.starts_with("Validate: "));
        assert!(!summary.ends_with("..."));
    }

    #[test]
    fn test_create_summary_with_prefix_no_prefix() {
        let long_summary = "a".repeat(150);
        let summary = Task::create_summary_with_prefix("", &long_summary);

        assert_eq!(summary.len(), 140);
        assert!(summary.ends_with("..."));
    }

    // ========================
    // Enum Tests
    // ========================

    #[test]
    fn test_task_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::Blocked.to_string(), "blocked");
        assert_eq!(TaskStatus::Ready.to_string(), "ready");
        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(TaskStatus::Failed.to_string(), "failed");
        assert_eq!(TaskStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn test_task_status_from_str() {
        assert_eq!(
            TaskStatus::from_str("pending").unwrap(),
            TaskStatus::Pending
        );
        assert_eq!(TaskStatus::from_str("READY").unwrap(), TaskStatus::Ready);
        assert!(TaskStatus::from_str("invalid").is_err());
    }

    #[test]
    fn test_task_source_display() {
        assert_eq!(TaskSource::Human.to_string(), "human");
        assert_eq!(
            TaskSource::AgentRequirements.to_string(),
            "agent_requirements"
        );
        assert_eq!(TaskSource::AgentPlanner.to_string(), "agent_planner");
        assert_eq!(
            TaskSource::AgentImplementation.to_string(),
            "agent_implementation"
        );
    }

    #[test]
    fn test_dependency_type_display() {
        assert_eq!(DependencyType::Sequential.to_string(), "sequential");
        assert_eq!(DependencyType::Parallel.to_string(), "parallel");
    }

    // ========================
    // Constructor Tests
    // ========================

    #[test]
    fn test_task_new_creates_valid_task() {
        let task = create_test_task();
        assert_eq!(task.summary, "Test task");
        assert_eq!(task.description, "Test description");
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.priority, 5);
        assert_eq!(task.retry_count, 0);
        assert_eq!(task.max_retries, 3);
    }

    #[test]
    fn test_task_new_sets_timestamps() {
        let task = create_test_task();
        assert!(task.started_at.is_none());
        assert!(task.completed_at.is_none());
        assert!(task.submitted_at <= Utc::now());
    }

    // ========================
    // State Transition Tests
    // ========================

    #[test]
    fn test_mark_ready_from_pending() {
        let mut task = create_test_task();
        assert_eq!(task.status, TaskStatus::Pending);

        task.mark_ready().unwrap();
        assert_eq!(task.status, TaskStatus::Ready);
    }

    #[test]
    fn test_mark_ready_from_blocked() {
        let mut task = create_test_task();
        task.status = TaskStatus::Blocked;

        task.mark_ready().unwrap();
        assert_eq!(task.status, TaskStatus::Ready);
    }

    #[test]
    fn test_mark_ready_from_invalid_state() {
        let mut task = create_test_task();
        task.status = TaskStatus::Running;

        let result = task.mark_ready();
        assert!(result.is_err());
        assert_eq!(task.status, TaskStatus::Running);
    }

    #[test]
    fn test_block_from_pending() {
        let mut task = create_test_task();
        task.block(2).unwrap();
        assert_eq!(task.status, TaskStatus::Blocked);
    }

    #[test]
    fn test_start_from_ready() {
        let mut task = create_test_task();
        task.status = TaskStatus::Ready;

        task.start().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.started_at.is_some());
    }

    #[test]
    fn test_start_from_pending_fails() {
        let mut task = create_test_task();
        assert_eq!(task.status, TaskStatus::Pending);

        let result = task.start();
        assert!(result.is_err());
    }

    #[test]
    fn test_complete_from_running() {
        let mut task = create_test_task();
        task.status = TaskStatus::Ready;
        task.start().unwrap();

        let result_data = Some(serde_json::json!({"result": "success"}));
        task.complete(result_data.clone()).unwrap();

        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.completed_at.is_some());
        assert_eq!(task.result_data, result_data);
    }

    #[test]
    fn test_complete_from_pending_fails() {
        let mut task = create_test_task();
        let result = task.complete(None);
        assert!(result.is_err());
    }

    #[test]
    fn test_fail_from_running() {
        let mut task = create_test_task();
        task.status = TaskStatus::Ready;
        task.start().unwrap();

        task.fail("Test error".to_string()).unwrap();

        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.completed_at.is_some());
        assert_eq!(task.error_message, Some("Test error".to_string()));
    }

    #[test]
    fn test_cancel_from_any_non_terminal_state() {
        let mut task1 = create_test_task();
        task1.cancel().unwrap();
        assert_eq!(task1.status, TaskStatus::Cancelled);

        let mut task2 = create_test_task();
        task2.status = TaskStatus::Ready;
        task2.cancel().unwrap();
        assert_eq!(task2.status, TaskStatus::Cancelled);

        let mut task3 = create_test_task();
        task3.status = TaskStatus::Running;
        task3.cancel().unwrap();
        assert_eq!(task3.status, TaskStatus::Cancelled);
    }

    #[test]
    fn test_cancel_from_terminal_state_fails() {
        let mut task = create_test_task();
        task.status = TaskStatus::Completed;

        let result = task.cancel();
        assert!(result.is_err());
    }

    #[test]
    fn test_retry_failed_task() {
        let mut task = create_test_task();
        task.status = TaskStatus::Failed;
        task.retry_count = 1;

        task.retry().unwrap();

        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.retry_count, 2);
        assert!(task.started_at.is_none());
        assert!(task.completed_at.is_none());
        assert!(task.error_message.is_none());
    }

    #[test]
    fn test_retry_exceeds_max_retries() {
        let mut task = create_test_task();
        task.status = TaskStatus::Failed;
        task.retry_count = 3;
        task.max_retries = 3;

        let result = task.retry();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TaskError::MaxRetriesExceeded { .. }
        ));
    }

    #[test]
    fn test_retry_from_non_failed_state_fails() {
        let mut task = create_test_task();
        task.status = TaskStatus::Pending;

        let result = task.retry();
        assert!(result.is_err());
    }

    // ========================
    // Query Method Tests
    // ========================

    #[test]
    fn test_is_terminal() {
        let mut task = create_test_task();
        assert!(!task.is_terminal());

        task.status = TaskStatus::Completed;
        assert!(task.is_terminal());

        task.status = TaskStatus::Failed;
        assert!(task.is_terminal());

        task.status = TaskStatus::Cancelled;
        assert!(task.is_terminal());
    }

    #[test]
    fn test_is_ready() {
        let mut task = create_test_task();
        assert!(!task.is_ready());

        task.status = TaskStatus::Ready;
        assert!(task.is_ready());
    }

    #[test]
    fn test_is_running() {
        let mut task = create_test_task();
        assert!(!task.is_running());

        task.status = TaskStatus::Running;
        assert!(task.is_running());
    }

    #[test]
    fn test_can_retry() {
        let mut task = create_test_task();
        task.status = TaskStatus::Failed;
        task.retry_count = 2;
        task.max_retries = 3;
        assert!(task.can_retry());

        task.retry_count = 3;
        assert!(!task.can_retry());

        task.status = TaskStatus::Completed;
        assert!(!task.can_retry());
    }

    #[test]
    fn test_is_blocked() {
        let mut task = create_test_task();
        assert!(!task.is_blocked());

        task.status = TaskStatus::Blocked;
        assert!(task.is_blocked());
    }

    #[test]
    fn test_is_completed() {
        let mut task = create_test_task();
        assert!(!task.is_completed());

        task.status = TaskStatus::Completed;
        assert!(task.is_completed());
    }

    #[test]
    fn test_is_failed() {
        let mut task = create_test_task();
        assert!(!task.is_failed());

        task.status = TaskStatus::Failed;
        assert!(task.is_failed());
    }

    #[test]
    fn test_is_cancelled() {
        let mut task = create_test_task();
        assert!(!task.is_cancelled());

        task.status = TaskStatus::Cancelled;
        assert!(task.is_cancelled());
    }

    // ========================
    // Business Logic Tests
    // ========================

    #[test]
    fn test_calculate_priority_base() {
        let task = create_test_task();
        assert!((task.calculate_priority() - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_priority_with_depth() {
        let mut task = create_test_task();
        task.dependency_depth = 2;
        assert!((task.calculate_priority() - 6.0).abs() < f64::EPSILON); // 5 + (2 * 0.5)

        task.dependency_depth = 4;
        assert!((task.calculate_priority() - 7.0).abs() < f64::EPSILON); // 5 + (4 * 0.5)
    }

    #[test]
    fn test_update_calculated_priority() {
        let mut task = create_test_task();
        task.dependency_depth = 3;

        task.update_calculated_priority();
        assert!((task.calculated_priority - 6.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_is_timed_out() {
        let mut task = create_test_task();
        assert!(!task.is_timed_out());

        task.started_at = Some(Utc::now() - chrono::Duration::seconds(3700));
        task.max_execution_timeout_seconds = 3600;
        assert!(task.is_timed_out());

        task.max_execution_timeout_seconds = 4000;
        assert!(!task.is_timed_out());
    }

    #[test]
    fn test_elapsed_time_not_started() {
        let task = create_test_task();
        assert!(task.elapsed_time().is_none());
    }

    #[test]
    fn test_elapsed_time_running() {
        let mut task = create_test_task();
        task.started_at = Some(Utc::now() - chrono::Duration::seconds(10));

        let elapsed = task.elapsed_time().unwrap();
        assert!(elapsed >= 10);
    }

    #[test]
    fn test_elapsed_time_completed() {
        let mut task = create_test_task();
        task.started_at = Some(Utc::now() - chrono::Duration::seconds(20));
        task.completed_at = Some(Utc::now() - chrono::Duration::seconds(5));

        let elapsed = task.elapsed_time().unwrap();
        assert!((15..=16).contains(&elapsed));
    }

    #[test]
    fn test_has_dependencies() {
        let mut task = create_test_task();
        assert!(!task.has_dependencies());

        task.dependencies = Some(vec![Uuid::new_v4()]);
        assert!(task.has_dependencies());

        task.dependencies = Some(vec![]);
        assert!(!task.has_dependencies());
    }

    #[test]
    fn test_dependency_count() {
        let mut task = create_test_task();
        assert_eq!(task.dependency_count(), 0);

        task.dependencies = Some(vec![Uuid::new_v4(), Uuid::new_v4()]);
        assert_eq!(task.dependency_count(), 2);
    }

    #[test]
    fn test_dependencies_met_no_dependencies() {
        let task = create_test_task();
        assert!(task.dependencies_met(&[]));
    }

    #[test]
    fn test_dependencies_met_all_completed() {
        let dep1 = Uuid::new_v4();
        let dep2 = Uuid::new_v4();

        let mut task = create_test_task();
        task.dependencies = Some(vec![dep1, dep2]);

        assert!(task.dependencies_met(&[dep1, dep2]));
    }

    #[test]
    fn test_dependencies_met_some_incomplete() {
        let dep1 = Uuid::new_v4();
        let dep2 = Uuid::new_v4();

        let mut task = create_test_task();
        task.dependencies = Some(vec![dep1, dep2]);

        assert!(!task.dependencies_met(&[dep1]));
    }

    #[test]
    fn test_update_status_for_dependencies_met() {
        let dep1 = Uuid::new_v4();
        let mut task = create_test_task();
        task.dependencies = Some(vec![dep1]);

        task.update_status_for_dependencies(&[dep1]).unwrap();
        assert_eq!(task.status, TaskStatus::Ready);
    }

    #[test]
    fn test_update_status_for_dependencies_blocked() {
        let dep1 = Uuid::new_v4();
        let mut task = create_test_task();
        task.dependencies = Some(vec![dep1]);

        task.update_status_for_dependencies(&[]).unwrap();
        assert_eq!(task.status, TaskStatus::Blocked);
    }

    #[test]
    fn test_update_status_for_dependencies_ignores_terminal() {
        let mut task = create_test_task();
        task.status = TaskStatus::Completed;

        task.update_status_for_dependencies(&[]).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    #[test]
    fn test_is_past_deadline() {
        let mut task = create_test_task();
        assert!(!task.is_past_deadline());

        task.deadline = Some(Utc::now() + chrono::Duration::hours(1));
        assert!(!task.is_past_deadline());

        task.deadline = Some(Utc::now() - chrono::Duration::hours(1));
        assert!(task.is_past_deadline());
    }

    #[test]
    fn test_set_dependencies() {
        let dep1 = Uuid::new_v4();
        let dep2 = Uuid::new_v4();

        let mut task = create_test_task();
        task.set_dependencies(vec![dep1, dep2], DependencyType::Parallel, 3);

        assert_eq!(task.dependencies, Some(vec![dep1, dep2]));
        assert_eq!(task.dependency_type, DependencyType::Parallel);
        assert_eq!(task.dependency_depth, 3);
        assert!((task.calculated_priority - 6.5).abs() < f64::EPSILON); // 5 + (3 * 0.5)
    }

    // ========================
    // Validation Tests
    // ========================

    #[test]
    fn test_validate_summary_success() {
        let task = create_test_task();
        assert!(task.validate_summary().is_ok());
    }

    #[test]
    fn test_validate_summary_too_long() {
        let mut task = create_test_task();
        task.summary = "a".repeat(141);
        assert!(task.validate_summary().is_err());
    }

    #[test]
    fn test_validate_priority_success() {
        let task = create_test_task();
        assert!(task.validate_priority().is_ok());
    }

    #[test]
    fn test_validate_priority_too_high() {
        let mut task = create_test_task();
        task.priority = 11;
        assert!(task.validate_priority().is_err());
    }

    // ========================
    // Serialization Tests
    // ========================

    #[test]
    fn test_task_serialization() {
        let task = create_test_task();
        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("pending"));
        assert!(json.contains("Test task"));
    }

    #[test]
    fn test_task_deserialization() {
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "summary": "Test",
            "description": "Description",
            "agent_type": "test-agent",
            "priority": 5,
            "calculated_priority": 5.0,
            "status": "pending",
            "dependencies": null,
            "dependency_type": "sequential",
            "dependency_depth": 0,
            "input_data": null,
            "result_data": null,
            "error_message": null,
            "retry_count": 0,
            "max_retries": 3,
            "max_execution_timeout_seconds": 3600,
            "submitted_at": "2024-01-01T00:00:00Z",
            "started_at": null,
            "completed_at": null,
            "last_updated_at": "2024-01-01T00:00:00Z",
            "created_by": null,
            "parent_task_id": null,
            "session_id": null,
            "source": "human",
            "deadline": null,
            "estimated_duration_seconds": null,
            "feature_branch": null,
            "task_branch": null,
            "worktree_path": null
        }"#;

        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.summary, "Test");
        assert_eq!(task.status, TaskStatus::Pending);
    }
}

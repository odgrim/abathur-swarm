use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

use crate::domain::error::TaskError;

/// Task lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending, // Submitted, dependencies not yet checked
    Blocked, // Waiting for dependencies
    Ready,   // Dependencies met, ready for execution
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Blocked => write!(f, "blocked"),
            TaskStatus::Ready => write!(f, "ready"),
            TaskStatus::Running => write!(f, "running"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Failed => write!(f, "failed"),
            TaskStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(TaskStatus::Pending),
            "blocked" => Ok(TaskStatus::Blocked),
            "ready" => Ok(TaskStatus::Ready),
            "running" => Ok(TaskStatus::Running),
            "completed" => Ok(TaskStatus::Completed),
            "failed" => Ok(TaskStatus::Failed),
            "cancelled" => Ok(TaskStatus::Cancelled),
            _ => Err(anyhow::anyhow!("Invalid task status: {}", s)),
        }
    }
}

/// Origin of task submission
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            TaskSource::Human => write!(f, "human"),
            TaskSource::AgentRequirements => write!(f, "agent_requirements"),
            TaskSource::AgentPlanner => write!(f, "agent_planner"),
            TaskSource::AgentImplementation => write!(f, "agent_implementation"),
        }
    }
}

impl FromStr for TaskSource {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "human" => Ok(TaskSource::Human),
            "agent_requirements" => Ok(TaskSource::AgentRequirements),
            "agent_planner" => Ok(TaskSource::AgentPlanner),
            "agent_implementation" => Ok(TaskSource::AgentImplementation),
            _ => Err(anyhow::anyhow!("Invalid task source: {}", s)),
        }
    }
}

/// Type of dependency relationship
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Sequential, // B depends on A completing
    Parallel,   // C depends on A AND B both completing (AND logic)
}

impl fmt::Display for DependencyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyType::Sequential => write!(f, "sequential"),
            DependencyType::Parallel => write!(f, "parallel"),
        }
    }
}

impl FromStr for DependencyType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" => Ok(DependencyType::Sequential),
            "parallel" => Ok(DependencyType::Parallel),
            _ => Err(anyhow::anyhow!("Invalid dependency type: {}", s)),
        }
    }
}

/// Represents a unit of work in the task queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub summary: String,
    pub description: String,
    pub agent_type: String,
    pub priority: u8,
    pub calculated_priority: f64,
    pub status: TaskStatus,
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
    pub parent_task_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub source: TaskSource,
    pub deadline: Option<DateTime<Utc>>,
    pub estimated_duration_seconds: Option<u32>,
    pub feature_branch: Option<String>,
    pub task_branch: Option<String>,
    pub worktree_path: Option<String>,
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
        }
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

    // ==================== State Transition Methods ====================

    /// Check if status can transition to target status
    pub fn can_transition_to(&self, target: TaskStatus) -> bool {
        TaskStatus::is_valid_transition(self.status, target)
    }

    /// Mark task as ready if it's currently pending or blocked
    pub fn mark_ready(&mut self) -> Result<(), TaskError> {
        if !self.can_transition_to(TaskStatus::Ready) {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Ready,
            });
        }
        self.status = TaskStatus::Ready;
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Start task execution (transition to Running)
    pub fn start(&mut self) -> Result<(), TaskError> {
        if !self.can_transition_to(TaskStatus::Running) {
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
    pub fn complete(&mut self) -> Result<(), TaskError> {
        if !self.can_transition_to(TaskStatus::Completed) {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Completed,
            });
        }
        self.status = TaskStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Mark task as failed with error message
    pub fn fail(&mut self, error_message: String) -> Result<(), TaskError> {
        if !self.can_transition_to(TaskStatus::Failed) {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Failed,
            });
        }
        self.status = TaskStatus::Failed;
        self.error_message = Some(error_message);
        self.completed_at = Some(Utc::now());
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Cancel task
    pub fn cancel(&mut self) -> Result<(), TaskError> {
        if !self.can_transition_to(TaskStatus::Cancelled) {
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

    /// Mark task as blocked
    pub fn block(&mut self) -> Result<(), TaskError> {
        if !self.can_transition_to(TaskStatus::Blocked) {
            return Err(TaskError::InvalidStateTransition {
                from: self.status,
                to: TaskStatus::Blocked,
            });
        }
        self.status = TaskStatus::Blocked;
        self.last_updated_at = Utc::now();
        Ok(())
    }

    // ==================== Business Logic Query Methods ====================

    /// Check if task is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }

    /// Check if task is ready for execution
    pub fn is_ready(&self) -> bool {
        matches!(self.status, TaskStatus::Ready)
    }

    /// Check if task is currently running
    pub fn is_running(&self) -> bool {
        matches!(self.status, TaskStatus::Running)
    }

    /// Check if task can be retried
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    /// Increment retry count
    pub fn increment_retry(&mut self) -> Result<(), TaskError> {
        if !self.can_retry() {
            return Err(TaskError::MaxRetriesExceeded {
                retry_count: self.retry_count,
                max_retries: self.max_retries,
            });
        }
        self.retry_count += 1;
        self.last_updated_at = Utc::now();
        Ok(())
    }

    /// Calculate effective priority (base priority + depth boost)
    pub fn calculate_priority(&self) -> f64 {
        self.priority as f64 + (self.dependency_depth as f64 * 0.5)
    }

    /// Update calculated priority
    pub fn update_calculated_priority(&mut self) {
        self.calculated_priority = self.calculate_priority();
        self.last_updated_at = Utc::now();
    }

    /// Check if task has dependencies
    pub fn has_dependencies(&self) -> bool {
        self.dependencies
            .as_ref()
            .map(|deps| !deps.is_empty())
            .unwrap_or(false)
    }

    /// Get elapsed time since start (if running)
    pub fn elapsed_time(&self) -> Option<chrono::Duration> {
        self.started_at
            .map(|start| Utc::now().signed_duration_since(start))
    }

    /// Check if execution has timed out
    pub fn is_timed_out(&self) -> bool {
        if let Some(elapsed) = self.elapsed_time() {
            elapsed.num_seconds() as u32 > self.max_execution_timeout_seconds
        } else {
            false
        }
    }
}

impl TaskStatus {
    /// Check if transition from current status to target status is valid
    pub fn is_valid_transition(from: TaskStatus, to: TaskStatus) -> bool {
        match (from, to) {
            // Pending can go to Blocked, Ready, or Cancelled
            (TaskStatus::Pending, TaskStatus::Blocked) => true,
            (TaskStatus::Pending, TaskStatus::Ready) => true,
            (TaskStatus::Pending, TaskStatus::Cancelled) => true,

            // Blocked can go to Ready or Cancelled
            (TaskStatus::Blocked, TaskStatus::Ready) => true,
            (TaskStatus::Blocked, TaskStatus::Cancelled) => true,

            // Ready can go to Running or Cancelled
            (TaskStatus::Ready, TaskStatus::Running) => true,
            (TaskStatus::Ready, TaskStatus::Cancelled) => true,

            // Running can go to Completed, Failed, or Cancelled
            (TaskStatus::Running, TaskStatus::Completed) => true,
            (TaskStatus::Running, TaskStatus::Failed) => true,
            (TaskStatus::Running, TaskStatus::Cancelled) => true,

            // Failed can go to Ready (for retry) or Cancelled
            (TaskStatus::Failed, TaskStatus::Ready) => true,
            (TaskStatus::Failed, TaskStatus::Cancelled) => true,

            // Terminal states cannot transition further
            (TaskStatus::Completed, _) => false,
            (TaskStatus::Cancelled, _) => false,

            // Same state is allowed (no-op)
            (a, b) if a == b => true,

            // All other transitions are invalid
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "pending".parse::<TaskStatus>().unwrap(),
            TaskStatus::Pending
        );
        assert_eq!(
            "BLOCKED".parse::<TaskStatus>().unwrap(),
            TaskStatus::Blocked
        );
        assert_eq!("Ready".parse::<TaskStatus>().unwrap(), TaskStatus::Ready);
        assert_eq!(
            "running".parse::<TaskStatus>().unwrap(),
            TaskStatus::Running
        );
        assert_eq!(
            "completed".parse::<TaskStatus>().unwrap(),
            TaskStatus::Completed
        );
        assert_eq!("failed".parse::<TaskStatus>().unwrap(), TaskStatus::Failed);
        assert_eq!(
            "cancelled".parse::<TaskStatus>().unwrap(),
            TaskStatus::Cancelled
        );
        assert!("invalid".parse::<TaskStatus>().is_err());
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
    fn test_task_source_from_str() {
        assert_eq!("human".parse::<TaskSource>().unwrap(), TaskSource::Human);
        assert_eq!(
            "AGENT_REQUIREMENTS".parse::<TaskSource>().unwrap(),
            TaskSource::AgentRequirements
        );
        assert_eq!(
            "agent_planner".parse::<TaskSource>().unwrap(),
            TaskSource::AgentPlanner
        );
        assert_eq!(
            "Agent_Implementation".parse::<TaskSource>().unwrap(),
            TaskSource::AgentImplementation
        );
        assert!("invalid".parse::<TaskSource>().is_err());
    }

    #[test]
    fn test_dependency_type_display() {
        assert_eq!(DependencyType::Sequential.to_string(), "sequential");
        assert_eq!(DependencyType::Parallel.to_string(), "parallel");
    }

    #[test]
    fn test_dependency_type_from_str() {
        assert_eq!(
            "sequential".parse::<DependencyType>().unwrap(),
            DependencyType::Sequential
        );
        assert_eq!(
            "PARALLEL".parse::<DependencyType>().unwrap(),
            DependencyType::Parallel
        );
        assert!("invalid".parse::<DependencyType>().is_err());
    }

    #[test]
    fn test_task_new() {
        let task = Task::new(
            "Test task".to_string(),
            "A test task description".to_string(),
        );

        assert_eq!(task.summary, "Test task");
        assert_eq!(task.description, "A test task description");
        assert_eq!(task.agent_type, "requirements-gatherer");
        assert_eq!(task.priority, 5);
        assert_eq!(task.calculated_priority, 5.0);
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.dependencies, None);
        assert_eq!(task.dependency_type, DependencyType::Sequential);
        assert_eq!(task.dependency_depth, 0);
        assert_eq!(task.retry_count, 0);
        assert_eq!(task.max_retries, 3);
        assert_eq!(task.max_execution_timeout_seconds, 3600);
        assert_eq!(task.source, TaskSource::Human);
        assert_eq!(task.input_data, None);
        assert_eq!(task.result_data, None);
        assert_eq!(task.error_message, None);
        assert_eq!(task.created_by, None);
        assert_eq!(task.parent_task_id, None);
        assert_eq!(task.session_id, None);
        assert_eq!(task.started_at, None);
        assert_eq!(task.completed_at, None);
        assert_eq!(task.deadline, None);
        assert_eq!(task.estimated_duration_seconds, None);
        assert_eq!(task.feature_branch, None);
        assert_eq!(task.task_branch, None);
        assert_eq!(task.worktree_path, None);
    }

    #[test]
    fn test_validate_summary_valid() {
        let task = Task::new("Short summary".to_string(), "Description".to_string());
        assert!(task.validate_summary().is_ok());
    }

    #[test]
    fn test_validate_summary_max_length() {
        // Exactly 140 characters should be valid
        let summary = "a".repeat(140);
        let task = Task::new(summary, "Description".to_string());
        assert!(task.validate_summary().is_ok());
    }

    #[test]
    fn test_validate_summary_too_long() {
        // 141 characters should fail
        let summary = "a".repeat(141);
        let task = Task::new(summary, "Description".to_string());
        let result = task.validate_summary();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds 140 characters")
        );
    }

    #[test]
    fn test_validate_priority_valid() {
        for priority in 0..=10 {
            let mut task = Task::new("Test".to_string(), "Test".to_string());
            task.priority = priority;
            assert!(
                task.validate_priority().is_ok(),
                "Priority {} should be valid",
                priority
            );
        }
    }

    #[test]
    fn test_validate_priority_too_high() {
        let mut task = Task::new("Test".to_string(), "Test".to_string());
        task.priority = 11;
        let result = task.validate_priority();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be between 0 and 10")
        );
    }

    #[test]
    fn test_task_with_dependencies() {
        let dep_id = Uuid::new_v4();
        let mut task = Task::new("Test".to_string(), "Test".to_string());
        task.dependencies = Some(vec![dep_id]);
        task.dependency_type = DependencyType::Parallel;
        task.dependency_depth = 2;

        assert_eq!(task.dependencies, Some(vec![dep_id]));
        assert_eq!(task.dependency_type, DependencyType::Parallel);
        assert_eq!(task.dependency_depth, 2);
    }

    #[test]
    fn test_task_serialization() {
        let task = Task::new("Test task".to_string(), "Description".to_string());
        let serialized = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&serialized).unwrap();

        assert_eq!(task.summary, deserialized.summary);
        assert_eq!(task.description, deserialized.description);
        assert_eq!(task.status, deserialized.status);
        assert_eq!(task.source, deserialized.source);
    }

    #[test]
    fn test_task_status_serialization() {
        let status = TaskStatus::Running;
        let serialized = serde_json::to_string(&status).unwrap();
        assert_eq!(serialized, "\"running\"");
        let deserialized: TaskStatus = serde_json::from_str(&serialized).unwrap();
        assert_eq!(status, deserialized);
    }

    #[test]
    fn test_task_source_serialization() {
        let source = TaskSource::AgentPlanner;
        let serialized = serde_json::to_string(&source).unwrap();
        assert_eq!(serialized, "\"agent_planner\"");
        let deserialized: TaskSource = serde_json::from_str(&serialized).unwrap();
        assert_eq!(source, deserialized);
    }

    #[test]
    fn test_dependency_type_serialization() {
        let dep_type = DependencyType::Parallel;
        let serialized = serde_json::to_string(&dep_type).unwrap();
        assert_eq!(serialized, "\"parallel\"");
        let deserialized: DependencyType = serde_json::from_str(&serialized).unwrap();
        assert_eq!(dep_type, deserialized);
    }
}

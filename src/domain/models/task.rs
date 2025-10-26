use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Task lifecycle states
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Check if dependencies exist (helper for validation)
    pub fn has_dependencies(&self) -> bool {
        self.dependencies.as_ref().is_some_and(|deps| !deps.is_empty())
    }

    /// Get dependencies as a slice
    pub fn get_dependencies(&self) -> &[Uuid] {
        self.dependencies.as_deref().unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("Test task".to_string(), "Test description".to_string());
        assert_eq!(task.summary, "Test task");
        assert_eq!(task.description, "Test description");
        assert_eq!(task.priority, 5);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(!task.has_dependencies());
    }

    #[test]
    fn test_validate_summary() {
        let mut task = Task::new("Valid".to_string(), "Desc".to_string());
        assert!(task.validate_summary().is_ok());

        task.summary = "a".repeat(141);
        assert!(task.validate_summary().is_err());
    }

    #[test]
    fn test_validate_priority() {
        let mut task = Task::new("Test".to_string(), "Desc".to_string());
        task.priority = 10;
        assert!(task.validate_priority().is_ok());

        task.priority = 11;
        assert!(task.validate_priority().is_err());
    }

    #[test]
    fn test_task_status_from_str() {
        assert_eq!(
            TaskStatus::from_str("pending").unwrap(),
            TaskStatus::Pending
        );
        assert_eq!(
            TaskStatus::from_str("RUNNING").unwrap(),
            TaskStatus::Running
        );
        assert!(TaskStatus::from_str("invalid").is_err());
    }

    #[test]
    fn test_task_has_dependencies() {
        let mut task = Task::new("Test".to_string(), "Desc".to_string());
        assert!(!task.has_dependencies());

        task.dependencies = Some(vec![Uuid::new_v4()]);
        assert!(task.has_dependencies());

        task.dependencies = Some(vec![]);
        assert!(!task.has_dependencies());
    }
}

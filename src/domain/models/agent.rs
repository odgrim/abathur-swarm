use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Agent execution states
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Busy,
    Terminated,
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentStatus::Idle => write!(f, "idle"),
            AgentStatus::Busy => write!(f, "busy"),
            AgentStatus::Terminated => write!(f, "terminated"),
        }
    }
}

impl FromStr for AgentStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(AgentStatus::Idle),
            "busy" => Ok(AgentStatus::Busy),
            "terminated" => Ok(AgentStatus::Terminated),
            _ => Err(anyhow::anyhow!("Invalid agent status: {}", s)),
        }
    }
}

/// Represents an agent that can execute tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub agent_type: String,
    pub status: AgentStatus,
    pub current_task_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
    pub memory_usage_bytes: u64,
    pub cpu_usage_percent: f64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
}

impl Agent {
    /// Create a new agent with the specified agent type
    pub fn new(agent_type: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            agent_type,
            status: AgentStatus::Idle,
            current_task_id: None,
            created_at: now,
            heartbeat_at: now,
            memory_usage_bytes: 0,
            cpu_usage_percent: 0.0,
            tasks_completed: 0,
            tasks_failed: 0,
        }
    }

    /// Check if the agent is idle
    pub fn is_idle(&self) -> bool {
        matches!(self.status, AgentStatus::Idle)
    }

    /// Check if the agent is busy
    pub fn is_busy(&self) -> bool {
        matches!(self.status, AgentStatus::Busy)
    }

    /// Check if the agent is terminated
    pub fn is_terminated(&self) -> bool {
        matches!(self.status, AgentStatus::Terminated)
    }

    /// Assign a task to the agent
    pub fn assign_task(&mut self, task_id: Uuid) -> Result<(), anyhow::Error> {
        if self.is_terminated() {
            return Err(anyhow::anyhow!(
                "Cannot assign task to terminated agent {}",
                self.id
            ));
        }
        if self.is_busy() {
            return Err(anyhow::anyhow!(
                "Agent {} is already busy with task {:?}",
                self.id,
                self.current_task_id
            ));
        }

        self.status = AgentStatus::Busy;
        self.current_task_id = Some(task_id);
        Ok(())
    }

    /// Complete the current task
    pub fn complete_task(&mut self, success: bool) -> Result<(), anyhow::Error> {
        if !self.is_busy() {
            return Err(anyhow::anyhow!(
                "Agent {} is not busy, cannot complete task",
                self.id
            ));
        }

        self.status = AgentStatus::Idle;
        self.current_task_id = None;

        if success {
            self.tasks_completed += 1;
        } else {
            self.tasks_failed += 1;
        }

        Ok(())
    }

    /// Update the agent's heartbeat timestamp
    pub fn update_heartbeat(&mut self) {
        self.heartbeat_at = Utc::now();
    }

    /// Update resource usage metrics
    pub fn update_resources(&mut self, memory_bytes: u64, cpu_percent: f64) {
        self.memory_usage_bytes = memory_bytes;
        self.cpu_usage_percent = cpu_percent;
    }

    /// Terminate the agent
    pub fn terminate(&mut self) -> Result<(), anyhow::Error> {
        if self.is_busy() {
            return Err(anyhow::anyhow!(
                "Cannot terminate agent {} while busy with task {:?}",
                self.id,
                self.current_task_id
            ));
        }

        self.status = AgentStatus::Terminated;
        Ok(())
    }

    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        let total = self.tasks_completed + self.tasks_failed;
        if total == 0 {
            return 0.0;
        }
        self.tasks_completed as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_status_display() {
        assert_eq!(AgentStatus::Idle.to_string(), "idle");
        assert_eq!(AgentStatus::Busy.to_string(), "busy");
        assert_eq!(AgentStatus::Terminated.to_string(), "terminated");
    }

    #[test]
    fn test_agent_status_from_str() {
        assert_eq!("idle".parse::<AgentStatus>().unwrap(), AgentStatus::Idle);
        assert_eq!("BUSY".parse::<AgentStatus>().unwrap(), AgentStatus::Busy);
        assert_eq!(
            "Terminated".parse::<AgentStatus>().unwrap(),
            AgentStatus::Terminated
        );
        assert!("invalid".parse::<AgentStatus>().is_err());
    }

    #[test]
    fn test_agent_new() {
        let agent = Agent::new("test-agent".to_string());

        assert_eq!(agent.agent_type, "test-agent");
        assert_eq!(agent.status, AgentStatus::Idle);
        assert_eq!(agent.current_task_id, None);
        assert_eq!(agent.memory_usage_bytes, 0);
        assert_eq!(agent.cpu_usage_percent, 0.0);
        assert_eq!(agent.tasks_completed, 0);
        assert_eq!(agent.tasks_failed, 0);
        assert!(agent.is_idle());
        assert!(!agent.is_busy());
        assert!(!agent.is_terminated());
    }

    #[test]
    fn test_agent_assign_task() {
        let mut agent = Agent::new("test-agent".to_string());
        let task_id = Uuid::new_v4();

        assert!(agent.assign_task(task_id).is_ok());
        assert_eq!(agent.status, AgentStatus::Busy);
        assert_eq!(agent.current_task_id, Some(task_id));
        assert!(agent.is_busy());
        assert!(!agent.is_idle());
    }

    #[test]
    fn test_agent_assign_task_when_busy() {
        let mut agent = Agent::new("test-agent".to_string());
        let task_id1 = Uuid::new_v4();
        let task_id2 = Uuid::new_v4();

        agent.assign_task(task_id1).unwrap();
        let result = agent.assign_task(task_id2);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already busy"));
        assert_eq!(agent.current_task_id, Some(task_id1));
    }

    #[test]
    fn test_agent_assign_task_when_terminated() {
        let mut agent = Agent::new("test-agent".to_string());
        agent.status = AgentStatus::Terminated;

        let task_id = Uuid::new_v4();
        let result = agent.assign_task(task_id);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("terminated"));
    }

    #[test]
    fn test_agent_complete_task_success() {
        let mut agent = Agent::new("test-agent".to_string());
        let task_id = Uuid::new_v4();

        agent.assign_task(task_id).unwrap();
        assert!(agent.complete_task(true).is_ok());

        assert_eq!(agent.status, AgentStatus::Idle);
        assert_eq!(agent.current_task_id, None);
        assert_eq!(agent.tasks_completed, 1);
        assert_eq!(agent.tasks_failed, 0);
        assert!(agent.is_idle());
    }

    #[test]
    fn test_agent_complete_task_failure() {
        let mut agent = Agent::new("test-agent".to_string());
        let task_id = Uuid::new_v4();

        agent.assign_task(task_id).unwrap();
        assert!(agent.complete_task(false).is_ok());

        assert_eq!(agent.status, AgentStatus::Idle);
        assert_eq!(agent.current_task_id, None);
        assert_eq!(agent.tasks_completed, 0);
        assert_eq!(agent.tasks_failed, 1);
    }

    #[test]
    fn test_agent_complete_task_when_idle() {
        let mut agent = Agent::new("test-agent".to_string());

        let result = agent.complete_task(true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not busy"));
    }

    #[test]
    fn test_agent_update_heartbeat() {
        let mut agent = Agent::new("test-agent".to_string());
        let initial_heartbeat = agent.heartbeat_at;

        // Sleep briefly to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));
        agent.update_heartbeat();

        assert!(agent.heartbeat_at > initial_heartbeat);
    }

    #[test]
    fn test_agent_update_resources() {
        let mut agent = Agent::new("test-agent".to_string());

        agent.update_resources(1024 * 1024, 45.5);

        assert_eq!(agent.memory_usage_bytes, 1024 * 1024);
        assert_eq!(agent.cpu_usage_percent, 45.5);
    }

    #[test]
    fn test_agent_terminate() {
        let mut agent = Agent::new("test-agent".to_string());

        assert!(agent.terminate().is_ok());
        assert_eq!(agent.status, AgentStatus::Terminated);
        assert!(agent.is_terminated());
    }

    #[test]
    fn test_agent_terminate_when_busy() {
        let mut agent = Agent::new("test-agent".to_string());
        let task_id = Uuid::new_v4();
        agent.assign_task(task_id).unwrap();

        let result = agent.terminate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("while busy"));
        assert_eq!(agent.status, AgentStatus::Busy);
    }

    #[test]
    fn test_agent_success_rate_no_tasks() {
        let agent = Agent::new("test-agent".to_string());
        assert_eq!(agent.success_rate(), 0.0);
    }

    #[test]
    fn test_agent_success_rate_all_success() {
        let mut agent = Agent::new("test-agent".to_string());
        agent.tasks_completed = 10;
        agent.tasks_failed = 0;

        assert_eq!(agent.success_rate(), 1.0);
    }

    #[test]
    fn test_agent_success_rate_all_failures() {
        let mut agent = Agent::new("test-agent".to_string());
        agent.tasks_completed = 0;
        agent.tasks_failed = 5;

        assert_eq!(agent.success_rate(), 0.0);
    }

    #[test]
    fn test_agent_success_rate_mixed() {
        let mut agent = Agent::new("test-agent".to_string());
        agent.tasks_completed = 7;
        agent.tasks_failed = 3;

        assert_eq!(agent.success_rate(), 0.7);
    }

    #[test]
    fn test_agent_multiple_tasks_workflow() {
        let mut agent = Agent::new("test-agent".to_string());

        // Complete first task successfully
        let task1 = Uuid::new_v4();
        agent.assign_task(task1).unwrap();
        agent.complete_task(true).unwrap();

        // Complete second task with failure
        let task2 = Uuid::new_v4();
        agent.assign_task(task2).unwrap();
        agent.complete_task(false).unwrap();

        // Complete third task successfully
        let task3 = Uuid::new_v4();
        agent.assign_task(task3).unwrap();
        agent.complete_task(true).unwrap();

        assert_eq!(agent.tasks_completed, 2);
        assert_eq!(agent.tasks_failed, 1);
        assert_eq!(agent.success_rate(), 2.0 / 3.0);
        assert!(agent.is_idle());
    }

    #[test]
    fn test_agent_serialization() {
        let agent = Agent::new("test-agent".to_string());
        let serialized = serde_json::to_string(&agent).unwrap();
        let deserialized: Agent = serde_json::from_str(&serialized).unwrap();

        assert_eq!(agent.id, deserialized.id);
        assert_eq!(agent.agent_type, deserialized.agent_type);
        assert_eq!(agent.status, deserialized.status);
        assert_eq!(agent.current_task_id, deserialized.current_task_id);
        assert_eq!(agent.tasks_completed, deserialized.tasks_completed);
        assert_eq!(agent.tasks_failed, deserialized.tasks_failed);
    }

    #[test]
    fn test_agent_status_serialization() {
        let status = AgentStatus::Busy;
        let serialized = serde_json::to_string(&status).unwrap();
        assert_eq!(serialized, "\"busy\"");
        let deserialized: AgentStatus = serde_json::from_str(&serialized).unwrap();
        assert_eq!(status, deserialized);
    }
}

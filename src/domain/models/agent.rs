use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Agent status enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Busy,
    Terminated,
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Busy => write!(f, "busy"),
            Self::Terminated => write!(f, "terminated"),
        }
    }
}

impl FromStr for AgentStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(Self::Idle),
            "busy" => Ok(Self::Busy),
            "terminated" => Ok(Self::Terminated),
            _ => Err(anyhow::anyhow!("Invalid agent status: {s}")),
        }
    }
}

/// Agent entity representing an agent in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique agent identifier
    pub id: Uuid,

    /// Type of agent (e.g., "general-purpose", "code-reviewer")
    pub agent_type: String,

    /// Current agent status
    pub status: AgentStatus,

    /// ID of the currently executing task (if any)
    pub current_task_id: Option<Uuid>,

    /// Last heartbeat timestamp
    pub heartbeat_at: DateTime<Utc>,

    /// Current memory usage in bytes
    pub memory_usage_bytes: u64,

    /// Current CPU usage percentage (0.0 - 100.0)
    pub cpu_usage_percent: f64,

    /// Agent creation timestamp
    pub created_at: DateTime<Utc>,

    /// Agent termination timestamp (if terminated)
    pub terminated_at: Option<DateTime<Utc>>,
}

impl Agent {
    /// Create a new agent with default values
    pub fn new(id: Uuid, agent_type: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            agent_type,
            status: AgentStatus::Idle,
            current_task_id: None,
            heartbeat_at: now,
            memory_usage_bytes: 0,
            cpu_usage_percent: 0.0,
            created_at: now,
            terminated_at: None,
        }
    }

    /// Check if agent is stale based on heartbeat threshold
    pub fn is_stale(&self, threshold: chrono::Duration) -> bool {
        let elapsed = Utc::now() - self.heartbeat_at;
        elapsed > threshold
    }

    /// Update heartbeat to current time
    pub fn update_heartbeat(&mut self) {
        self.heartbeat_at = Utc::now();
    }

    /// Terminate the agent
    pub fn terminate(&mut self) {
        self.status = AgentStatus::Terminated;
        self.terminated_at = Some(Utc::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_status_serialization() {
        assert_eq!(AgentStatus::Idle.to_string(), "idle");
        assert_eq!(AgentStatus::Busy.to_string(), "busy");
        assert_eq!(AgentStatus::Terminated.to_string(), "terminated");
    }

    #[test]
    fn test_agent_status_from_str() {
        assert_eq!("idle".parse::<AgentStatus>().unwrap(), AgentStatus::Idle);
        assert_eq!("IDLE".parse::<AgentStatus>().unwrap(), AgentStatus::Idle);
        assert_eq!("busy".parse::<AgentStatus>().unwrap(), AgentStatus::Busy);
        assert_eq!(
            "terminated".parse::<AgentStatus>().unwrap(),
            AgentStatus::Terminated
        );
        assert!("invalid".parse::<AgentStatus>().is_err());
    }

    #[test]
    fn test_agent_new() {
        let id = Uuid::new_v4();
        let agent = Agent::new(id, "test-agent".to_string());

        assert_eq!(agent.id, id);
        assert_eq!(agent.agent_type, "test-agent");
        assert_eq!(agent.status, AgentStatus::Idle);
        assert!(agent.current_task_id.is_none());
        assert_eq!(agent.memory_usage_bytes, 0);
        assert_eq!(agent.cpu_usage_percent, 0.0);
        assert!(agent.terminated_at.is_none());
    }

    #[test]
    fn test_agent_is_stale() {
        let mut agent = Agent::new(Uuid::new_v4(), "test".to_string());

        // Not stale immediately
        assert!(!agent.is_stale(chrono::Duration::seconds(60)));

        // Make it stale
        agent.heartbeat_at = Utc::now() - chrono::Duration::seconds(120);
        assert!(agent.is_stale(chrono::Duration::seconds(60)));
    }

    #[test]
    fn test_agent_terminate() {
        let mut agent = Agent::new(Uuid::new_v4(), "test".to_string());

        assert_eq!(agent.status, AgentStatus::Idle);
        assert!(agent.terminated_at.is_none());

        agent.terminate();

        assert_eq!(agent.status, AgentStatus::Terminated);
        assert!(agent.terminated_at.is_some());
    }
}

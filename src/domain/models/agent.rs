//! Agent domain model.
//!
//! Agents are autonomous entities that execute tasks.
//! They have templates defining their capabilities and behavior.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Agent tier classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTier {
    /// High-level planning and decomposition
    Architect,
    /// Domain-specific expertise
    Specialist,
    /// Task execution
    Worker,
}

impl Default for AgentTier {
    fn default() -> Self {
        Self::Worker
    }
}

impl AgentTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Architect => "architect",
            Self::Specialist => "specialist",
            Self::Worker => "worker",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "architect" => Some(Self::Architect),
            "specialist" => Some(Self::Specialist),
            "worker" => Some(Self::Worker),
            _ => None,
        }
    }

    /// Maximum concurrent instances for this tier.
    pub fn max_instances(&self) -> u32 {
        match self {
            Self::Architect => 2,
            Self::Specialist => 5,
            Self::Worker => 20,
        }
    }

    /// Default maximum turns for agents of this tier.
    pub fn max_turns(&self) -> u32 {
        match self {
            Self::Architect => 50,
            Self::Specialist => 35,
            Self::Worker => 25,
        }
    }
}

/// Status of an agent template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Available for use
    Active,
    /// Temporarily disabled
    Disabled,
    /// Deprecated, not for new tasks
    Deprecated,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Deprecated => "deprecated",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            "deprecated" => Some(Self::Deprecated),
            _ => None,
        }
    }
}

/// Tool capability for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCapability {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Whether required or optional
    pub required: bool,
}

impl ToolCapability {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            required: false,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// Constraint on agent behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConstraint {
    /// Constraint name
    pub name: String,
    /// Constraint description
    pub description: String,
    /// Whether enforced or advisory
    pub enforced: bool,
}

impl AgentConstraint {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            enforced: true,
        }
    }

    pub fn advisory(mut self) -> Self {
        self.enforced = false;
        self
    }
}

/// A2A Agent Card (for agent-to-agent communication).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCard {
    /// Capabilities this agent provides
    pub capabilities: Vec<String>,
    /// Input types this agent accepts
    pub accepts: Vec<String>,
    /// Output types this agent produces
    pub produces: Vec<String>,
    /// Agents this agent can hand off to
    pub handoff_targets: Vec<String>,
    /// Custom metadata
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// Agent template defining behavior and capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    /// Unique identifier
    pub id: Uuid,
    /// Agent name/type (e.g., "overmind", "code-writer", "test-runner")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Agent tier
    pub tier: AgentTier,
    /// Template version
    pub version: u32,
    /// System prompt
    pub system_prompt: String,
    /// Available tools
    pub tools: Vec<ToolCapability>,
    /// Behavioral constraints
    pub constraints: Vec<AgentConstraint>,
    /// A2A agent card
    pub agent_card: AgentCard,
    /// Maximum turns per task
    pub max_turns: u32,
    /// Whether this agent is read-only (produces findings via memory, not code commits).
    /// When true, commit verification is disabled and memory verification is enabled.
    pub read_only: bool,
    /// Status
    pub status: AgentStatus,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When last updated
    pub updated_at: DateTime<Utc>,
}

impl AgentTemplate {
    /// Create a new agent template.
    pub fn new(name: impl Into<String>, tier: AgentTier) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: String::new(),
            tier,
            version: 1,
            system_prompt: String::new(),
            tools: Vec::new(),
            constraints: Vec::new(),
            agent_card: AgentCard::default(),
            max_turns: 25,
            read_only: false,
            status: AgentStatus::Active,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set system prompt.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Add a tool capability.
    pub fn with_tool(mut self, tool: ToolCapability) -> Self {
        self.tools.push(tool);
        self
    }

    /// Add a constraint.
    pub fn with_constraint(mut self, constraint: AgentConstraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Set max turns.
    pub fn with_max_turns(mut self, turns: u32) -> Self {
        self.max_turns = turns;
        self
    }

    /// Set read-only mode.
    /// Read-only agents produce findings via memory rather than code commits.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Add handoff target.
    pub fn with_handoff_target(mut self, target: impl Into<String>) -> Self {
        self.agent_card.handoff_targets.push(target.into());
        self
    }

    /// Add capability to agent card.
    pub fn with_capability(mut self, cap: impl Into<String>) -> Self {
        self.agent_card.capabilities.push(cap.into());
        self
    }

    /// Increment version.
    pub fn bump_version(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now();
    }

    /// Validate template.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Agent name cannot be empty".to_string());
        }
        if self.system_prompt.is_empty() {
            return Err("System prompt cannot be empty".to_string());
        }
        if self.max_turns == 0 {
            return Err("Max turns must be greater than 0".to_string());
        }
        Ok(())
    }

    /// Check if agent can hand off to another agent.
    pub fn can_handoff_to(&self, target: &str) -> bool {
        self.agent_card.handoff_targets.contains(&target.to_string())
    }

    /// Check if agent has a specific capability.
    pub fn has_capability(&self, cap: &str) -> bool {
        self.agent_card.capabilities.iter().any(|c| c == cap)
    }

    /// Check if agent has a specific tool.
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.iter().any(|t| t.name == tool_name)
    }
}

/// Running agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    /// Instance ID
    pub id: Uuid,
    /// Template this instance is based on
    pub template_id: Uuid,
    /// Template name (denormalized for convenience)
    pub template_name: String,
    /// Current task being executed
    pub current_task_id: Option<Uuid>,
    /// Turn count for current task
    pub turn_count: u32,
    /// Instance status
    pub status: InstanceStatus,
    /// When started
    pub started_at: DateTime<Utc>,
    /// When completed (if done)
    pub completed_at: Option<DateTime<Utc>>,
}

/// Status of an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    /// Idle, waiting for task
    Idle,
    /// Executing a task
    Running,
    /// Waiting for handoff completion
    WaitingHandoff,
    /// Completed
    Completed,
    /// Failed
    Failed,
}

impl InstanceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::WaitingHandoff => "waiting_handoff",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "idle" => Some(Self::Idle),
            "running" => Some(Self::Running),
            "waiting_handoff" => Some(Self::WaitingHandoff),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl AgentInstance {
    /// Create a new instance from a template.
    pub fn from_template(template: &AgentTemplate) -> Self {
        Self {
            id: Uuid::new_v4(),
            template_id: template.id,
            template_name: template.name.clone(),
            current_task_id: None,
            turn_count: 0,
            status: InstanceStatus::Idle,
            started_at: Utc::now(),
            completed_at: None,
        }
    }

    /// Assign a task to this instance.
    pub fn assign_task(&mut self, task_id: Uuid) {
        self.current_task_id = Some(task_id);
        self.turn_count = 0;
        self.status = InstanceStatus::Running;
    }

    /// Record a turn.
    pub fn record_turn(&mut self) {
        self.turn_count += 1;
    }

    /// Check if max turns exceeded.
    pub fn is_over_limit(&self, max_turns: u32) -> bool {
        self.turn_count >= max_turns
    }

    /// Mark as completed.
    pub fn complete(&mut self) {
        self.status = InstanceStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.current_task_id = None;
    }

    /// Mark as failed.
    pub fn fail(&mut self) {
        self.status = InstanceStatus::Failed;
        self.completed_at = Some(Utc::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_template_creation() {
        let template = AgentTemplate::new("test-agent", AgentTier::Worker)
            .with_description("A test agent")
            .with_prompt("You are a test agent.")
            .with_max_turns(10);

        assert_eq!(template.name, "test-agent");
        assert_eq!(template.tier, AgentTier::Worker);
        assert!(!template.read_only);
        assert!(template.validate().is_ok());
    }

    #[test]
    fn test_agent_template_read_only() {
        let template = AgentTemplate::new("researcher", AgentTier::Worker)
            .with_description("A read-only research agent")
            .with_prompt("You are a researcher.")
            .with_read_only(true);

        assert!(template.read_only);

        // Verify it can be toggled back
        let template = template.with_read_only(false);
        assert!(!template.read_only);
    }

    #[test]
    fn test_agent_validation() {
        let template = AgentTemplate::new("", AgentTier::Worker);
        assert!(template.validate().is_err());

        let template = AgentTemplate::new("valid", AgentTier::Worker)
            .with_prompt(""); // Empty prompt
        assert!(template.validate().is_err());
    }

    #[test]
    fn test_agent_tools_and_capabilities() {
        let template = AgentTemplate::new("coder", AgentTier::Specialist)
            .with_prompt("You are a coder.")
            .with_tool(ToolCapability::new("read", "Read files").required())
            .with_tool(ToolCapability::new("write", "Write files"))
            .with_capability("code-generation")
            .with_handoff_target("reviewer");

        assert!(template.has_tool("read"));
        assert!(template.has_tool("write"));
        assert!(!template.has_tool("delete"));
        assert!(template.has_capability("code-generation"));
        assert!(template.can_handoff_to("reviewer"));
    }

    #[test]
    fn test_agent_instance() {
        let template = AgentTemplate::new("test", AgentTier::Worker)
            .with_prompt("Test")
            .with_max_turns(5);

        let mut instance = AgentInstance::from_template(&template);
        assert_eq!(instance.status, InstanceStatus::Idle);

        let task_id = Uuid::new_v4();
        instance.assign_task(task_id);
        assert_eq!(instance.status, InstanceStatus::Running);
        assert_eq!(instance.current_task_id, Some(task_id));

        for _ in 0..6 {
            instance.record_turn();
        }
        assert!(instance.is_over_limit(5));

        instance.complete();
        assert_eq!(instance.status, InstanceStatus::Completed);
        assert!(instance.completed_at.is_some());
    }

    #[test]
    fn test_tier_properties() {
        assert_eq!(AgentTier::Architect.max_instances(), 2);
        assert_eq!(AgentTier::Specialist.max_instances(), 5);
        assert_eq!(AgentTier::Worker.max_instances(), 20);
    }
}

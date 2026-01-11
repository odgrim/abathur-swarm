//! Substrate domain models.
//!
//! Substrates are LLM backends that agents run on. The primary substrate
//! is Claude Code CLI, but the architecture supports multiple backends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of LLM substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubstrateType {
    /// Claude Code CLI
    ClaudeCode,
    /// Direct Anthropic API
    AnthropicApi,
    /// Mock substrate for testing
    Mock,
}

impl SubstrateType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::AnthropicApi => "anthropic_api",
            Self::Mock => "mock",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude_code" | "claudecode" => Some(Self::ClaudeCode),
            "anthropic_api" | "anthropicapi" | "api" => Some(Self::AnthropicApi),
            "mock" | "test" => Some(Self::Mock),
            _ => None,
        }
    }
}

impl Default for SubstrateType {
    fn default() -> Self {
        Self::ClaudeCode
    }
}

/// Configuration for a substrate invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateConfig {
    /// Substrate type
    pub substrate_type: SubstrateType,
    /// Maximum turns for this invocation
    pub max_turns: u32,
    /// Working directory for the invocation
    pub working_dir: Option<String>,
    /// MCP servers to connect to
    pub mcp_servers: Vec<String>,
    /// Environment variables to set
    pub env_vars: Vec<(String, String)>,
    /// Model to use (if applicable)
    pub model: Option<String>,
    /// Temperature setting
    pub temperature: Option<f32>,
    /// Whether to allow tool use
    pub allow_tools: bool,
    /// Allowed file patterns (glob)
    pub allowed_files: Vec<String>,
    /// Denied file patterns (glob)
    pub denied_files: Vec<String>,
}

impl Default for SubstrateConfig {
    fn default() -> Self {
        Self {
            substrate_type: SubstrateType::ClaudeCode,
            max_turns: 25,
            working_dir: None,
            mcp_servers: vec![],
            env_vars: vec![],
            model: None,
            temperature: None,
            allow_tools: true,
            allowed_files: vec![],
            denied_files: vec![],
        }
    }
}

impl SubstrateConfig {
    pub fn claude_code() -> Self {
        Self {
            substrate_type: SubstrateType::ClaudeCode,
            ..Default::default()
        }
    }

    pub fn mock() -> Self {
        Self {
            substrate_type: SubstrateType::Mock,
            max_turns: 1,
            ..Default::default()
        }
    }

    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    pub fn with_max_turns(mut self, turns: u32) -> Self {
        self.max_turns = turns;
        self
    }

    pub fn with_mcp_server(mut self, server: impl Into<String>) -> Self {
        self.mcp_servers.push(server.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), value.into()));
        self
    }
}

/// Status of a substrate session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Session starting
    Starting,
    /// Session active and processing
    Active,
    /// Waiting for user input (shouldn't happen in swarm mode)
    WaitingInput,
    /// Session completed successfully
    Completed,
    /// Session failed
    Failed,
    /// Session timed out
    TimedOut,
    /// Session was terminated
    Terminated,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Active => "active",
            Self::WaitingInput => "waiting_input",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Terminated => "terminated",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::TimedOut | Self::Terminated)
    }
}

/// A substrate session representing an LLM invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateSession {
    /// Unique session ID
    pub id: Uuid,
    /// Associated task ID
    pub task_id: Uuid,
    /// Agent template being used
    pub agent_template: String,
    /// Substrate configuration
    pub config: SubstrateConfig,
    /// Current status
    pub status: SessionStatus,
    /// Number of turns executed
    pub turns_completed: u32,
    /// Input tokens used
    pub input_tokens: u64,
    /// Output tokens used
    pub output_tokens: u64,
    /// Cache read tokens
    pub cache_read_tokens: u64,
    /// Cache write tokens
    pub cache_write_tokens: u64,
    /// Cost in cents (if tracked)
    pub cost_cents: Option<f64>,
    /// Final output/result
    pub result: Option<String>,
    /// Error message if failed
    pub error: Option<String>,
    /// Process ID (if subprocess)
    pub process_id: Option<u32>,
    /// When session started
    pub started_at: DateTime<Utc>,
    /// When session ended
    pub ended_at: Option<DateTime<Utc>>,
}

impl SubstrateSession {
    pub fn new(task_id: Uuid, agent_template: impl Into<String>, config: SubstrateConfig) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id,
            agent_template: agent_template.into(),
            config,
            status: SessionStatus::Starting,
            turns_completed: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_cents: None,
            result: None,
            error: None,
            process_id: None,
            started_at: Utc::now(),
            ended_at: None,
        }
    }

    pub fn start(&mut self, process_id: Option<u32>) {
        self.status = SessionStatus::Active;
        self.process_id = process_id;
    }

    pub fn record_turn(&mut self, input_tokens: u64, output_tokens: u64) {
        self.turns_completed += 1;
        self.input_tokens += input_tokens;
        self.output_tokens += output_tokens;
    }

    pub fn complete(&mut self, result: impl Into<String>) {
        self.status = SessionStatus::Completed;
        self.result = Some(result.into());
        self.ended_at = Some(Utc::now());
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.status = SessionStatus::Failed;
        self.error = Some(error.into());
        self.ended_at = Some(Utc::now());
    }

    pub fn timeout(&mut self) {
        self.status = SessionStatus::TimedOut;
        self.error = Some("Session timed out".to_string());
        self.ended_at = Some(Utc::now());
    }

    pub fn terminate(&mut self) {
        self.status = SessionStatus::Terminated;
        self.ended_at = Some(Utc::now());
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    pub fn duration_seconds(&self) -> Option<i64> {
        self.ended_at.map(|end| (end - self.started_at).num_seconds())
    }

    pub fn is_over_turn_limit(&self) -> bool {
        self.turns_completed >= self.config.max_turns
    }
}

/// Output from a streaming substrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubstrateOutput {
    /// Text output from the assistant
    AssistantText { content: String },
    /// Tool call started
    ToolStart { name: String, id: String },
    /// Tool call result
    ToolResult { id: String, result: String, is_error: bool },
    /// Turn completed
    TurnComplete { turn_number: u32, input_tokens: u64, output_tokens: u64 },
    /// Session complete
    SessionComplete { result: String },
    /// Error occurred
    Error { message: String },
    /// Status update
    Status { message: String },
}

/// Request to invoke a substrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateRequest {
    /// Task ID
    pub task_id: Uuid,
    /// System prompt
    pub system_prompt: String,
    /// Initial user prompt
    pub user_prompt: String,
    /// Agent template name
    pub agent_template: String,
    /// Configuration
    pub config: SubstrateConfig,
    /// Continue from session ID (for resume)
    pub resume_session: Option<Uuid>,
}

impl SubstrateRequest {
    pub fn new(
        task_id: Uuid,
        agent_template: impl Into<String>,
        system_prompt: impl Into<String>,
        user_prompt: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            system_prompt: system_prompt.into(),
            user_prompt: user_prompt.into(),
            agent_template: agent_template.into(),
            config: SubstrateConfig::default(),
            resume_session: None,
        }
    }

    pub fn with_config(mut self, config: SubstrateConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_resume(mut self, session_id: Uuid) -> Self {
        self.resume_session = Some(session_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substrate_config_builder() {
        let config = SubstrateConfig::claude_code()
            .with_working_dir("/tmp/test")
            .with_max_turns(10)
            .with_mcp_server("memory-server")
            .with_env("TASK_ID", "abc123");

        assert_eq!(config.substrate_type, SubstrateType::ClaudeCode);
        assert_eq!(config.working_dir, Some("/tmp/test".to_string()));
        assert_eq!(config.max_turns, 10);
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.env_vars.len(), 1);
    }

    #[test]
    fn test_session_lifecycle() {
        let task_id = Uuid::new_v4();
        let config = SubstrateConfig::default();
        let mut session = SubstrateSession::new(task_id, "test-agent", config);

        assert_eq!(session.status, SessionStatus::Starting);
        assert_eq!(session.turns_completed, 0);

        session.start(Some(12345));
        assert_eq!(session.status, SessionStatus::Active);
        assert_eq!(session.process_id, Some(12345));

        session.record_turn(1000, 500);
        assert_eq!(session.turns_completed, 1);
        assert_eq!(session.total_tokens(), 1500);

        session.complete("Task completed successfully");
        assert_eq!(session.status, SessionStatus::Completed);
        assert!(session.result.is_some());
        assert!(session.ended_at.is_some());
    }

    #[test]
    fn test_turn_limit() {
        let task_id = Uuid::new_v4();
        let config = SubstrateConfig::default().with_max_turns(3);
        let mut session = SubstrateSession::new(task_id, "test", config);

        session.record_turn(100, 50);
        assert!(!session.is_over_turn_limit());

        session.record_turn(100, 50);
        session.record_turn(100, 50);
        assert!(session.is_over_turn_limit());
    }
}

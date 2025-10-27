///! LLM Substrate Port
///!
///! This module defines the abstraction layer for different LLM backends (substrates).
///! Substrates can be:
///! - Claude Code CLI (default, no API key needed)
///! - Anthropic API (direct API access)
///! - OpenAI API
///! - Local models (Ollama, etc.)
///!
///! This allows mixing and matching agents with the best model/platform for their purpose.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Request to execute a task via an LLM substrate
#[derive(Debug, Clone)]
pub struct SubstrateRequest {
    /// Unique task identifier
    pub task_id: Uuid,

    /// Agent type (e.g., "requirements-gatherer", "rust-specialist")
    pub agent_type: String,

    /// Task prompt/description
    pub prompt: String,

    /// Optional context data (previous outputs, dependencies, etc.)
    pub context: Option<serde_json::Value>,

    /// Execution parameters (model, temperature, etc.)
    pub parameters: ExecutionParameters,
}

/// Parameters for controlling LLM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionParameters {
    /// Maximum tokens to generate (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Sampling temperature (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// Additional substrate-specific parameters
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for ExecutionParameters {
    fn default() -> Self {
        Self {
            max_tokens: Some(4096),
            temperature: Some(0.7),
            timeout_secs: Some(300), // 5 minutes default
            extra: HashMap::new(),
        }
    }
}

/// Response from LLM substrate after task execution
#[derive(Debug, Clone)]
pub struct SubstrateResponse {
    /// Task identifier
    pub task_id: Uuid,

    /// Generated content/output
    pub content: String,

    /// Reason execution stopped
    pub stop_reason: StopReason,

    /// Token usage statistics (if available)
    pub usage: Option<TokenUsage>,

    /// Additional metadata from the substrate
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Reason why execution stopped
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Natural end of generation
    EndTurn,

    /// Hit maximum token limit
    MaxTokens,

    /// Execution timeout
    Timeout,

    /// User or system interrupt
    Cancelled,

    /// Error occurred
    Error,

    /// Other/unknown reason
    Other(String),
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Health status of a substrate
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Substrate is healthy and available
    Healthy,

    /// Substrate is degraded but usable
    Degraded,

    /// Substrate is unavailable
    Unavailable,
}

/// Error types for substrate operations
#[derive(Debug, thiserror::Error)]
pub enum SubstrateError {
    #[error("Substrate not configured: {0}")]
    NotConfigured(String),

    #[error("Substrate unavailable: {0}")]
    Unavailable(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Execution timeout after {0}s")]
    Timeout(u64),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

/// Port trait for LLM substrate implementations
///
/// This trait abstracts away the specifics of different LLM backends,
/// allowing the orchestrator to work with any substrate that implements
/// this interface.
///
/// # Implementations
///
/// - **ClaudeCodeSubstrate**: Shells out to Claude Code CLI (default, no API key)
/// - **AnthropicApiSubstrate**: Uses Anthropic API directly (requires API key)
/// - Future: OpenAI, local models, etc.
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` for concurrent use across tokio tasks.
#[async_trait]
pub trait LlmSubstrate: Send + Sync {
    /// Get the unique identifier for this substrate type
    ///
    /// Examples: "claude-code", "anthropic-api", "openai", "ollama"
    fn substrate_id(&self) -> &str;

    /// Get human-readable name for this substrate
    fn substrate_name(&self) -> &str;

    /// Execute a task via this substrate
    ///
    /// # Arguments
    /// * `request` - The task execution request
    ///
    /// # Returns
    /// * `Ok(SubstrateResponse)` - Successful execution with generated content
    /// * `Err(SubstrateError)` - Execution failed
    ///
    /// # Errors
    /// - `SubstrateError::Unavailable` - Substrate cannot be reached
    /// - `SubstrateError::RateLimitExceeded` - Rate limit hit (caller should retry)
    /// - `SubstrateError::Timeout` - Execution exceeded timeout
    /// - `SubstrateError::ExecutionFailed` - Execution error
    async fn execute(&self, request: SubstrateRequest) -> Result<SubstrateResponse, SubstrateError>;

    /// Check health of this substrate
    ///
    /// Verifies that the substrate is available and properly configured.
    /// Useful for startup validation and monitoring.
    ///
    /// # Returns
    /// * `Ok(HealthStatus)` - Current health status
    /// * `Err(SubstrateError)` - Cannot determine health
    async fn health_check(&self) -> Result<HealthStatus, SubstrateError>;

    /// Check if this substrate can handle a specific agent type
    ///
    /// Some substrates may be better suited for certain agent types.
    /// For example, coding-focused agents might prefer Claude Code.
    ///
    /// # Arguments
    /// * `agent_type` - The agent type to check
    ///
    /// # Returns
    /// * `true` - This substrate can handle the agent type
    /// * `false` - This substrate should not be used for this agent type
    ///
    /// Default implementation returns `true` for all agent types.
    fn can_handle_agent_type(&self, _agent_type: &str) -> bool {
        true
    }
}

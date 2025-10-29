use serde::{Deserialize, Serialize};

/// Main configuration structure for Abathur
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    /// Maximum number of concurrent agents (1-100)
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,

    /// Database configuration
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Rate limiting configuration
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Retry policy configuration
    #[serde(default)]
    pub retry: RetryConfig,

    /// MCP server configurations
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,

    /// LLM substrate configurations
    #[serde(default)]
    pub substrates: SubstratesConfig,
}

const fn default_max_agents() -> usize {
    10
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            database: DatabaseConfig::default(),
            logging: LoggingConfig::default(),
            rate_limit: RateLimitConfig::default(),
            retry: RetryConfig::default(),
            mcp_servers: vec![],
            substrates: SubstratesConfig::default(),
        }
    }
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DatabaseConfig {
    /// Path to `SQLite` database file
    #[serde(default = "default_database_path")]
    pub path: String,

    /// Maximum number of database connections in pool
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

fn default_database_path() -> String {
    ".abathur/abathur.db".to_string()
}

const fn default_max_connections() -> u32 {
    10
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_database_path(),
            max_connections: default_max_connections(),
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LoggingConfig {
    /// Log level: trace, debug, info, warn, error
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format: json or pretty
    #[serde(default = "default_log_format")]
    pub format: String,

    /// Number of days to retain logs
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "json".to_string()
}

const fn default_retention_days() -> u32 {
    30
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
            retention_days: default_retention_days(),
        }
    }
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RateLimitConfig {
    /// Requests per second allowed
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: f64,

    /// Burst size for token bucket
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
}

const fn default_requests_per_second() -> f64 {
    10.0
}

const fn default_burst_size() -> u32 {
    20
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: default_requests_per_second(),
            burst_size: default_burst_size(),
        }
    }
}

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Initial backoff delay in milliseconds
    #[serde(default = "default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,

    /// Maximum backoff delay in milliseconds
    #[serde(default = "default_max_backoff_ms")]
    pub max_backoff_ms: u64,
}

const fn default_max_retries() -> u32 {
    3
}

const fn default_initial_backoff_ms() -> u64 {
    10000
}

const fn default_max_backoff_ms() -> u64 {
    300_000
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_backoff_ms: default_initial_backoff_ms(),
            max_backoff_ms: default_max_backoff_ms(),
        }
    }
}

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct McpServerConfig {
    /// Server name
    pub name: String,

    /// Command to execute
    pub command: String,

    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// LLM Substrates configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubstratesConfig {
    /// Default substrate to use if agent type has no mapping
    #[serde(default = "default_substrate")]
    pub default_substrate: String,

    /// Enabled substrate types
    #[serde(default = "default_enabled_substrates")]
    pub enabled: Vec<String>,

    /// Claude Code substrate configuration
    #[serde(default)]
    pub claude_code: ClaudeCodeSubstrateConfig,

    /// Anthropic API substrate configuration
    #[serde(default)]
    pub anthropic_api: AnthropicApiSubstrateConfig,

    /// Agent type to substrate mappings
    /// Maps agent type patterns to specific substrates
    #[serde(default)]
    pub agent_mappings: std::collections::HashMap<String, String>,
}

fn default_substrate() -> String {
    "claude-code".to_string()
}

fn default_enabled_substrates() -> Vec<String> {
    vec!["claude-code".to_string()]
}

impl Default for SubstratesConfig {
    fn default() -> Self {
        Self {
            default_substrate: default_substrate(),
            enabled: default_enabled_substrates(),
            claude_code: ClaudeCodeSubstrateConfig::default(),
            anthropic_api: AnthropicApiSubstrateConfig::default(),
            agent_mappings: std::collections::HashMap::new(),
        }
    }
}

/// Claude Code substrate configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClaudeCodeSubstrateConfig {
    /// Path to claude CLI executable
    #[serde(default = "default_claude_path")]
    pub claude_path: String,

    /// Working directory for claude execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,

    /// Default timeout in seconds
    #[serde(default = "default_claude_timeout")]
    pub timeout_secs: u64,
}

fn default_claude_path() -> String {
    "claude".to_string()
}

fn default_claude_timeout() -> u64 {
    300
}

impl Default for ClaudeCodeSubstrateConfig {
    fn default() -> Self {
        Self {
            claude_path: default_claude_path(),
            working_dir: None,
            timeout_secs: default_claude_timeout(),
        }
    }
}

/// Anthropic API substrate configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnthropicApiSubstrateConfig {
    /// Enable Anthropic API substrate
    #[serde(default)]
    pub enabled: bool,

    /// API key (can also be set via ANTHROPIC_API_KEY env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Model to use
    #[serde(default = "default_anthropic_model")]
    pub model: String,

    /// Base URL for API (for testing/proxies)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

fn default_anthropic_model() -> String {
    "claude-sonnet-4-5-20250929".to_string()
}

impl Default for AnthropicApiSubstrateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            model: default_anthropic_model(),
            base_url: None,
        }
    }
}

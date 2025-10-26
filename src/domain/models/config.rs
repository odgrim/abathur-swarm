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

    /// Resource limits per agent
    #[serde(default)]
    pub resource_limits: ResourceLimitsConfig,
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
            resource_limits: ResourceLimitsConfig::default(),
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

/// Resource limits configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResourceLimitsConfig {
    /// Memory limit per agent in MB
    #[serde(default = "default_per_agent_memory_mb")]
    pub per_agent_memory_mb: u64,

    /// Total memory limit in MB
    #[serde(default = "default_total_memory_mb")]
    pub total_memory_mb: u64,
}

const fn default_per_agent_memory_mb() -> u64 {
    512
}

const fn default_total_memory_mb() -> u64 {
    4096
}

impl Default for ResourceLimitsConfig {
    fn default() -> Self {
        Self {
            per_agent_memory_mb: default_per_agent_memory_mb(),
            total_memory_mb: default_total_memory_mb(),
        }
    }
}

use thiserror::Error;
use uuid::Uuid;

use super::models::task::TaskStatus;

/// Domain-level errors for task operations
#[derive(Error, Debug)]
pub enum TaskError {
    #[error("Invalid summary: {0}")]
    InvalidSummary(String),

    #[error("Invalid priority: {0} (must be 0-10)")]
    InvalidPriority(u8),

    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition { from: TaskStatus, to: TaskStatus },

    #[error("Circular dependency detected: {0:?}")]
    CircularDependency(Vec<Uuid>),

    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    #[error("Task has unmet dependencies: {0:?}")]
    UnmetDependencies(Vec<Uuid>),

    #[error("Task cannot be retried: retry count {retry_count} exceeds max retries {max_retries}")]
    MaxRetriesExceeded { retry_count: u32, max_retries: u32 },

    #[error("Task execution timeout exceeded: {timeout_seconds}s")]
    TimeoutExceeded { timeout_seconds: u32 },

    #[error("Task is in terminal state: {0:?}")]
    TaskInTerminalState(TaskStatus),
}

/// Domain-level errors
#[derive(Error, Debug)]
pub enum DomainError {
    #[error("Task error: {0}")]
    Task(#[from] TaskError),
}

/// Database-level errors
#[derive(Error, Debug)]
pub enum DatabaseError {
    /// Database connection could not be established
    #[error("Database connection failed: {0}")]
    ConnectionFailed(String),

    /// A database query failed
    #[error("Query failed: {0}")]
    QueryFailed(String),

    /// Database migration failed
    #[error("Migration failed: {0}")]
    MigrationFailed(String),

    /// Database transaction failed
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),

    /// Database constraint violation
    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),

    /// Row not found in query result
    #[error("Row not found")]
    RowNotFound,

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl DatabaseError {
    /// Returns true if this error is transient and could succeed on retry
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::ConnectionFailed(_) | Self::TransactionFailed(_))
    }
}

/// Errors related to Claude API interactions
#[derive(Error, Debug)]
pub enum ClaudeApiError {
    /// API request failed due to network or HTTP error
    #[error("API request failed: {0}")]
    RequestFailed(String),

    /// Rate limit has been exceeded
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// Authentication failed (invalid API key)
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// API response was invalid or could not be parsed
    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// Request timed out after specified duration
    #[error("Timeout after {0} seconds")]
    Timeout(u64),

    /// API returned an error status code
    #[error("API error {status}: {message}")]
    ApiError { status: u16, message: String },

    /// Token limit exceeded for the request
    #[error("Token limit exceeded: requested {requested}, limit {limit}")]
    TokenLimitExceeded { requested: usize, limit: usize },
}

impl ClaudeApiError {
    /// Returns true if this error is transient and should be retried
    pub const fn is_transient(&self) -> bool {
        match self {
            Self::RateLimitExceeded | Self::Timeout(_) | Self::RequestFailed(_) => true,
            Self::ApiError { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// Returns true if this error is permanent and should not be retried
    pub const fn is_permanent(&self) -> bool {
        match self {
            Self::AuthenticationFailed(_) | Self::TokenLimitExceeded { .. } => true,
            Self::ApiError { status, .. } => *status == 400 || *status == 401,
            _ => false,
        }
    }
}

/// Errors related to MCP (Model Context Protocol) operations
#[derive(Error, Debug)]
pub enum McpError {
    /// MCP server with the given name was not found
    #[error("MCP server not found: {0}")]
    ServerNotFound(String),

    /// MCP tool call failed
    #[error("MCP tool call failed: {0}")]
    ToolCallFailed(String),

    /// MCP server process crashed
    #[error("MCP server crashed")]
    ServerCrashed,

    /// MCP protocol error
    #[error("MCP protocol error: {0}")]
    ProtocolError(String),

    /// Failed to spawn MCP server process
    #[error("Failed to spawn MCP server: {0}")]
    SpawnFailed(String),

    /// MCP server health check failed
    #[error("MCP server health check failed for '{0}'")]
    HealthCheckFailed(String),

    /// MCP tool not found on server
    #[error("MCP tool '{tool}' not found on server '{server}'")]
    ToolNotFound { server: String, tool: String },
}

impl McpError {
    /// Returns true if this error is transient and could succeed on retry
    pub const fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ServerCrashed | Self::HealthCheckFailed(_) | Self::ToolCallFailed(_)
        )
    }
}

/// Errors related to configuration loading and validation
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Configuration file was not found at the specified path
    #[error("Config file not found: {0}")]
    FileNotFound(String),

    /// Invalid YAML syntax in configuration file
    #[error("Invalid YAML: {0}")]
    InvalidYaml(String),

    /// Required configuration field is missing
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// Configuration field has an invalid value
    #[error("Invalid value for {field}: {value}")]
    InvalidValue { field: String, value: String },

    /// I/O error while reading configuration file
    #[error("I/O error reading config: {0}")]
    IoError(String),

    /// Environment variable error
    #[error("Environment variable error: {0}")]
    EnvVarError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_error_not_found_display() {
        let task_id = Uuid::new_v4();
        let err = TaskError::TaskNotFound(task_id);
        assert_eq!(err.to_string(), format!("Task not found: {task_id}"));
    }

    #[test]
    fn test_task_error_invalid_priority_display() {
        let err = TaskError::InvalidPriority(15);
        assert_eq!(err.to_string(), "Invalid priority: 15 (must be 0-10)");
    }

    #[test]
    fn test_task_error_circular_dependency_display() {
        let task_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let err = TaskError::CircularDependency(task_ids.clone());
        assert!(err.to_string().contains("Circular dependency detected"));
    }

    #[test]
    fn test_database_error_display() {
        let err = DatabaseError::ConnectionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "Database connection failed: timeout");

        let err = DatabaseError::QueryFailed("syntax error".to_string());
        assert_eq!(err.to_string(), "Query failed: syntax error");
    }

    #[test]
    fn test_database_error_is_transient() {
        assert!(DatabaseError::ConnectionFailed("timeout".to_string()).is_transient());
        assert!(DatabaseError::TransactionFailed("conflict".to_string()).is_transient());
        assert!(!DatabaseError::ConstraintViolation("unique".to_string()).is_transient());
    }

    #[test]
    fn test_claude_api_error_display() {
        let err = ClaudeApiError::RateLimitExceeded;
        assert_eq!(err.to_string(), "Rate limit exceeded");

        let err = ClaudeApiError::Timeout(30);
        assert_eq!(err.to_string(), "Timeout after 30 seconds");

        let err = ClaudeApiError::ApiError {
            status: 500,
            message: "Internal server error".to_string(),
        };
        assert_eq!(err.to_string(), "API error 500: Internal server error");
    }

    #[test]
    fn test_claude_api_error_is_transient() {
        assert!(ClaudeApiError::RateLimitExceeded.is_transient());
        assert!(ClaudeApiError::Timeout(30).is_transient());
        assert!(
            ClaudeApiError::ApiError {
                status: 500,
                message: "error".to_string()
            }
            .is_transient()
        );
        assert!(!ClaudeApiError::AuthenticationFailed("invalid key".to_string()).is_transient());
    }

    #[test]
    fn test_claude_api_error_is_permanent() {
        assert!(ClaudeApiError::AuthenticationFailed("invalid key".to_string()).is_permanent());
        assert!(
            ClaudeApiError::TokenLimitExceeded {
                requested: 10000,
                limit: 8000
            }
            .is_permanent()
        );
        assert!(!ClaudeApiError::RateLimitExceeded.is_permanent());
    }

    #[test]
    fn test_mcp_error_display() {
        let err = McpError::ServerNotFound("test-server".to_string());
        assert_eq!(err.to_string(), "MCP server not found: test-server");

        let err = McpError::ToolNotFound {
            server: "test-server".to_string(),
            tool: "test-tool".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "MCP tool 'test-tool' not found on server 'test-server'"
        );
    }

    #[test]
    fn test_mcp_error_is_transient() {
        assert!(McpError::ServerCrashed.is_transient());
        assert!(McpError::HealthCheckFailed("server".to_string()).is_transient());
        assert!(!McpError::ServerNotFound("server".to_string()).is_transient());
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::FileNotFound("/path/to/config.yaml".to_string());
        assert_eq!(
            err.to_string(),
            "Config file not found: /path/to/config.yaml"
        );

        let err = ConfigError::InvalidValue {
            field: "priority".to_string(),
            value: "invalid".to_string(),
        };
        assert_eq!(err.to_string(), "Invalid value for priority: invalid");
    }
}

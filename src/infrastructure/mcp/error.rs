use thiserror::Error;

/// Errors that can occur during MCP operations
#[derive(Error, Debug)]
pub enum McpError {
    #[error("Server not found: {0}")]
    ServerNotFound(String),

    #[error("Server already running: {0}")]
    ServerAlreadyRunning(String),

    #[error("Server not running: {0}")]
    ServerNotRunning(String),

    #[error("Health check failed for server: {0}")]
    HealthCheckFailed(String),

    #[error("Health check timeout for server: {0}")]
    HealthCheckTimeout(String),

    #[error("Failed to spawn server process: {source}")]
    ProcessSpawnError {
        #[from]
        source: std::io::Error,
    },

    #[error("Server process terminated unexpectedly: {0}")]
    ProcessTerminated(String),

    #[error("Failed to communicate with server: {0}")]
    CommunicationError(String),

    #[error("JSON-RPC error: {0}")]
    JsonRpcError(String),

    #[error("Server restart failed: {0}")]
    RestartFailed(String),

    #[error("Transport error: {0}")]
    TransportError(String),

    #[error("Lock poisoned")]
    LockPoisoned,
}

impl<T> From<std::sync::PoisonError<T>> for McpError {
    fn from(_: std::sync::PoisonError<T>) -> Self {
        McpError::LockPoisoned
    }
}

/// Result type alias for MCP operations
pub type Result<T> = std::result::Result<T, McpError>;

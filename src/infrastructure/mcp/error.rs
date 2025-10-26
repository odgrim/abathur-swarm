//! Error types for MCP (Model Context Protocol) integration.
//!
//! Provides structured error handling for MCP server lifecycle management,
//! protocol communication, and resource operations.

use thiserror::Error;

/// Errors that can occur during MCP operations.
///
/// This enum covers all error cases for MCP server management, including
/// server lifecycle issues, protocol errors, and resource access failures.
#[derive(Error, Debug)]
pub enum McpError {
    /// MCP server with the given name was not found in configuration.
    #[error("MCP server not found: {0}")]
    ServerNotFound(String),

    /// Failed to start an MCP server process.
    ///
    /// This can occur due to invalid configuration, missing executable,
    /// or permission issues.
    #[error("Failed to start MCP server: {0}")]
    ServerStartFailed(String),

    /// MCP server process crashed unexpectedly.
    ///
    /// The server should be restarted automatically when this occurs.
    #[error("MCP server crashed: {0}")]
    ServerCrashed(String),

    /// MCP server did not respond within the expected timeout period.
    ///
    /// This may indicate a hung process that should be restarted.
    #[error("MCP server timeout")]
    ServerTimeout,

    /// Received an invalid or malformed response from the MCP server.
    ///
    /// This indicates a protocol violation or version mismatch.
    #[error("Invalid MCP response: {0}")]
    InvalidResponse(String),

    /// MCP protocol-level error occurred during communication.
    ///
    /// This covers JSON-RPC errors, unsupported operations, etc.
    #[error("MCP protocol error: {0}")]
    ProtocolError(String),

    /// Requested tool was not found on the MCP server.
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    /// Requested resource was not found on the MCP server.
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    /// I/O error occurred during MCP operations.
    ///
    /// Automatically converted from `std::io::Error`.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    ///
    /// Automatically converted from `serde_json::Error`.
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// An unknown or unexpected error occurred.
    #[error("Unknown MCP error: {0}")]
    Unknown(String),
}

impl McpError {
    /// Determines if the MCP server should be restarted after this error.
    ///
    /// Returns `true` for transient errors that can be recovered from by
    /// restarting the server process, such as crashes, timeouts, or I/O errors.
    ///
    /// Returns `false` for permanent errors like configuration issues or
    /// protocol violations that won't be fixed by a restart.
    ///
    /// # Examples
    ///
    /// ```
    /// use abathur::infrastructure::mcp::error::McpError;
    ///
    /// let crash_error = McpError::ServerCrashed("process exited".to_string());
    /// assert!(crash_error.should_restart());
    ///
    /// let not_found = McpError::ToolNotFound("foo".to_string());
    /// assert!(!not_found.should_restart());
    /// ```
    pub fn should_restart(&self) -> bool {
        matches!(
            self,
            McpError::ServerCrashed(_) | McpError::ServerTimeout | McpError::IoError(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_should_restart_for_crash() {
        let err = McpError::ServerCrashed("unexpected exit".to_string());
        assert!(err.should_restart());
    }

    #[test]
    fn test_should_restart_for_timeout() {
        let err = McpError::ServerTimeout;
        assert!(err.should_restart());
    }

    #[test]
    fn test_should_restart_for_io_error() {
        let io_err = io::Error::new(io::ErrorKind::BrokenPipe, "pipe broken");
        let err = McpError::IoError(io_err);
        assert!(err.should_restart());
    }

    #[test]
    fn test_should_not_restart_for_not_found() {
        let err = McpError::ServerNotFound("test-server".to_string());
        assert!(!err.should_restart());
    }

    #[test]
    fn test_should_not_restart_for_tool_not_found() {
        let err = McpError::ToolNotFound("test-tool".to_string());
        assert!(!err.should_restart());
    }

    #[test]
    fn test_should_not_restart_for_protocol_error() {
        let err = McpError::ProtocolError("invalid method".to_string());
        assert!(!err.should_restart());
    }

    #[test]
    fn test_should_not_restart_for_invalid_response() {
        let err = McpError::InvalidResponse("bad json".to_string());
        assert!(!err.should_restart());
    }

    #[test]
    fn test_error_display_server_not_found() {
        let err = McpError::ServerNotFound("my-server".to_string());
        assert_eq!(err.to_string(), "MCP server not found: my-server");
    }

    #[test]
    fn test_error_display_server_crashed() {
        let err = McpError::ServerCrashed("exit code 1".to_string());
        assert_eq!(err.to_string(), "MCP server crashed: exit code 1");
    }

    #[test]
    fn test_error_display_timeout() {
        let err = McpError::ServerTimeout;
        assert_eq!(err.to_string(), "MCP server timeout");
    }

    #[test]
    fn test_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let mcp_err: McpError = io_err.into();

        assert!(matches!(mcp_err, McpError::IoError(_)));
        assert!(mcp_err.to_string().contains("IO error"));
    }

    #[test]
    fn test_from_json_error() {
        let json_str = "{invalid json}";
        let json_err = serde_json::from_str::<serde_json::Value>(json_str).unwrap_err();
        let mcp_err: McpError = json_err.into();

        assert!(matches!(mcp_err, McpError::JsonError(_)));
        assert!(mcp_err.to_string().contains("JSON error"));
    }
}

//! Common test utilities for integration tests
//!
//! Provides shared fixtures, helpers, and test utilities used across
//! multiple integration test files.

use std::path::PathBuf;
use tempfile::TempDir;

/// Create a temporary directory for test isolation
///
/// Returns a TempDir that will be cleaned up when dropped.
pub fn temp_dir() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp dir")
}

/// Create a temporary test database
///
/// Returns the path to a SQLite database file in a temporary directory.
pub fn temp_db_path() -> (TempDir, PathBuf) {
    let dir = temp_dir();
    let db_path = dir.path().join("test.db");
    (dir, db_path)
}

/// Setup test logging
///
/// Initializes tracing subscriber for test output.
/// Call this at the beginning of tests that need logging.
#[allow(dead_code)]
pub fn setup_test_logging() {
    use tracing_subscriber::fmt;

    let _ = fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}

/// Wait for a condition to be true with timeout
///
/// Polls the predicate every 100ms until it returns true or timeout is reached.
///
/// # Arguments
///
/// * `predicate` - Function that returns true when condition is met
/// * `timeout` - Maximum time to wait in milliseconds
///
/// # Returns
///
/// * `true` - Condition was met within timeout
/// * `false` - Timeout occurred
#[allow(dead_code)]
pub async fn wait_for<F>(mut predicate: F, timeout_ms: u64) -> bool
where
    F: FnMut() -> bool,
{
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);

    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    false
}

/// Mock data generators
pub mod mock_data {
    use serde_json::json;

    /// Generate mock tool definition
    #[allow(dead_code)]
    pub fn mock_tool(name: &str) -> serde_json::Value {
        json!({
            "name": name,
            "description": format!("Mock tool: {}", name),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Input parameter"
                    }
                },
                "required": ["input"]
            }
        })
    }

    /// Generate mock resource definition
    #[allow(dead_code)]
    pub fn mock_resource(uri: &str, name: &str) -> serde_json::Value {
        json!({
            "uri": uri,
            "name": name,
            "mimeType": "text/plain"
        })
    }

    /// Generate mock tool call response
    #[allow(dead_code)]
    pub fn mock_tool_response(content: &str) -> serde_json::Value {
        json!({
            "content": [{
                "type": "text",
                "text": content
            }]
        })
    }

    /// Generate mock resource read response
    #[allow(dead_code)]
    pub fn mock_resource_content(text: &str) -> serde_json::Value {
        json!({
            "contents": [{
                "uri": "test://resource",
                "mimeType": "text/plain",
                "text": text
            }]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_dir_creation() {
        let dir = temp_dir();
        assert!(dir.path().exists());
        assert!(dir.path().is_dir());
    }

    #[test]
    fn test_temp_db_path() {
        let (_dir, path) = temp_db_path();
        assert!(path.file_name().is_some());
        assert_eq!(path.file_name().unwrap(), "test.db");
    }

    #[tokio::test]
    async fn test_wait_for_immediate_true() {
        let result = wait_for(|| true, 1000).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_wait_for_timeout() {
        let result = wait_for(|| false, 200).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_wait_for_eventual_true() {
        let start = std::time::Instant::now();
        let result = wait_for(|| start.elapsed().as_millis() > 150, 1000).await;
        assert!(result);
    }
}

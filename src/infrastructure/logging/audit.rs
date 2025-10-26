//! Audit logging for security-relevant operations
//!
//! Provides structured JSON audit trail for critical operations like:
//! - Task creation/cancellation
//! - Agent spawning/failures
//! - Configuration changes
//! - API key access

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

/// Audit logger for security-relevant operations
#[derive(Clone)]
pub struct AuditLogger {
    log_file: Arc<Mutex<File>>,
}

/// Audit event types for categorizing operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    TaskCreated,
    TaskCancelled,
    AgentSpawned,
    AgentFailed,
    ConfigChanged,
    ApiKeyAccessed,
}

/// Outcome of an audited operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Success,
    Failure,
    PartialSuccess,
}

/// Complete audit event record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub operation: String,
    pub actor: String,
    pub resource_id: Option<String>,
    pub outcome: AuditOutcome,
    pub metadata: Option<Value>,
}

impl AuditLogger {
    /// Create a new audit logger writing to the specified file
    ///
    /// Creates parent directories if they don't exist
    /// Opens file in append mode to preserve existing audit trail
    pub async fn new(log_path: impl AsRef<Path>) -> Result<Self> {
        let log_path = log_path.as_ref();

        // Create parent directories if needed
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("failed to create audit log directory")?;
        }

        // Open file in append mode (blocking operation)
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .context("failed to open audit log file")?;

        Ok(Self {
            log_file: Arc::new(Mutex::new(file)),
        })
    }

    /// Log an audit event
    ///
    /// Writes the event as a JSON line to the audit log file
    /// Also emits a tracing info event for structured logging
    pub async fn log_event(&self, event: AuditEvent) -> Result<()> {
        // Serialize to JSON
        let json = serde_json::to_string(&event).context("failed to serialize audit event")?;

        // Write to file (blocking operation in mutex)
        {
            let mut file = self
                .log_file
                .lock()
                .map_err(|e| anyhow::anyhow!("audit log mutex poisoned: {}", e))?;

            writeln!(file, "{}", json).context("failed to write audit event")?;
            file.flush().context("failed to flush audit log")?;
        }

        // Also log to tracing infrastructure
        info!(
            event_type = ?event.event_type,
            operation = %event.operation,
            actor = %event.actor,
            resource_id = ?event.resource_id,
            outcome = ?event.outcome,
            "audit event"
        );

        Ok(())
    }

    /// Convenience method for logging an operation
    ///
    /// # Arguments
    /// * `operation` - Description of the operation (e.g., "cancel_task", "spawn_agent")
    /// * `actor` - User or system component performing the operation
    /// * `resource_id` - Optional identifier of the affected resource
    /// * `success` - Whether the operation succeeded
    /// * `metadata` - Optional additional context as JSON
    pub async fn log_operation(
        &self,
        operation: &str,
        actor: &str,
        resource_id: Option<&str>,
        success: bool,
        metadata: Option<Value>,
    ) -> Result<()> {
        // Infer event type from operation name
        let event_type = self.infer_event_type(operation);

        let outcome = if success {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        };

        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type,
            operation: operation.to_string(),
            actor: actor.to_string(),
            resource_id: resource_id.map(String::from),
            outcome,
            metadata,
        };

        self.log_event(event).await
    }

    /// Infer event type from operation name
    fn infer_event_type(&self, operation: &str) -> AuditEventType {
        let op_lower = operation.to_lowercase();

        if op_lower.contains("task") && op_lower.contains("create") {
            AuditEventType::TaskCreated
        } else if op_lower.contains("task") && op_lower.contains("cancel") {
            AuditEventType::TaskCancelled
        } else if op_lower.contains("agent") && op_lower.contains("fail") {
            // Check for "fail" before "spawn" to handle "spawn_failed" correctly
            AuditEventType::AgentFailed
        } else if op_lower.contains("agent") && op_lower.contains("spawn") {
            AuditEventType::AgentSpawned
        } else if op_lower.contains("config") {
            AuditEventType::ConfigChanged
        } else if op_lower.contains("api") && op_lower.contains("key") {
            AuditEventType::ApiKeyAccessed
        } else {
            // Default to config changed for unrecognized operations
            warn!(operation = %operation, "could not infer audit event type");
            AuditEventType::ConfigChanged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_audit_logger_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let _logger = AuditLogger::new(&log_path).await.unwrap();
        assert!(log_path.exists());
    }

    #[tokio::test]
    async fn test_audit_logger_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("logs/audit/events.log");

        let _logger = AuditLogger::new(&log_path).await.unwrap();
        assert!(log_path.exists());
        assert!(log_path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn test_log_operation_writes_json() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let logger = AuditLogger::new(&log_path).await.unwrap();

        logger
            .log_operation("create_task", "user@example.com", Some("task-123"), true, None)
            .await
            .unwrap();

        // Read the log file
        let contents = std::fs::read_to_string(&log_path).unwrap();
        assert!(!contents.is_empty());

        // Parse as JSON
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 1);

        let event: AuditEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event.operation, "create_task");
        assert_eq!(event.actor, "user@example.com");
        assert_eq!(event.resource_id, Some("task-123".to_string()));
        assert_eq!(event.outcome, AuditOutcome::Success);
    }

    #[tokio::test]
    async fn test_log_event_with_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let logger = AuditLogger::new(&log_path).await.unwrap();

        let metadata = serde_json::json!({
            "priority": 5,
            "dependencies": ["dep1", "dep2"]
        });

        logger
            .log_operation(
                "spawn_agent",
                "system",
                Some("agent-456"),
                true,
                Some(metadata.clone()),
            )
            .await
            .unwrap();

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let event: AuditEvent = serde_json::from_str(contents.lines().next().unwrap()).unwrap();

        assert_eq!(event.metadata, Some(metadata));
    }

    #[tokio::test]
    async fn test_multiple_events_append() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let logger = AuditLogger::new(&log_path).await.unwrap();

        logger
            .log_operation("create_task", "user1", Some("task-1"), true, None)
            .await
            .unwrap();

        logger
            .log_operation("cancel_task", "user2", Some("task-2"), false, None)
            .await
            .unwrap();

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        let event1: AuditEvent = serde_json::from_str(lines[0]).unwrap();
        let event2: AuditEvent = serde_json::from_str(lines[1]).unwrap();

        assert_eq!(event1.actor, "user1");
        assert_eq!(event2.actor, "user2");
        assert_eq!(event2.outcome, AuditOutcome::Failure);
    }

    #[tokio::test]
    async fn test_concurrent_writes() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let logger = AuditLogger::new(&log_path).await.unwrap();

        // Spawn multiple concurrent writes
        let mut handles = vec![];
        for i in 0..10 {
            let logger_clone = logger.clone();
            let handle = tokio::spawn(async move {
                logger_clone
                    .log_operation(
                        "concurrent_test",
                        &format!("user{}", i),
                        Some(&format!("resource-{}", i)),
                        true,
                        None,
                    )
                    .await
                    .unwrap();
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all 10 events were written
        let contents = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 10);
    }

    #[tokio::test]
    async fn test_event_type_inference() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let logger = AuditLogger::new(&log_path).await.unwrap();

        logger
            .log_operation("create_task", "user", None, true, None)
            .await
            .unwrap();

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let event: AuditEvent = serde_json::from_str(contents.lines().next().unwrap()).unwrap();

        assert_eq!(event.event_type, AuditEventType::TaskCreated);
    }

    #[tokio::test]
    async fn test_failure_outcome() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("audit.log");

        let logger = AuditLogger::new(&log_path).await.unwrap();

        logger
            .log_operation("agent_spawn_failed", "system", Some("agent-999"), false, None)
            .await
            .unwrap();

        let contents = std::fs::read_to_string(&log_path).unwrap();
        let event: AuditEvent = serde_json::from_str(contents.lines().next().unwrap()).unwrap();

        assert_eq!(event.outcome, AuditOutcome::Failure);
        assert_eq!(event.event_type, AuditEventType::AgentFailed);
    }
}

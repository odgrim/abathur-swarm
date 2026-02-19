//! ClickUp API response and request models.
//!
//! These structs map to the ClickUp REST API v2 JSON payloads.
//! They are used internally by the ClickUp adapter and are not
//! part of the public domain model.

use serde::{Deserialize, Serialize};

/// A task returned by the ClickUp API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpTask {
    /// Unique task identifier (e.g., "abc123").
    pub id: String,
    /// Task name / title.
    pub name: String,
    /// Task description (may be empty or contain markdown).
    #[serde(default)]
    pub description: Option<String>,
    /// Current status of the task.
    pub status: ClickUpStatus,
    /// Priority level (may be absent if unset).
    pub priority: Option<ClickUpPriority>,
    /// Unix timestamp (milliseconds) of the last update.
    #[serde(default)]
    pub date_updated: Option<String>,
    /// URL to view the task in the ClickUp UI.
    #[serde(default)]
    pub url: Option<String>,
    /// Tags applied to the task.
    #[serde(default)]
    pub tags: Vec<ClickUpTag>,
    /// Reference to the list this task belongs to.
    #[serde(default)]
    pub list: Option<ClickUpListRef>,
}

/// The status of a ClickUp task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpStatus {
    /// The status string (e.g., "open", "in progress", "closed").
    pub status: String,
    /// Optional status type (e.g., "open", "closed", "custom").
    #[serde(rename = "type", default)]
    pub status_type: Option<String>,
}

/// A ClickUp priority value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpPriority {
    /// Numeric priority identifier (1 = urgent, 2 = high, 3 = normal, 4 = low).
    pub id: String,
    /// Human-readable priority name.
    pub priority: String,
}

/// A tag applied to a ClickUp task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpTag {
    /// The tag name.
    pub name: String,
}

/// A reference to a ClickUp list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpListRef {
    /// The list's unique identifier.
    pub id: String,
    /// The list's display name (may be absent in some responses).
    #[serde(default)]
    pub name: Option<String>,
}

/// Response wrapper for the "get tasks" endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpTasksResponse {
    /// The list of tasks returned.
    pub tasks: Vec<ClickUpTask>,
}

/// Response from task creation or single-task endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpTaskResponse {
    /// The created/returned task's identifier.
    pub id: String,
    /// URL to view the task in the ClickUp UI.
    #[serde(default)]
    pub url: Option<String>,
}

/// Request body for posting a comment on a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickUpCommentRequest {
    /// The plain-text comment body.
    pub comment_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tasks_response_deserialization() {
        let json = r#"{
            "tasks": [
                {
                    "id": "abc123",
                    "name": "Fix login bug",
                    "description": "Users can't log in",
                    "status": { "status": "open", "type": "open" },
                    "priority": { "id": "2", "priority": "high" },
                    "date_updated": "1700000000000",
                    "url": "https://app.clickup.com/t/abc123",
                    "tags": [{ "name": "bug" }],
                    "list": { "id": "list1", "name": "Sprint 1" }
                }
            ]
        }"#;
        let resp: ClickUpTasksResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.tasks.len(), 1);
        assert_eq!(resp.tasks[0].id, "abc123");
        assert_eq!(resp.tasks[0].name, "Fix login bug");
        assert_eq!(resp.tasks[0].status.status, "open");
        assert!(resp.tasks[0].priority.is_some());
        assert_eq!(resp.tasks[0].tags.len(), 1);
        assert_eq!(resp.tasks[0].tags[0].name, "bug");
    }

    #[test]
    fn test_task_response_deserialization() {
        let json = r#"{ "id": "xyz789", "url": "https://app.clickup.com/t/xyz789" }"#;
        let resp: ClickUpTaskResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "xyz789");
        assert_eq!(resp.url.as_deref(), Some("https://app.clickup.com/t/xyz789"));
    }

    #[test]
    fn test_comment_request_serialization() {
        let req = ClickUpCommentRequest {
            comment_text: "Task completed.".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("comment_text"));
        assert!(json.contains("Task completed."));
    }

    #[test]
    fn test_task_with_missing_optional_fields() {
        let json = r#"{
            "id": "min1",
            "name": "Minimal task",
            "status": { "status": "open" },
            "priority": null,
            "tags": []
        }"#;
        let task: ClickUpTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "min1");
        assert!(task.description.is_none());
        assert!(task.priority.is_none());
        assert!(task.date_updated.is_none());
        assert!(task.url.is_none());
        assert!(task.list.is_none());
    }
}

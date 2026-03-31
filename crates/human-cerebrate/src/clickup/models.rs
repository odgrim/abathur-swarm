//! ClickUp API request and response types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub description: String,
    pub priority: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpTask {
    #[expect(dead_code, reason = "deserialized from ClickUp API, used in logging/debugging")]
    pub id: String,
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub name: String,
    pub status: ClickUpStatus,
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub date_created: String,
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub due_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpStatus {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ClickUpCommentsResponse {
    pub comments: Vec<ClickUpComment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClickUpComment {
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub id: String,
    pub comment_text: String,
    pub date: String,
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub user: ClickUpUser,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClickUpUser {
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub id: u64,
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskResponse {
    pub id: String,
    #[expect(dead_code, reason = "deserialized from ClickUp API")]
    pub name: String,
    pub status: ClickUpStatus,
}

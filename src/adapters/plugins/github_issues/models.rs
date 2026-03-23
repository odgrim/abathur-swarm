//! GitHub Issues API response and request models.
//!
//! These structs map to the GitHub REST API v3 JSON payloads.
//! They are used internally by the GitHub Issues adapter and are not
//! part of the public domain model.

use serde::{Deserialize, Serialize};

/// An issue returned by the GitHub API.
///
/// Note: issues and pull requests share the same endpoint. Pull requests
/// include a non-null `pull_request` field; ingestion skips those.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubIssue {
    /// Unique numeric identifier for the issue.
    pub id: u64,
    /// Sequential number within the repository (e.g., 42 → "#42").
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// Issue body text (may be absent or null).
    #[serde(default)]
    pub body: Option<String>,
    /// Current state: "open" or "closed".
    pub state: String,
    /// Labels applied to the issue.
    #[serde(default)]
    pub labels: Vec<GitHubLabel>,
    /// Present when this item is actually a pull request, not an issue.
    #[serde(default)]
    pub pull_request: Option<GitHubPullRequestRef>,
    /// ISO 8601 timestamp of the last update.
    pub updated_at: String,
    /// URL to view the issue in the GitHub UI.
    pub html_url: String,
    /// ISO 8601 timestamp of creation.
    pub created_at: String,
}

/// A label applied to a GitHub issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubLabel {
    /// The label name (e.g., "bug", "priority: high").
    pub name: String,
    /// Hex colour without the leading `#`.
    pub color: String,
}

/// Reference object present on pull requests (absent on plain issues).
///
/// The issues endpoint returns this stub for PRs. Use [`GitHubPullRequestDetail`]
/// (fetched via `GET /pulls/{n}`) for full PR metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestRef {
    /// API URL of the pull request resource.
    pub url: String,
}

/// A GitHub user (author, assignee, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    /// The user's login handle (e.g., "octocat").
    pub login: String,
}

/// The head (source) branch of a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestHead {
    /// Head branch name (e.g., "feature/my-change").
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The SHA at the tip of the head branch.
    pub sha: String,
}

/// The base (target) branch of a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestBase {
    /// Base branch name (e.g., "main").
    #[serde(rename = "ref")]
    pub ref_name: String,
}

/// Full pull request detail from `GET /repos/{owner}/{repo}/pulls/{number}`.
///
/// The issues endpoint only returns a [`GitHubPullRequestRef`] stub; this struct
/// carries the additional metadata needed for review: branch info, merge state,
/// draft status, and author.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestDetail {
    /// PR number within the repository.
    pub number: u64,
    /// PR title.
    pub title: String,
    /// PR body text (may be absent or null).
    #[serde(default)]
    pub body: Option<String>,
    /// Current state: "open" or "closed".
    pub state: String,
    /// Whether this PR is a draft.
    #[serde(default)]
    pub draft: bool,
    /// Whether the PR is mergeable (null when unknown/computing).
    #[serde(default)]
    pub mergeable: Option<bool>,
    /// The head (source) branch.
    pub head: GitHubPullRequestHead,
    /// The base (target) branch.
    pub base: GitHubPullRequestBase,
    /// The PR author.
    pub user: GitHubUser,
    /// URL to view the PR in the GitHub UI.
    pub html_url: String,
    /// ISO 8601 timestamp of the last update.
    pub updated_at: String,
    /// ISO 8601 timestamp of creation.
    pub created_at: String,
}

/// Request body for posting a comment on an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubCommentRequest {
    /// The comment body (plain text or Markdown).
    pub body: String,
}

/// Request body for creating a new GitHub issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubCreateIssueRequest {
    /// Issue title.
    pub title: String,
    /// Issue body text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Labels to apply to the new issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
}

/// Response from the create-issue and create-pull-request endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubCreateIssueResponse {
    /// Issue or PR number within the repository.
    pub number: u64,
    /// URL to view the issue/PR in the GitHub UI.
    pub html_url: String,
}

/// Request body for updating (patching) an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubIssueUpdateRequest {
    /// New state: "open" or "closed".
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_issue_deserialization() {
        let json = r#"{
            "id": 1,
            "number": 42,
            "title": "Fix login bug",
            "body": "Users cannot log in after the last deploy.",
            "state": "open",
            "labels": [
                { "name": "bug", "color": "d73a4a" },
                { "name": "priority: high", "color": "e4e669" }
            ],
            "pull_request": null,
            "updated_at": "2024-01-15T10:30:00Z",
            "html_url": "https://github.com/org/repo/issues/42",
            "created_at": "2024-01-14T08:00:00Z"
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.number, 42);
        assert_eq!(issue.title, "Fix login bug");
        assert_eq!(issue.state, "open");
        assert_eq!(issue.labels.len(), 2);
        assert_eq!(issue.labels[0].name, "bug");
        assert!(issue.pull_request.is_none());
        assert!(issue.body.is_some());
    }

    #[test]
    fn test_minimal_issue_deserialization() {
        let json = r#"{
            "id": 2,
            "number": 1,
            "title": "Minimal issue",
            "state": "closed",
            "updated_at": "2024-01-10T00:00:00Z",
            "html_url": "https://github.com/org/repo/issues/1",
            "created_at": "2024-01-09T00:00:00Z"
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.number, 1);
        assert!(issue.body.is_none());
        assert!(issue.labels.is_empty());
        assert!(issue.pull_request.is_none());
    }

    #[test]
    fn test_pr_detection_via_pull_request_field() {
        let json = r#"{
            "id": 3,
            "number": 99,
            "title": "Add feature X",
            "state": "open",
            "labels": [],
            "pull_request": { "url": "https://api.github.com/repos/org/repo/pulls/99" },
            "updated_at": "2024-01-16T12:00:00Z",
            "html_url": "https://github.com/org/repo/pull/99",
            "created_at": "2024-01-15T09:00:00Z"
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert!(issue.pull_request.is_some());
    }

    #[test]
    fn test_comment_request_serialization() {
        let req = GitHubCommentRequest {
            body: "Task completed.".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("body"));
        assert!(json.contains("Task completed."));
    }

    #[test]
    fn test_create_issue_request_serialization() {
        let req = GitHubCreateIssueRequest {
            title: "New issue".to_string(),
            body: Some("Description here".to_string()),
            labels: Some(vec!["bug".to_string()]),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("title"));
        assert!(json.contains("New issue"));
        assert!(json.contains("body"));
        assert!(json.contains("labels"));
    }

    #[test]
    fn test_create_issue_request_omits_none_fields() {
        let req = GitHubCreateIssueRequest {
            title: "Minimal".to_string(),
            body: None,
            labels: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"body\""));
        assert!(!json.contains("\"labels\""));
    }

    #[test]
    fn test_pull_request_detail_deserialization() {
        let json = r#"{
            "number": 99,
            "title": "Add feature X",
            "body": "This PR adds feature X.",
            "state": "open",
            "draft": false,
            "mergeable": true,
            "head": { "ref": "feature/x", "sha": "abc123" },
            "base": { "ref": "main" },
            "user": { "login": "octocat" },
            "html_url": "https://github.com/org/repo/pull/99",
            "updated_at": "2024-01-16T12:00:00Z",
            "created_at": "2024-01-15T09:00:00Z"
        }"#;
        let pr: super::GitHubPullRequestDetail = serde_json::from_str(json).unwrap();
        assert_eq!(pr.number, 99);
        assert_eq!(pr.title, "Add feature X");
        assert!(!pr.draft);
        assert_eq!(pr.mergeable, Some(true));
        assert_eq!(pr.head.sha, "abc123");
        assert_eq!(pr.head.ref_name, "feature/x");
        assert_eq!(pr.base.ref_name, "main");
        assert_eq!(pr.user.login, "octocat");
    }

    #[test]
    fn test_pull_request_detail_draft_default() {
        let json = r#"{
            "number": 1,
            "title": "Draft PR",
            "state": "open",
            "head": { "ref": "feature/y", "sha": "def456" },
            "base": { "ref": "main" },
            "user": { "login": "dev" },
            "html_url": "https://github.com/org/repo/pull/1",
            "updated_at": "2024-01-16T12:00:00Z",
            "created_at": "2024-01-15T09:00:00Z"
        }"#;
        let pr: super::GitHubPullRequestDetail = serde_json::from_str(json).unwrap();
        // draft defaults to false, mergeable defaults to None
        assert!(!pr.draft);
        assert!(pr.mergeable.is_none());
        assert!(pr.body.is_none());
    }

    #[test]
    fn test_create_issue_response_deserialization() {
        let json = r#"{ "number": 7, "html_url": "https://github.com/org/repo/issues/7" }"#;
        let resp: GitHubCreateIssueResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.number, 7);
        assert_eq!(resp.html_url, "https://github.com/org/repo/issues/7");
    }
}

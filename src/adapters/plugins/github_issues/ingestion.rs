//! GitHub Issues ingestion adapter.
//!
//! Polls a GitHub repository for issues and maps them to [`IngestionItem`]s.
//! Pull requests are automatically filtered out. Supports incremental
//! polling via the `since` parameter and optional label-based filtering.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::errors::DomainResult;
use crate::domain::models::adapter::{AdapterManifest, IngestionItem};
use crate::domain::models::TaskPriority;
use crate::domain::ports::adapter::IngestionAdapter;

use super::client::GitHubClient;
use super::models::GitHubIssue;

/// Adapter that ingests issues from a GitHub repository.
///
/// Configuration is read from the [`AdapterManifest::config`] map:
/// - `owner` (required): repository owner (user or organisation name).
/// - `repo` (required): repository name.
/// - `state` (optional): issue state filter — `"open"`, `"closed"`, or
///   `"all"`. Defaults to `"open"`.
/// - `filter_labels` (optional): comma-separated list of label names; when
///   set, only issues that carry at least one matching label are ingested.
#[derive(Debug)]
pub struct GitHubIngestionAdapter {
    /// The adapter manifest describing capabilities and config.
    manifest: AdapterManifest,
    /// Shared GitHub HTTP client.
    client: Arc<GitHubClient>,
}

impl GitHubIngestionAdapter {
    /// Create a new ingestion adapter.
    pub fn new(manifest: AdapterManifest, client: Arc<GitHubClient>) -> Self {
        Self { manifest, client }
    }

    /// Read the `owner` from the manifest config.
    fn owner(&self) -> Option<&str> {
        self.manifest
            .config
            .get("owner")
            .and_then(|v| v.as_str())
    }

    /// Read the `repo` from the manifest config.
    fn repo(&self) -> Option<&str> {
        self.manifest
            .config
            .get("repo")
            .and_then(|v| v.as_str())
    }

    /// Read the `state` from the manifest config, defaulting to `"open"`.
    fn state(&self) -> &str {
        self.manifest
            .config
            .get("state")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("open")
    }

    /// Read the optional `filter_labels` list from the manifest config.
    ///
    /// Returns `None` when the key is absent or empty, so that callers
    /// can distinguish "no filter" from "filter on empty set".
    fn filter_labels(&self) -> Option<Vec<&str>> {
        let raw = self
            .manifest
            .config
            .get("filter_labels")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())?;

        let labels: Vec<&str> = raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        if labels.is_empty() {
            None
        } else {
            Some(labels)
        }
    }

    /// Map a GitHub issue's labels to a [`TaskPriority`].
    ///
    /// Recognises labels whose names contain priority keywords
    /// (case-insensitive):
    /// - `"critical"` → [`TaskPriority::Critical`]
    /// - `"high"` → [`TaskPriority::High`]
    /// - `"medium"` or `"normal"` → [`TaskPriority::Normal`]
    /// - `"low"` → [`TaskPriority::Low`]
    ///
    /// Returns `None` when no priority label is found.
    pub fn extract_priority(issue: &GitHubIssue) -> Option<TaskPriority> {
        for label in &issue.labels {
            let name = label.name.to_lowercase();
            if name.contains("critical") {
                return Some(TaskPriority::Critical);
            }
            if name.contains("high") {
                return Some(TaskPriority::High);
            }
            if name.contains("medium") || name.contains("normal") {
                return Some(TaskPriority::Normal);
            }
            if name.contains("low") {
                return Some(TaskPriority::Low);
            }
        }
        None
    }

    /// Convert a [`GitHubIssue`] to an [`IngestionItem`].
    fn to_ingestion_item(issue: &GitHubIssue) -> IngestionItem {
        let external_id = issue.number.to_string();
        let description = issue.body.clone().unwrap_or_default();

        let mut item = IngestionItem::new(&external_id, &issue.title, description);

        if let Some(priority) = Self::extract_priority(issue) {
            item = item.with_priority(priority);
        }

        // Store the GitHub state and URL as metadata.
        item = item.with_metadata("github_state", serde_json::json!(issue.state));
        item = item.with_metadata("github_url", serde_json::json!(issue.html_url));

        // Store label names as metadata.
        if !issue.labels.is_empty() {
            let label_names: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
            item = item.with_metadata("github_labels", serde_json::json!(label_names));
        }

        // Parse `updated_at` (ISO 8601) to `DateTime<Utc>`.
        if let Ok(dt) = issue.updated_at.parse::<DateTime<Utc>>() {
            item = item.with_external_updated_at(dt);
        }

        item
    }
}

#[async_trait]
impl IngestionAdapter for GitHubIngestionAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    async fn poll(&self, last_poll: Option<DateTime<Utc>>) -> DomainResult<Vec<IngestionItem>> {
        let owner = self.owner().ok_or_else(|| {
            crate::domain::errors::DomainError::ValidationFailed(
                "GitHub Issues adapter config missing required 'owner'".to_string(),
            )
        })?;
        let repo = self.repo().ok_or_else(|| {
            crate::domain::errors::DomainError::ValidationFailed(
                "GitHub Issues adapter config missing required 'repo'".to_string(),
            )
        })?;

        let state = self.state();

        // Convert last_poll to an ISO 8601 string for the `since` parameter.
        let since_str = last_poll.map(|dt| dt.to_rfc3339());
        let since = since_str.as_deref();

        tracing::info!(
            owner = owner,
            repo = repo,
            state = state,
            since = ?since,
            "Polling GitHub Issues"
        );

        let issues = self.client.list_issues(owner, repo, state, since).await?;

        let filter_labels = self.filter_labels();

        let items: Vec<IngestionItem> = issues
            .iter()
            // Skip pull requests — GitHub returns them from the issues endpoint.
            .filter(|issue| issue.pull_request.is_none())
            .filter(|issue| {
                // If filter_labels is configured, only include matching issues.
                if let Some(ref required) = filter_labels {
                    issue
                        .labels
                        .iter()
                        .any(|l| required.contains(&l.name.as_str()))
                } else {
                    true
                }
            })
            .map(Self::to_ingestion_item)
            .collect();

        tracing::info!(
            count = items.len(),
            total_fetched = issues.len(),
            "GitHub Issues ingestion poll complete"
        );

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::plugins::github_issues::models::{GitHubLabel, GitHubPullRequestRef};
    use crate::domain::models::adapter::{
        AdapterCapability, AdapterDirection, AdapterType,
    };

    fn test_manifest() -> AdapterManifest {
        AdapterManifest::new(
            "github-issues",
            AdapterType::Native,
            AdapterDirection::Bidirectional,
        )
        .with_capability(AdapterCapability::PollItems)
        .with_config("owner", serde_json::json!("my-org"))
        .with_config("repo", serde_json::json!("my-repo"))
    }

    fn make_github_issue(number: u64, title: &str, labels: Vec<&str>) -> GitHubIssue {
        GitHubIssue {
            id: number,
            number,
            title: title.to_string(),
            body: Some("Issue description".to_string()),
            state: "open".to_string(),
            labels: labels
                .into_iter()
                .map(|n| GitHubLabel {
                    name: n.to_string(),
                    color: "ffffff".to_string(),
                })
                .collect(),
            pull_request: None,
            updated_at: "2024-01-15T10:30:00Z".to_string(),
            html_url: format!("https://github.com/my-org/my-repo/issues/{number}"),
            created_at: "2024-01-14T08:00:00Z".to_string(),
        }
    }

    // ── Priority extraction ─────────────────────────────────────────────────

    #[test]
    fn test_extract_priority_critical() {
        let issue = make_github_issue(1, "Crash on startup", vec!["priority: critical"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), Some(TaskPriority::Critical));
    }

    #[test]
    fn test_extract_priority_high() {
        let issue = make_github_issue(2, "Slow query", vec!["priority: high"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), Some(TaskPriority::High));
    }

    #[test]
    fn test_extract_priority_normal_via_medium() {
        let issue = make_github_issue(3, "UI glitch", vec!["priority: medium"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), Some(TaskPriority::Normal));
    }

    #[test]
    fn test_extract_priority_normal_via_normal() {
        let issue = make_github_issue(4, "Docs update", vec!["priority: normal"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), Some(TaskPriority::Normal));
    }

    #[test]
    fn test_extract_priority_low() {
        let issue = make_github_issue(5, "Minor typo", vec!["priority: low"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), Some(TaskPriority::Low));
    }

    #[test]
    fn test_extract_priority_case_insensitive() {
        let issue = make_github_issue(6, "Thing", vec!["PRIORITY: HIGH"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), Some(TaskPriority::High));
    }

    #[test]
    fn test_extract_priority_no_match() {
        let issue = make_github_issue(7, "No priority label", vec!["bug", "help wanted"]);
        assert_eq!(GitHubIngestionAdapter::extract_priority(&issue), None);
    }

    // ── to_ingestion_item ───────────────────────────────────────────────────

    #[test]
    fn test_to_ingestion_item_full() {
        let issue = make_github_issue(42, "Fix login bug", vec!["bug", "priority: high"]);
        let item = GitHubIngestionAdapter::to_ingestion_item(&issue);

        assert_eq!(item.external_id, "42");
        assert_eq!(item.title, "Fix login bug");
        assert_eq!(item.description, "Issue description");
        assert_eq!(item.priority, Some(TaskPriority::High));
        assert!(item.metadata.contains_key("github_state"));
        assert!(item.metadata.contains_key("github_url"));
        assert!(item.metadata.contains_key("github_labels"));
        assert!(item.external_updated_at.is_some());
    }

    #[test]
    fn test_to_ingestion_item_minimal() {
        let mut issue = make_github_issue(1, "Minimal", vec![]);
        issue.body = None;
        let item = GitHubIngestionAdapter::to_ingestion_item(&issue);

        assert_eq!(item.external_id, "1");
        assert_eq!(item.description, "");
        assert!(item.priority.is_none());
        assert!(!item.metadata.contains_key("github_labels"));
    }

    // ── filter_labels config ────────────────────────────────────────────────

    #[test]
    fn test_filter_labels_from_config() {
        let manifest = test_manifest()
            .with_config("filter_labels", serde_json::json!("bug, enhancement"));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        let labels = adapter.filter_labels().unwrap();
        assert!(labels.contains(&"bug"));
        assert!(labels.contains(&"enhancement"));
    }

    #[test]
    fn test_filter_labels_absent() {
        let manifest = test_manifest();
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert!(adapter.filter_labels().is_none());
    }

    #[test]
    fn test_filter_labels_empty_string_treated_as_absent() {
        let manifest = test_manifest()
            .with_config("filter_labels", serde_json::json!(""));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert!(adapter.filter_labels().is_none());
    }

    // ── state config ────────────────────────────────────────────────────────

    #[test]
    fn test_state_defaults_to_open() {
        let manifest = test_manifest();
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert_eq!(adapter.state(), "open");
    }

    #[test]
    fn test_state_from_config() {
        let manifest = test_manifest()
            .with_config("state", serde_json::json!("all"));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert_eq!(adapter.state(), "all");
    }

    // ── PR filtering ────────────────────────────────────────────────────────

    #[test]
    fn test_pr_is_skipped() {
        let mut pr = make_github_issue(99, "Add feature X", vec![]);
        pr.pull_request = Some(GitHubPullRequestRef {
            url: "https://api.github.com/repos/org/repo/pulls/99".to_string(),
        });
        let regular = make_github_issue(100, "Regular issue", vec![]);

        // Replicate the exact filter predicate from poll() to verify PRs are excluded
        // from the IngestionItem output without requiring a live network call.
        let issues = vec![pr, regular];
        let items: Vec<IngestionItem> = issues
            .iter()
            .filter(|issue| issue.pull_request.is_none())
            .map(GitHubIngestionAdapter::to_ingestion_item)
            .collect();

        assert_eq!(items.len(), 1, "Only the non-PR issue should be included");
        assert_eq!(items[0].external_id, "100", "PR issue #99 must be filtered out");
    }
}

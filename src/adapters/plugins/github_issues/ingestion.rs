//! GitHub Issues ingestion adapter.
//!
//! Polls a GitHub repository for issues and maps them to [`IngestionItem`]s.
//! When `ingest_pull_requests` is enabled, pull requests are hydrated via the
//! pulls endpoint, filtered (draft, author, base branch), and emitted with
//! `IngestionItemKind::PullRequest` and PR-specific metadata. When disabled
//! (the default), pull requests are silently skipped.
//! Supports incremental polling via the `since` parameter and optional
//! label-based filtering.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::errors::DomainResult;
use crate::domain::models::adapter::{AdapterManifest, IngestionItem, IngestionItemKind};
use crate::domain::models::TaskPriority;
use crate::domain::ports::adapter::IngestionAdapter;

use super::client::GitHubClient;
use super::models::{GitHubIssue, GitHubPullRequestDetail};

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

    /// Whether PR ingestion is enabled. Defaults to `false` (opt-in).
    fn ingest_pull_requests(&self) -> bool {
        self.manifest
            .config
            .get("ingest_pull_requests")
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("true"))
            .or_else(|| {
                self.manifest
                    .config
                    .get("ingest_pull_requests")
                    .and_then(|v| v.as_bool())
            })
            .unwrap_or(false)
    }

    /// Read the optional `pr_base_filter` list from config.
    ///
    /// When set, only PRs targeting one of these base branches are ingested.
    fn pr_base_filter(&self) -> Option<Vec<&str>> {
        let raw = self
            .manifest
            .config
            .get("pr_base_filter")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())?;

        let branches: Vec<&str> = raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        if branches.is_empty() {
            None
        } else {
            Some(branches)
        }
    }

    /// Read the optional `pr_ignore_authors` list from config.
    ///
    /// PRs authored by any of these logins are skipped (e.g., bot accounts).
    fn pr_ignore_authors(&self) -> Option<Vec<&str>> {
        let raw = self
            .manifest
            .config
            .get("pr_ignore_authors")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())?;

        let authors: Vec<&str> = raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        if authors.is_empty() {
            None
        } else {
            Some(authors)
        }
    }

    /// Maximum characters of diff text to include in the task description.
    ///
    /// Defaults to 100 000 characters. Diffs exceeding this limit are truncated
    /// with a marker indicating the omission.
    fn max_diff_chars(&self) -> usize {
        self.manifest
            .config
            .get("max_diff_chars")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .map(|n| n as usize)
            .unwrap_or(100_000)
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

    /// Convert a [`GitHubIssue`] to an [`IngestionItem`] with `IngestionItemKind::Issue`.
    fn to_ingestion_item(issue: &GitHubIssue) -> IngestionItem {
        let external_id = issue.number.to_string();
        let description = issue.body.clone().unwrap_or_default();

        let mut item = IngestionItem::new(&external_id, &issue.title, description)
            .with_item_kind(IngestionItemKind::Issue);

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

    /// Convert a [`GitHubPullRequestDetail`] and its diff to an [`IngestionItem`]
    /// with `IngestionItemKind::PullRequest`.
    fn to_pr_ingestion_item(
        pr: &GitHubPullRequestDetail,
        diff: &str,
        max_diff_chars: usize,
    ) -> IngestionItem {
        let external_id = pr.number.to_string();

        // Truncate diff if it exceeds the limit.
        let truncated_diff = if diff.len() > max_diff_chars {
            let truncated = &diff[..max_diff_chars];
            format!(
                "{truncated}\n\n... [diff truncated at {max_diff_chars} chars; \
                 full diff is {} chars] ...",
                diff.len()
            )
        } else {
            diff.to_string()
        };

        let description = format!(
            "{}\n\n## Diff\n\n```diff\n{}\n```",
            pr.body.as_deref().unwrap_or(""),
            truncated_diff,
        );

        let mut item = IngestionItem::new(&external_id, &pr.title, description)
            .with_item_kind(IngestionItemKind::PullRequest);

        // Store PR-specific metadata.
        item = item.with_metadata("github_state", serde_json::json!(pr.state));
        item = item.with_metadata("github_url", serde_json::json!(pr.html_url));
        item = item.with_metadata("pr_head_sha", serde_json::json!(pr.head.sha));
        item = item.with_metadata("pr_head_ref", serde_json::json!(pr.head.ref_name));
        item = item.with_metadata("pr_base_ref", serde_json::json!(pr.base.ref_name));
        item = item.with_metadata("pr_author", serde_json::json!(pr.user.login));
        item = item.with_metadata("pr_draft", serde_json::json!(pr.draft));
        if let Some(mergeable) = pr.mergeable {
            item = item.with_metadata("pr_mergeable", serde_json::json!(mergeable));
        }

        // Parse `updated_at` (ISO 8601) to `DateTime<Utc>`.
        if let Ok(dt) = pr.updated_at.parse::<DateTime<Utc>>() {
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

        let all_items = self.client.list_issues(owner, repo, state, since).await?;

        let filter_labels = self.filter_labels();
        let ingest_prs = self.ingest_pull_requests();
        let pr_base_filter = self.pr_base_filter();
        let pr_ignore_authors = self.pr_ignore_authors();
        let max_diff_chars = self.max_diff_chars();

        let mut items: Vec<IngestionItem> = Vec::new();

        for gh_item in &all_items {
            if gh_item.pull_request.is_some() {
                // ── Pull request path ──────────────────────────────────────
                if !ingest_prs {
                    tracing::debug!(
                        number = gh_item.number,
                        "Skipping PR (ingest_pull_requests is disabled)"
                    );
                    continue;
                }

                // Hydrate full PR details from the pulls endpoint.
                let pr = match self.client.get_pull_request(owner, repo, gh_item.number).await {
                    Ok(pr) => pr,
                    Err(e) => {
                        tracing::warn!(
                            number = gh_item.number,
                            error = %e,
                            "Failed to hydrate PR details, skipping"
                        );
                        continue;
                    }
                };

                // Skip draft PRs.
                if pr.draft {
                    tracing::debug!(number = pr.number, "Skipping draft PR");
                    continue;
                }

                // Skip ignored authors.
                if let Some(ref ignored) = pr_ignore_authors
                    && ignored.contains(&pr.user.login.as_str())
                {
                    tracing::debug!(
                        number = pr.number,
                        author = %pr.user.login,
                        "Skipping PR from ignored author"
                    );
                    continue;
                }

                // Filter by base branch.
                if let Some(ref allowed_bases) = pr_base_filter
                    && !allowed_bases.contains(&pr.base.ref_name.as_str())
                {
                    tracing::debug!(
                        number = pr.number,
                        base = %pr.base.ref_name,
                        "Skipping PR: base branch not in pr_base_filter"
                    );
                    continue;
                }

                // Fetch the unified diff.
                let diff = match self.client.get_pull_request_diff(owner, repo, pr.number).await {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(
                            number = pr.number,
                            error = %e,
                            "Failed to fetch PR diff, skipping"
                        );
                        continue;
                    }
                };

                items.push(Self::to_pr_ingestion_item(&pr, &diff, max_diff_chars));
            } else {
                // ── Issue path (existing behavior) ─────────────────────────
                // Apply label filter if configured.
                if let Some(ref required) = filter_labels
                    && !gh_item
                        .labels
                        .iter()
                        .any(|l| required.contains(&l.name.as_str()))
                {
                    continue;
                }

                items.push(Self::to_ingestion_item(gh_item));
            }
        }

        tracing::info!(
            count = items.len(),
            total_fetched = all_items.len(),
            "GitHub Issues ingestion poll complete"
        );

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::plugins::github_issues::models::{
        GitHubLabel, GitHubPullRequestRef,
        GitHubPullRequestDetail, GitHubPullRequestHead, GitHubPullRequestBase, GitHubUser,
    };
    use crate::domain::models::adapter::{
        AdapterCapability, AdapterDirection, AdapterType, IngestionItemKind,
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
    fn test_pr_is_skipped_when_ingest_disabled() {
        // When ingest_pull_requests is not set (default=false), PRs are filtered out.
        let mut pr = make_github_issue(99, "Add feature X", vec![]);
        pr.pull_request = Some(GitHubPullRequestRef {
            url: "https://api.github.com/repos/org/repo/pulls/99".to_string(),
        });
        let regular = make_github_issue(100, "Regular issue", vec![]);

        // Replicate the branching logic from poll(): when ingest_pull_requests
        // is false, PRs are skipped — only issues go through to_ingestion_item.
        let issues = vec![pr, regular];
        let ingest_prs = false; // default
        let items: Vec<IngestionItem> = issues
            .iter()
            .filter(|issue| {
                if issue.pull_request.is_some() {
                    ingest_prs
                } else {
                    true
                }
            })
            .map(GitHubIngestionAdapter::to_ingestion_item)
            .collect();

        assert_eq!(items.len(), 1, "Only the non-PR issue should be included");
        assert_eq!(items[0].external_id, "100", "PR issue #99 must be filtered out");
    }

    // ── to_ingestion_item sets IngestionItemKind::Issue ──────────────────

    #[test]
    fn test_to_ingestion_item_sets_issue_kind() {
        let issue = make_github_issue(42, "Fix login bug", vec!["bug"]);
        let item = GitHubIngestionAdapter::to_ingestion_item(&issue);
        assert_eq!(item.item_kind, Some(IngestionItemKind::Issue));
    }

    // ── PR ingestion item ────────────────────────────────────────────────

    fn make_pr_detail(number: u64, title: &str, draft: bool) -> GitHubPullRequestDetail {
        GitHubPullRequestDetail {
            number,
            title: title.to_string(),
            body: Some("PR description body".to_string()),
            state: "open".to_string(),
            draft,
            mergeable: Some(true),
            head: GitHubPullRequestHead {
                ref_name: "feature/my-change".to_string(),
                sha: "abc123def456".to_string(),
            },
            base: GitHubPullRequestBase {
                ref_name: "main".to_string(),
            },
            user: GitHubUser {
                login: "contributor".to_string(),
            },
            html_url: format!("https://github.com/org/repo/pull/{number}"),
            updated_at: "2024-01-15T10:30:00Z".to_string(),
            created_at: "2024-01-14T08:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_to_pr_ingestion_item_full() {
        let pr = make_pr_detail(99, "Add feature X", false);
        let diff = "diff --git a/src/main.rs b/src/main.rs\n+println!(\"hello\");\n";
        let item = GitHubIngestionAdapter::to_pr_ingestion_item(&pr, diff, 100_000);

        assert_eq!(item.external_id, "99");
        assert_eq!(item.title, "Add feature X");
        assert_eq!(item.item_kind, Some(IngestionItemKind::PullRequest));
        assert!(item.description.contains("PR description body"));
        assert!(item.description.contains("```diff"));
        assert!(item.description.contains("println"));

        // Check PR metadata.
        assert_eq!(
            item.metadata.get("pr_head_sha").and_then(|v| v.as_str()),
            Some("abc123def456")
        );
        assert_eq!(
            item.metadata.get("pr_head_ref").and_then(|v| v.as_str()),
            Some("feature/my-change")
        );
        assert_eq!(
            item.metadata.get("pr_base_ref").and_then(|v| v.as_str()),
            Some("main")
        );
        assert_eq!(
            item.metadata.get("pr_author").and_then(|v| v.as_str()),
            Some("contributor")
        );
        assert_eq!(
            item.metadata.get("pr_draft").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            item.metadata.get("pr_mergeable").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(item.external_updated_at.is_some());
    }

    #[test]
    fn test_to_pr_ingestion_item_truncates_diff() {
        let pr = make_pr_detail(100, "Large PR", false);
        let diff = "x".repeat(200);
        let item = GitHubIngestionAdapter::to_pr_ingestion_item(&pr, &diff, 50);

        // The diff in the description should be truncated.
        assert!(item.description.contains("[diff truncated at 50 chars"));
        assert!(item.description.contains("full diff is 200 chars"));
    }

    // ── Config helpers ───────────────────────────────────────────────────

    #[test]
    fn test_ingest_pull_requests_default_false() {
        let manifest = test_manifest();
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert!(!adapter.ingest_pull_requests());
    }

    #[test]
    fn test_ingest_pull_requests_enabled() {
        let manifest = test_manifest()
            .with_config("ingest_pull_requests", serde_json::json!("true"));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert!(adapter.ingest_pull_requests());
    }

    #[test]
    fn test_ingest_pull_requests_bool_value() {
        let manifest = test_manifest()
            .with_config("ingest_pull_requests", serde_json::json!(true));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert!(adapter.ingest_pull_requests());
    }

    #[test]
    fn test_pr_base_filter_from_config() {
        let manifest = test_manifest()
            .with_config("pr_base_filter", serde_json::json!("main, develop"));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        let bases = adapter.pr_base_filter().unwrap();
        assert!(bases.contains(&"main"));
        assert!(bases.contains(&"develop"));
    }

    #[test]
    fn test_pr_base_filter_absent() {
        let manifest = test_manifest();
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert!(adapter.pr_base_filter().is_none());
    }

    #[test]
    fn test_pr_ignore_authors_from_config() {
        let manifest = test_manifest()
            .with_config("pr_ignore_authors", serde_json::json!("dependabot[bot], renovate[bot]"));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        let authors = adapter.pr_ignore_authors().unwrap();
        assert!(authors.contains(&"dependabot[bot]"));
        assert!(authors.contains(&"renovate[bot]"));
    }

    #[test]
    fn test_max_diff_chars_default() {
        let manifest = test_manifest();
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert_eq!(adapter.max_diff_chars(), 100_000);
    }

    #[test]
    fn test_max_diff_chars_from_config() {
        let manifest = test_manifest()
            .with_config("max_diff_chars", serde_json::json!("50000"));
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubIngestionAdapter::new(manifest, client);
        assert_eq!(adapter.max_diff_chars(), 50_000);
    }
}

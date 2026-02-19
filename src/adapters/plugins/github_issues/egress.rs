//! GitHub Issues egress adapter.
//!
//! Executes egress actions against the GitHub REST API, mapping each
//! [`EgressAction`] variant to the corresponding API call. Supports
//! status updates (open/close), comments, issue creation, and a custom
//! `create_pr` action.

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::adapter::{AdapterManifest, EgressAction, EgressResult};
use crate::domain::ports::adapter::EgressAdapter;

use super::client::GitHubClient;

/// Adapter that pushes actions to GitHub Issues (close/reopen, comments, creation).
///
/// Configuration is read from the [`AdapterManifest::config`] map:
/// - `owner` (required): repository owner (user or organisation).
/// - `repo` (required): repository name.
#[derive(Debug)]
pub struct GitHubEgressAdapter {
    /// The adapter manifest describing capabilities and config.
    manifest: AdapterManifest,
    /// Shared GitHub HTTP client.
    client: Arc<GitHubClient>,
}

impl GitHubEgressAdapter {
    /// Create a new egress adapter.
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

    /// Parse an issue number from an `external_id` string.
    ///
    /// Accepts plain numeric strings only (e.g., `"42"`).
    ///
    /// Returns `Err(ValidationFailed)` if the string is not a valid u64.
    pub(crate) fn parse_issue_number(external_id: &str) -> DomainResult<u64> {
        external_id.trim().parse::<u64>().map_err(|_| {
            DomainError::ValidationFailed(format!(
                "GitHub external_id must be a numeric issue number, got: '{external_id}'"
            ))
        })
    }

    /// Map a status string to a GitHub issue state.
    ///
    /// Returns `"open"` or `"closed"` depending on the semantic of the
    /// supplied status name. Unrecognised values default to `"open"`.
    pub fn to_github_state(new_status: &str) -> &'static str {
        match new_status.to_lowercase().as_str() {
            "close" | "closed" | "done" | "completed" | "resolved" | "wontfix" => "closed",
            _ => "open",
        }
    }

    /// Format the body for a pull request creation, optionally appending a
    /// `"Closes #N"` reference when `issue_number` is provided.
    ///
    /// - If `body` is empty and `issue_number` is `Some(n)`, returns `"Closes #n"`.
    /// - If `body` is non-empty and `issue_number` is `Some(n)`, appends
    ///   `"\n\nCloses #n"` to the body.
    /// - If `issue_number` is `None`, returns `body` unchanged.
    pub fn format_pr_body(body: &str, issue_number: Option<u64>) -> String {
        if let Some(n) = issue_number {
            if body.is_empty() {
                format!("Closes #{n}")
            } else {
                format!("{body}\n\nCloses #{n}")
            }
        } else {
            body.to_string()
        }
    }
}

#[async_trait]
impl EgressAdapter for GitHubEgressAdapter {
    fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    async fn execute(&self, action: &EgressAction) -> DomainResult<EgressResult> {
        // Resolve owner/repo once and return early with a clear error if missing.
        let owner = self.owner().ok_or_else(|| {
            DomainError::ValidationFailed(
                "GitHub Issues egress adapter config missing required 'owner'".to_string(),
            )
        })?;
        let repo = self.repo().ok_or_else(|| {
            DomainError::ValidationFailed(
                "GitHub Issues egress adapter config missing required 'repo'".to_string(),
            )
        })?;

        match action {
            EgressAction::UpdateStatus {
                external_id,
                new_status,
            } => {
                let issue_number = Self::parse_issue_number(external_id)?;

                let github_state = Self::to_github_state(new_status);
                tracing::info!(
                    owner = owner,
                    repo = repo,
                    issue = issue_number,
                    state = github_state,
                    "GitHub Issues: updating issue state"
                );

                self.client
                    .update_issue_state(owner, repo, issue_number, github_state)
                    .await?;

                Ok(EgressResult::ok_with_id(external_id))
            }

            EgressAction::PostComment { external_id, body } => {
                let issue_number = Self::parse_issue_number(external_id)?;

                tracing::info!(
                    owner = owner,
                    repo = repo,
                    issue = issue_number,
                    body_len = body.len(),
                    "GitHub Issues: posting comment"
                );

                self.client
                    .post_comment(owner, repo, issue_number, body)
                    .await?;

                Ok(EgressResult::ok_with_id(external_id))
            }

            EgressAction::CreateItem {
                title,
                description,
                fields,
            } => {
                // Optional labels from fields.
                let labels: Option<Vec<String>> = fields
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    });

                tracing::info!(
                    owner = owner,
                    repo = repo,
                    title = %title,
                    "GitHub Issues: creating issue"
                );

                let resp = self
                    .client
                    .create_issue(
                        owner,
                        repo,
                        title,
                        Some(description.as_str()),
                        labels,
                    )
                    .await?;

                let result = EgressResult::ok_with_id(resp.number.to_string())
                    .with_url(&resp.html_url);
                Ok(result)
            }

            EgressAction::AttachArtifact { external_id, .. } => {
                tracing::warn!(
                    issue = %external_id,
                    "GitHub Issues: AttachArtifact is not yet implemented"
                );
                Ok(EgressResult::fail(
                    "AttachArtifact is not yet supported by the GitHub Issues adapter",
                ))
            }

            EgressAction::Custom { action_name, params } => {
                if action_name == "create_pr" {
                    let title = params
                        .get("title")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            DomainError::ValidationFailed(
                                "GitHub Issues create_pr requires 'title' param".to_string(),
                            )
                        })?;
                    let body = params
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let head = params
                        .get("head")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            DomainError::ValidationFailed(
                                "GitHub Issues create_pr requires 'head' param".to_string(),
                            )
                        })?;
                    let base = params
                        .get("base")
                        .and_then(|v| v.as_str())
                        .unwrap_or("main");

                    // Optionally append a "Closes #N" reference to the body.
                    let issue_number: Option<u64> = params.get("issue_number").and_then(|v| {
                        if let Some(n) = v.as_u64() {
                            Some(n)
                        } else if let Some(s) = v.as_str() {
                            s.parse::<u64>().ok()
                        } else {
                            None
                        }
                    });

                    let full_body = Self::format_pr_body(body, issue_number);

                    tracing::info!(
                        owner = owner,
                        repo = repo,
                        title = %title,
                        head = %head,
                        base = %base,
                        closes_issue = ?issue_number,
                        "GitHub Issues: creating pull request"
                    );

                    let resp = self
                        .client
                        .create_pull_request(owner, repo, title, &full_body, head, base)
                        .await?;

                    let result = EgressResult::ok_with_id(resp.number.to_string())
                        .with_url(&resp.html_url);
                    Ok(result)
                } else {
                    tracing::warn!(
                        action = %action_name,
                        "GitHub Issues: unknown custom action"
                    );
                    Ok(EgressResult::fail(format!(
                        "Custom action '{action_name}' is not supported by the GitHub Issues adapter"
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        .with_capability(AdapterCapability::UpdateStatus)
        .with_capability(AdapterCapability::PostComment)
        .with_capability(AdapterCapability::CreateItem)
        .with_capability(AdapterCapability::Custom)
        .with_config("owner", serde_json::json!("my-org"))
        .with_config("repo", serde_json::json!("my-repo"))
    }

    // ── parse_issue_number ──────────────────────────────────────────────────

    #[test]
    fn test_parse_issue_number_valid() {
        assert_eq!(GitHubEgressAdapter::parse_issue_number("42").unwrap(), 42u64);
    }

    #[test]
    fn test_parse_issue_number_zero() {
        assert_eq!(GitHubEgressAdapter::parse_issue_number("0").unwrap(), 0u64);
    }

    #[test]
    fn test_parse_issue_number_invalid() {
        let result = GitHubEgressAdapter::parse_issue_number("abc");
        assert!(result.is_err());
        match result {
            Err(DomainError::ValidationFailed(msg)) => {
                assert!(msg.contains("abc"), "error message should mention the bad input, got: {msg}");
            }
            other => panic!("Expected ValidationFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_issue_number_empty() {
        let result = GitHubEgressAdapter::parse_issue_number("");
        assert!(result.is_err());
        match result {
            Err(DomainError::ValidationFailed(_)) => {}
            other => panic!("Expected ValidationFailed, got: {other:?}"),
        }
    }

    // ── to_github_state ─────────────────────────────────────────────────────

    #[test]
    fn test_to_github_state_open_variants() {
        assert_eq!(GitHubEgressAdapter::to_github_state("open"), "open");
        assert_eq!(GitHubEgressAdapter::to_github_state("reopen"), "open");
        assert_eq!(GitHubEgressAdapter::to_github_state("in_progress"), "open");
        assert_eq!(GitHubEgressAdapter::to_github_state("pending"), "open");
        // Unknown → open
        assert_eq!(GitHubEgressAdapter::to_github_state("anything_else"), "open");
    }

    #[test]
    fn test_to_github_state_closed_variants() {
        assert_eq!(GitHubEgressAdapter::to_github_state("close"), "closed");
        assert_eq!(GitHubEgressAdapter::to_github_state("closed"), "closed");
        assert_eq!(GitHubEgressAdapter::to_github_state("done"), "closed");
        assert_eq!(GitHubEgressAdapter::to_github_state("completed"), "closed");
        assert_eq!(GitHubEgressAdapter::to_github_state("resolved"), "closed");
        assert_eq!(GitHubEgressAdapter::to_github_state("wontfix"), "closed");
    }

    #[test]
    fn test_to_github_state_case_insensitive() {
        assert_eq!(GitHubEgressAdapter::to_github_state("DONE"), "closed");
        assert_eq!(GitHubEgressAdapter::to_github_state("Closed"), "closed");
    }

    // ── missing owner/repo config ───────────────────────────────────────────

    #[test]
    fn test_missing_owner_config() {
        let manifest = AdapterManifest::new(
            "github-issues",
            AdapterType::Native,
            AdapterDirection::Egress,
        )
        .with_capability(AdapterCapability::UpdateStatus)
        .with_config("repo", serde_json::json!("my-repo"));

        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubEgressAdapter::new(manifest, client);
        assert!(adapter.owner().is_none());
    }

    #[test]
    fn test_missing_repo_config() {
        let manifest = AdapterManifest::new(
            "github-issues",
            AdapterType::Native,
            AdapterDirection::Egress,
        )
        .with_capability(AdapterCapability::UpdateStatus)
        .with_config("owner", serde_json::json!("my-org"));

        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubEgressAdapter::new(manifest, client);
        assert!(adapter.repo().is_none());
    }

    #[test]
    fn test_egress_adapter_manifest() {
        let manifest = test_manifest();
        let client = Arc::new(GitHubClient::new("token".to_string()));
        let adapter = GitHubEgressAdapter::new(manifest.clone(), client);

        assert_eq!(adapter.manifest().name, "github-issues");
        assert!(adapter.manifest().has_capability(AdapterCapability::UpdateStatus));
        assert!(adapter.manifest().has_capability(AdapterCapability::PostComment));
    }

    // ── format_pr_body / create_pr body formatting ──────────────────────────

    #[test]
    fn test_format_pr_body_no_issue_number() {
        let result = GitHubEgressAdapter::format_pr_body("My PR description", None);
        assert_eq!(result, "My PR description");
    }

    #[test]
    fn test_format_pr_body_empty_body_with_issue_number() {
        let result = GitHubEgressAdapter::format_pr_body("", Some(42));
        assert_eq!(result, "Closes #42");
    }

    #[test]
    fn test_format_pr_body_non_empty_body_with_issue_number() {
        let result = GitHubEgressAdapter::format_pr_body("Implements the new feature", Some(7));
        assert_eq!(result, "Implements the new feature\n\nCloses #7");
    }

    #[test]
    fn test_format_pr_body_no_issue_number_empty_body() {
        // Without an issue number and with an empty body, body is returned as-is (empty).
        let result = GitHubEgressAdapter::format_pr_body("", None);
        assert_eq!(result, "");
    }

    #[test]
    fn test_create_pr_issue_number_as_json_string() {
        // Simulate how the issue_number param is resolved from a JSON string value,
        // matching the logic in execute() before delegating to format_pr_body.
        let v = serde_json::json!("15");
        let parsed: Option<u64> = {
            if let Some(n) = v.as_u64() {
                Some(n)
            } else if let Some(s) = v.as_str() {
                s.parse::<u64>().ok()
            } else {
                None
            }
        };
        assert_eq!(parsed, Some(15));
        let body = GitHubEgressAdapter::format_pr_body("Fix things", parsed);
        assert_eq!(body, "Fix things\n\nCloses #15");
    }

    #[test]
    fn test_create_pr_issue_number_as_json_number() {
        // Simulate how the issue_number param is resolved from a JSON numeric value.
        let v = serde_json::json!(23u64);
        let parsed: Option<u64> = {
            if let Some(n) = v.as_u64() {
                Some(n)
            } else if let Some(s) = v.as_str() {
                s.parse::<u64>().ok()
            } else {
                None
            }
        };
        assert_eq!(parsed, Some(23));
        let body = GitHubEgressAdapter::format_pr_body("", parsed);
        assert_eq!(body, "Closes #23");
    }
}

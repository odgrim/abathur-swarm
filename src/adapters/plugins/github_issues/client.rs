//! GitHub HTTP client with rate limiting.
//!
//! Wraps the GitHub REST API v3, providing typed methods for the
//! operations used by the ingestion and egress adapters. Includes
//! a token-bucket rate limiter to stay within the 5 000 req/hour
//! authenticated API limit.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::sync::Mutex;

use crate::domain::errors::{DomainError, DomainResult};

use super::models::{
    GitHubCommentRequest, GitHubCreateIssueRequest, GitHubCreateIssueResponse, GitHubIssue,
    GitHubIssueUpdateRequest,
};

/// Base URL for the GitHub REST API v3.
const GITHUB_API_BASE: &str = "https://api.github.com";

/// Token-bucket rate limiter.
///
/// Allows up to `capacity` requests per `window`. When the bucket is
/// exhausted, [`acquire`](RateLimiter::acquire) sleeps until the window
/// resets and a token becomes available.
#[derive(Debug)]
pub struct RateLimiter {
    /// Maximum tokens in the bucket.
    capacity: u32,
    /// Current available tokens.
    tokens: u32,
    /// Duration of the refill window.
    window: Duration,
    /// When the current window started.
    window_start: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter with the given capacity and window.
    pub fn new(capacity: u32, window: Duration) -> Self {
        Self {
            capacity,
            tokens: capacity,
            window,
            window_start: Instant::now(),
        }
    }

    /// Acquire a single token, sleeping if necessary.
    ///
    /// If the current window has elapsed, the bucket is refilled.
    /// If no tokens are available, this method sleeps until the
    /// window resets.
    pub async fn acquire(&mut self) {
        let elapsed = self.window_start.elapsed();
        if elapsed >= self.window {
            // Refill the bucket and start a new window.
            self.tokens = self.capacity;
            self.window_start = Instant::now();
        }

        if self.tokens > 0 {
            self.tokens -= 1;
        } else {
            // Sleep until the window resets.
            let remaining = self.window.saturating_sub(elapsed);
            tracing::warn!(
                sleep_ms = remaining.as_millis() as u64,
                "GitHub rate limit reached, sleeping"
            );
            tokio::time::sleep(remaining).await;
            // After sleeping, refill and consume one token.
            self.tokens = self.capacity - 1;
            self.window_start = Instant::now();
        }
    }
}

/// HTTP client for the GitHub REST API v3.
///
/// All methods return [`DomainResult`] and map HTTP / network errors
/// to [`DomainError::ExecutionFailed`].
#[derive(Debug, Clone)]
pub struct GitHubClient {
    /// The underlying HTTP client.
    http: Client,
    /// GitHub personal access token or fine-grained token.
    token: String,
    /// Shared rate limiter (5 000 req/hr for authenticated requests).
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

impl GitHubClient {
    /// Create a new client with the given token.
    pub fn new(token: String) -> Self {
        // GitHub allows 5 000 authenticated requests per hour.
        let rate_limiter = RateLimiter::new(5_000, Duration::from_secs(3_600));
        Self {
            http: Client::new(),
            token,
            rate_limiter: Arc::new(Mutex::new(rate_limiter)),
        }
    }

    /// Create a client by reading the `GITHUB_TOKEN` environment variable.
    ///
    /// Returns `Err` if the variable is not set or is empty.
    pub fn from_env() -> Result<Self, String> {
        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| "GITHUB_TOKEN environment variable is not set".to_string())?;
        if token.is_empty() {
            return Err("GITHUB_TOKEN environment variable is empty".to_string());
        }
        Ok(Self::new(token))
    }

    /// Acquire a rate-limit token and build an authorized request.
    async fn rate_limited_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> reqwest::RequestBuilder {
        self.rate_limiter.lock().await.acquire().await;
        self.http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "abathur-swarm")
    }

    /// List issues from a repository.
    ///
    /// `state` must be `"open"`, `"closed"`, or `"all"`.
    /// If `since` is provided it must be an ISO 8601 timestamp; only
    /// issues updated at or after that time are returned.
    ///
    /// Note: GitHub's `/issues` endpoint also returns pull requests.
    /// Callers are responsible for filtering them out via the
    /// `pull_request` field.
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        state: &str,
        since: Option<&str>,
    ) -> DomainResult<Vec<GitHubIssue>> {
        let mut url = format!(
            "{}/repos/{}/{}/issues?state={}&per_page=100",
            GITHUB_API_BASE, owner, repo, state
        );
        if let Some(since_ts) = since {
            url.push_str(&format!("&since={since_ts}"));
        }

        let req = self
            .rate_limited_request(reqwest::Method::GET, &url)
            .await;

        let resp = req.send().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("GitHub list_issues request failed: {e}"))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "GitHub list_issues returned {status}: {body}"
            )));
        }

        resp.json::<Vec<GitHubIssue>>().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("GitHub list_issues parse failed: {e}"))
        })
    }

    /// Update the state of an issue (`"open"` or `"closed"`).
    pub async fn update_issue_state(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        state: &str,
    ) -> DomainResult<()> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}",
            GITHUB_API_BASE, owner, repo, issue_number
        );
        let body = GitHubIssueUpdateRequest {
            state: state.to_string(),
        };

        let resp = self
            .rate_limited_request(reqwest::Method::PATCH, &url)
            .await
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!(
                    "GitHub update_issue_state request failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "GitHub update_issue_state returned {status}: {body_text}"
            )));
        }

        Ok(())
    }

    /// Post a comment on an issue.
    pub async fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        comment: &str,
    ) -> DomainResult<()> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}/comments",
            GITHUB_API_BASE, owner, repo, issue_number
        );
        let body = GitHubCommentRequest {
            body: comment.to_string(),
        };

        let resp = self
            .rate_limited_request(reqwest::Method::POST, &url)
            .await
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!("GitHub post_comment request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "GitHub post_comment returned {status}: {body_text}"
            )));
        }

        Ok(())
    }

    /// Create a new issue in a repository.
    ///
    /// Returns the created issue's number and URL.
    pub async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: Option<&str>,
        labels: Option<Vec<String>>,
    ) -> DomainResult<GitHubCreateIssueResponse> {
        let url = format!("{}/repos/{}/{}/issues", GITHUB_API_BASE, owner, repo);
        let req_body = GitHubCreateIssueRequest {
            title: title.to_string(),
            body: body.map(str::to_string),
            labels,
        };

        let resp = self
            .rate_limited_request(reqwest::Method::POST, &url)
            .await
            .json(&req_body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!("GitHub create_issue request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "GitHub create_issue returned {status}: {body_text}"
            )));
        }

        resp.json::<GitHubCreateIssueResponse>().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("GitHub create_issue parse failed: {e}"))
        })
    }

    /// Create a pull request in a repository.
    ///
    /// Returns the created PR's number and URL.
    pub async fn create_pull_request(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> DomainResult<GitHubCreateIssueResponse> {
        let url = format!("{}/repos/{}/{}/pulls", GITHUB_API_BASE, owner, repo);
        let req_body = serde_json::json!({
            "title": title,
            "body": body,
            "head": head,
            "base": base,
        });

        let resp = self
            .rate_limited_request(reqwest::Method::POST, &url)
            .await
            .json(&req_body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!(
                    "GitHub create_pull_request request failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "GitHub create_pull_request returned {status}: {body_text}"
            )));
        }

        resp.json::<GitHubCreateIssueResponse>().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("GitHub create_pull_request parse failed: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_creation() {
        let rl = RateLimiter::new(5_000, Duration::from_secs(3_600));
        assert_eq!(rl.capacity, 5_000);
        assert_eq!(rl.tokens, 5_000);
    }

    #[tokio::test]
    async fn test_rate_limiter_acquire_decrements_tokens() {
        let mut rl = RateLimiter::new(5, Duration::from_secs(60));
        rl.acquire().await;
        assert_eq!(rl.tokens, 4);
        rl.acquire().await;
        assert_eq!(rl.tokens, 3);
    }

    #[test]
    fn test_client_from_env_missing() {
        // Ensure the env var is not set for this test.
        // SAFETY: test-only; tests are run single-threaded or with isolated state.
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        let result = GitHubClient::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not set"));
    }

    #[test]
    fn test_client_new() {
        let client = GitHubClient::new("ghp_test_token".to_string());
        assert_eq!(client.token, "ghp_test_token");
    }
}

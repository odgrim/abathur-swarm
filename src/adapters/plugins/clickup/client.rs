//! ClickUp HTTP client with rate limiting.
//!
//! Wraps the ClickUp REST API v2, providing typed methods for the
//! operations used by the ingestion and egress adapters. Includes
//! a token-bucket rate limiter to stay within the 100 req/min API limit.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::sync::Mutex;

use crate::domain::errors::{DomainError, DomainResult};

use super::models::{
    ClickUpCommentRequest, ClickUpTaskResponse, ClickUpTasksResponse,
};

/// Base URL for the ClickUp API v2.
const CLICKUP_API_BASE: &str = "https://api.clickup.com/api/v2";

/// Token-bucket rate limiter.
///
/// Allows up to `capacity` requests per `window`. When the bucket is
/// exhausted, [`acquire`](RateLimiter::acquire) sleeps until a token
/// becomes available.
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
                "ClickUp rate limit reached, sleeping"
            );
            tokio::time::sleep(remaining).await;
            // After sleeping, refill and consume one token.
            self.tokens = self.capacity - 1;
            self.window_start = Instant::now();
        }
    }
}

/// HTTP client for the ClickUp REST API v2.
///
/// All methods return [`DomainResult`] and map HTTP / network errors
/// to [`DomainError::ExecutionFailed`].
#[derive(Debug, Clone)]
pub struct ClickUpClient {
    /// The underlying HTTP client.
    http: Client,
    /// ClickUp personal API token.
    api_key: String,
    /// Shared rate limiter.
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

impl ClickUpClient {
    /// Create a new client with the given API key.
    pub fn new(api_key: String) -> Self {
        let rate_limiter = RateLimiter::new(100, Duration::from_secs(60));
        Self {
            http: Client::new(),
            api_key,
            rate_limiter: Arc::new(Mutex::new(rate_limiter)),
        }
    }

    /// Create a client by reading the `CLICKUP_API_KEY` environment variable.
    ///
    /// Returns `Err` if the variable is not set or is empty.
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("CLICKUP_API_KEY")
            .map_err(|_| "CLICKUP_API_KEY environment variable is not set".to_string())?;
        if api_key.is_empty() {
            return Err("CLICKUP_API_KEY environment variable is empty".to_string());
        }
        Ok(Self::new(api_key))
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
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
    }

    /// Fetch tasks from a ClickUp list.
    ///
    /// If `updated_after_ms` is provided, only tasks updated after that
    /// Unix timestamp (milliseconds) are returned.
    pub async fn get_tasks(
        &self,
        list_id: &str,
        updated_after_ms: Option<i64>,
    ) -> DomainResult<ClickUpTasksResponse> {
        let url = match updated_after_ms {
            Some(ts) => format!(
                "{}/list/{}/task?date_updated_gt={}",
                CLICKUP_API_BASE, list_id, ts
            ),
            None => format!("{}/list/{}/task", CLICKUP_API_BASE, list_id),
        };
        let req = self
            .rate_limited_request(reqwest::Method::GET, &url)
            .await;

        let resp = req.send().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("ClickUp get_tasks request failed: {e}"))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "ClickUp get_tasks returned {status}: {body}"
            )));
        }

        resp.json::<ClickUpTasksResponse>().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("ClickUp get_tasks parse failed: {e}"))
        })
    }

    /// Update the status of a ClickUp task.
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: &str,
    ) -> DomainResult<()> {
        let url = format!("{}/task/{}", CLICKUP_API_BASE, task_id);
        let body = serde_json::json!({ "status": status });

        let resp = self
            .rate_limited_request(reqwest::Method::PUT, &url)
            .await
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!(
                    "ClickUp update_task_status request failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status_code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "ClickUp update_task_status returned {status_code}: {body}"
            )));
        }

        Ok(())
    }

    /// Post a comment on a ClickUp task.
    pub async fn post_comment(
        &self,
        task_id: &str,
        comment: &str,
    ) -> DomainResult<()> {
        let url = format!("{}/task/{}/comment", CLICKUP_API_BASE, task_id);
        let body = ClickUpCommentRequest {
            comment_text: comment.to_string(),
        };

        let resp = self
            .rate_limited_request(reqwest::Method::POST, &url)
            .await
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!(
                    "ClickUp post_comment request failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "ClickUp post_comment returned {status}: {body}"
            )));
        }

        Ok(())
    }

    /// Create a new task in a ClickUp list.
    ///
    /// Returns the created task's ID and URL.
    pub async fn create_task(
        &self,
        list_id: &str,
        name: &str,
        description: &str,
    ) -> DomainResult<ClickUpTaskResponse> {
        let url = format!("{}/list/{}/task", CLICKUP_API_BASE, list_id);
        let body = serde_json::json!({
            "name": name,
            "description": description,
        });

        let resp = self
            .rate_limited_request(reqwest::Method::POST, &url)
            .await
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                DomainError::ExecutionFailed(format!(
                    "ClickUp create_task request failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::ExecutionFailed(format!(
                "ClickUp create_task returned {status}: {body}"
            )));
        }

        resp.json::<ClickUpTaskResponse>().await.map_err(|e| {
            DomainError::ExecutionFailed(format!("ClickUp create_task parse failed: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_creation() {
        let rl = RateLimiter::new(100, Duration::from_secs(60));
        assert_eq!(rl.capacity, 100);
        assert_eq!(rl.tokens, 100);
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
        unsafe { std::env::remove_var("CLICKUP_API_KEY") };
        let result = ClickUpClient::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not set"));
    }

    #[test]
    fn test_client_new() {
        let client = ClickUpClient::new("test-key".to_string());
        assert_eq!(client.api_key, "test-key");
    }
}

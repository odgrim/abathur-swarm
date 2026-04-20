//! ClickUp API v2 client with retry logic.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use std::time::Duration;
use tracing::warn;

use super::models::*;

/// Trait for ClickUp API operations, enabling test mocking.
#[async_trait]
pub trait ClickUpApi: Send + Sync {
    async fn create_task(&self, list_id: &str, req: &CreateTaskRequest) -> Result<CreateTaskResponse>;
    async fn get_task(&self, task_id: &str) -> Result<Option<ClickUpTask>>;
    async fn get_comments(&self, task_id: &str) -> Result<Vec<ClickUpComment>>;
}

/// Production ClickUp API client using reqwest.
pub struct ClickUpClient {
    client: reqwest::Client,
}

impl ClickUpClient {
    pub fn new(api_token: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&api_token)
                        .expect("Invalid API token"),
                );
                headers.insert(
                    reqwest::header::CONTENT_TYPE,
                    reqwest::header::HeaderValue::from_static("application/json"),
                );
                headers
            })
            .build()
            .expect("Failed to build ClickUp HTTP client");
        Self { client }
    }
}

/// Retry a future-producing closure with exponential backoff on 429/5xx.
async fn retry_with_backoff<F, Fut, T>(description: &str, f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    retry_with_backoff_opts(description, 6, Duration::from_secs(1), Duration::from_secs(60), true, f).await
}

/// Internal implementation allowing test-time customisation of attempt count and sleeping.
async fn retry_with_backoff_opts<F, Fut, T>(
    description: &str,
    max_attempts: u32,
    initial_delay: Duration,
    max_delay: Duration,
    sleep_between: bool,
    mut f: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut delay = initial_delay;
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 1..=max_attempts {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                let should_retry = e.downcast_ref::<RetryableError>().is_some();
                if !should_retry {
                    return Err(e);
                }
                if attempt == max_attempts {
                    last_error = Some(e);
                    break;
                }
                warn!("{description}: attempt {attempt} failed, retrying in {delay:?}: {e}");
                if sleep_between {
                    tokio::time::sleep(delay).await;
                }
                delay = (delay * 2).min(max_delay);
                last_error = Some(e);
            }
        }
    }

    let err = last_error.unwrap_or_else(|| anyhow::anyhow!("{description}: no attempts made"));
    Err(err).context(format!("{description}: all {max_attempts} attempts failed"))
}

/// Marker error type for retryable HTTP errors (429, 5xx).
#[derive(Debug, thiserror::Error)]
#[error("retryable HTTP error: status {status}")]
struct RetryableError {
    status: u16,
}

/// Check a reqwest response, returning RetryableError for 429/5xx.
async fn check_response(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    if status.as_u16() == 429 || status.is_server_error() {
        bail!(RetryableError { status: status.as_u16() });
    }
    let body = resp.text().await.unwrap_or_default();
    bail!("ClickUp API error: HTTP {status}: {body}");
}

#[async_trait]
impl ClickUpApi for ClickUpClient {
    async fn create_task(&self, list_id: &str, req: &CreateTaskRequest) -> Result<CreateTaskResponse> {
        let url = format!("https://api.clickup.com/api/v2/list/{list_id}/task");
        let client = &self.client;
        retry_with_backoff("create_task", || async {
            let resp = client
                .post(&url)
                .json(req)
                .send()
                .await
                .context("ClickUp create_task request failed")?;
            let resp = check_response(resp).await?;
            resp.json::<CreateTaskResponse>()
                .await
                .context("Failed to parse create_task response")
        })
        .await
    }

    async fn get_task(&self, task_id: &str) -> Result<Option<ClickUpTask>> {
        let url = format!("https://api.clickup.com/api/v2/task/{task_id}");
        let client = &self.client;
        retry_with_backoff("get_task", || async {
            let resp = client
                .get(&url)
                .send()
                .await
                .context("ClickUp get_task request failed")?;
            if resp.status().as_u16() == 404 {
                return Ok(None);
            }
            let resp = check_response(resp).await?;
            let task = resp
                .json::<ClickUpTask>()
                .await
                .context("Failed to parse get_task response")?;
            Ok(Some(task))
        })
        .await
    }

    async fn get_comments(&self, task_id: &str) -> Result<Vec<ClickUpComment>> {
        let url = format!("https://api.clickup.com/api/v2/task/{task_id}/comment");
        let client = &self.client;
        retry_with_backoff("get_comments", || async {
            let resp = client
                .get(&url)
                .send()
                .await
                .context("ClickUp get_comments request failed")?;
            let resp = check_response(resp).await?;
            let body = resp
                .json::<ClickUpCommentsResponse>()
                .await
                .context("Failed to parse get_comments response")?;
            Ok(body.comments)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_exhaustion_returns_last_error_instead_of_panicking() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = calls.clone();

        let result: Result<()> = retry_with_backoff_opts(
            "test_op",
            2,
            Duration::from_millis(0),
            Duration::from_millis(0),
            false,
            move || {
                let calls = calls_clone.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow::Error::new(RetryableError { status: 503 }))
                }
            },
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 2, "should attempt max_attempts times");
        let err = result.expect_err("exhausted retries must return Err, not panic or Ok");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("all 2 attempts failed"),
            "error should be wrapped with exhaustion context, got: {msg}"
        );
        assert!(
            err.chain().any(|e| e.downcast_ref::<RetryableError>().is_some()),
            "original RetryableError should be preserved in the error chain, got: {msg}"
        );
    }

    #[tokio::test]
    async fn non_retryable_error_short_circuits() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = calls.clone();

        let result: Result<()> = retry_with_backoff_opts(
            "test_op",
            5,
            Duration::from_millis(0),
            Duration::from_millis(0),
            false,
            move || {
                let calls = calls_clone.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow::anyhow!("permanent failure"))
                }
            },
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "non-retryable errors should not retry");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn successful_first_attempt_does_not_retry() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = calls.clone();

        let result: Result<u32> = retry_with_backoff_opts(
            "test_op",
            5,
            Duration::from_millis(0),
            Duration::from_millis(0),
            false,
            move || {
                let calls = calls_clone.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(42)
                }
            },
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "success on first attempt must not retry");
        assert_eq!(result.expect("success path returns Ok"), 42);
    }

    #[tokio::test]
    async fn permanent_http_error_returns_immediately_after_one_attempt() {
        // A non-retryable HTTP error (e.g. 400/404) is surfaced as a plain
        // anyhow error from check_response — verify the retry layer treats
        // it as terminal and does not loop.
        let calls = Arc::new(AtomicU32::new(0));
        let calls_clone = calls.clone();

        let result: Result<()> = retry_with_backoff_opts(
            "test_op",
            5,
            Duration::from_millis(0),
            Duration::from_millis(0),
            false,
            move || {
                let calls = calls_clone.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    // Mirrors the bail!() in check_response for a 4xx body.
                    Err(anyhow::anyhow!("ClickUp API error: HTTP 400 Bad Request: bad input"))
                }
            },
        )
        .await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "permanent (non-retryable) HTTP error must not be retried"
        );
        let err = result.expect_err("permanent error must surface as Err");
        let msg = format!("{err:#}");
        assert!(msg.contains("HTTP 400"), "error must propagate the original message verbatim, got: {msg}");
        assert!(
            !msg.contains("attempts failed"),
            "non-retryable errors should NOT be wrapped with the exhaustion context, got: {msg}"
        );
    }
}

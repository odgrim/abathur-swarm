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
async fn retry_with_backoff<F, Fut, T>(description: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let max_attempts = 6;
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(60);

    for attempt in 1..=max_attempts {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempt == max_attempts {
                    return Err(e).context(format!("{description}: all {max_attempts} attempts failed"));
                }
                let should_retry = e
                    .downcast_ref::<RetryableError>()
                    .is_some();
                if !should_retry {
                    return Err(e);
                }
                warn!("{description}: attempt {attempt} failed, retrying in {delay:?}: {e}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
            }
        }
    }
    unreachable!()
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

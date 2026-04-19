//! A2A wire protocol HTTP client.
//!
//! Implements the standard A2A JSON-RPC 2.0 methods over HTTP(S)
//! with SSE streaming support.

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::domain::models::a2a_protocol::{
    A2AJsonRpcRequest, A2AJsonRpcResponse, A2AStandardAgentCard, A2AStreamEvent, A2ATask,
    TaskQueryParams, TaskSendParams,
};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum A2AWireError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("A2A protocol error {code}: {message}")]
    Protocol {
        code: i32,
        message: String,
        data: Option<serde_json::Value>,
    },

    #[error("Discovery failed: {0}")]
    Discovery(String),

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Request timed out")]
    Timeout,

    #[error("Client build failed: {0}")]
    BuildError(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Wire-level A2A client interface.
///
/// Abstracts the HTTP transport so the federation service can be tested
/// with mock implementations.
#[async_trait]
pub trait A2AClient: Send + Sync {
    /// Fetch agent card from `/.well-known/agent.json`.
    async fn discover(&self, base_url: &str) -> Result<A2AStandardAgentCard, A2AWireError>;

    /// Send a message (`tasks/send`) — returns the created/updated task.
    async fn send_message(
        &self,
        url: &str,
        params: TaskSendParams,
    ) -> Result<A2ATask, A2AWireError>;

    /// Send a streaming message (`tasks/sendSubscribe`) — returns an SSE event stream.
    async fn send_streaming(
        &self,
        url: &str,
        params: TaskSendParams,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
        A2AWireError,
    >;

    /// Get task status (`tasks/get`).
    async fn get_task(
        &self,
        url: &str,
        task_id: &str,
        history_length: Option<u32>,
    ) -> Result<A2ATask, A2AWireError>;

    /// Cancel a task (`tasks/cancel`).
    async fn cancel_task(&self, url: &str, task_id: &str) -> Result<A2ATask, A2AWireError>;

    /// Subscribe to task updates (`tasks/resubscribe`) — returns an SSE event stream.
    async fn subscribe_to_task(
        &self,
        url: &str,
        task_id: &str,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
        A2AWireError,
    >;
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

/// HTTP-based A2A client using reqwest.
#[derive(Clone)]
pub struct HttpA2AClient {
    client: reqwest::Client,
    a2a_version: String,
}

impl HttpA2AClient {
    pub fn new() -> Result<Self, A2AWireError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| A2AWireError::BuildError(e.to_string()))?,
            a2a_version: "0.3".to_string(),
        })
    }

    /// Create a new client, panicking on failure.
    ///
    /// Use this only in contexts where client construction is infallible
    /// in practice (e.g., tests, static initialization).
    pub fn new_or_panic() -> Self {
        Self::new().expect("Failed to build HTTP client")
    }

    pub fn with_timeout(timeout: Duration) -> Result<Self, A2AWireError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|e| A2AWireError::BuildError(e.to_string()))?,
            a2a_version: "0.3".to_string(),
        })
    }

    /// Build a JSON-RPC 2.0 request body.
    fn build_jsonrpc(&self, method: &str, params: serde_json::Value) -> A2AJsonRpcRequest {
        A2AJsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            id: serde_json::Value::String(Uuid::new_v4().to_string()),
            params: Some(params),
        }
    }

    /// Send a JSON-RPC request and parse the result.
    async fn rpc_call<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, A2AWireError> {
        let request = self.build_jsonrpc(method, params);
        debug!(method, url, "A2A RPC call");

        let resp = self
            .client
            .post(url)
            .header("A2A-Version", &self.a2a_version)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = resp.status();
        let body: A2AJsonRpcResponse = resp.json().await?;

        if let Some(err) = body.error {
            return Err(A2AWireError::Protocol {
                code: err.code,
                message: err.message,
                data: err.data,
            });
        }

        match body.result {
            Some(serde_json::Value::Null) => Err(A2AWireError::Protocol {
                code: -32603,
                message: format!(
                    "Server returned null result for {} (HTTP {})",
                    method, status
                ),
                data: None,
            }),
            Some(result) => serde_json::from_value(result).map_err(A2AWireError::Json),
            None => Err(A2AWireError::Protocol {
                code: -32603,
                message: format!(
                    "Response missing result field for {} (HTTP {})",
                    method, status
                ),
                data: None,
            }),
        }
    }

    /// Send a JSON-RPC request expecting an SSE stream response.
    async fn rpc_stream(
        &self,
        url: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
        A2AWireError,
    > {
        let request = self.build_jsonrpc(method, params);
        debug!(method, url, "A2A RPC stream");

        let resp = self
            .client
            .post(url)
            .header("A2A-Version", &self.a2a_version)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&request)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(A2AWireError::Protocol {
                code: status.as_u16() as i32,
                message: format!("HTTP {}: {}", status, body),
                data: None,
            });
        }

        Ok(parse_sse_stream(resp))
    }
}

impl Default for HttpA2AClient {
    fn default() -> Self {
        Self::new().expect("Failed to build default HTTP client")
    }
}

#[async_trait]
impl A2AClient for HttpA2AClient {
    async fn discover(&self, base_url: &str) -> Result<A2AStandardAgentCard, A2AWireError> {
        let url = format!("{}/.well-known/agent.json", base_url.trim_end_matches('/'));
        debug!(url, "A2A discover");

        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| A2AWireError::Discovery(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(A2AWireError::Discovery(format!("HTTP {}", resp.status())));
        }

        resp.json()
            .await
            .map_err(|e| A2AWireError::Discovery(e.to_string()))
    }

    async fn send_message(
        &self,
        url: &str,
        params: TaskSendParams,
    ) -> Result<A2ATask, A2AWireError> {
        self.rpc_call(url, "tasks/send", serde_json::to_value(&params)?)
            .await
    }

    async fn send_streaming(
        &self,
        url: &str,
        params: TaskSendParams,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
        A2AWireError,
    > {
        self.rpc_stream(url, "tasks/sendSubscribe", serde_json::to_value(&params)?)
            .await
    }

    async fn get_task(
        &self,
        url: &str,
        task_id: &str,
        history_length: Option<u32>,
    ) -> Result<A2ATask, A2AWireError> {
        let params = TaskQueryParams {
            id: task_id.to_string(),
            history_length,
        };
        self.rpc_call(url, "tasks/get", serde_json::to_value(&params)?)
            .await
    }

    async fn cancel_task(&self, url: &str, task_id: &str) -> Result<A2ATask, A2AWireError> {
        let params = serde_json::json!({ "id": task_id });
        self.rpc_call(url, "tasks/cancel", params).await
    }

    async fn subscribe_to_task(
        &self,
        url: &str,
        task_id: &str,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>>,
        A2AWireError,
    > {
        let params = serde_json::json!({ "id": task_id });
        self.rpc_stream(url, "tasks/resubscribe", params).await
    }
}

// ---------------------------------------------------------------------------
// SSE parsing
// ---------------------------------------------------------------------------

/// Parse an SSE stream from a reqwest response into A2AStreamEvents.
fn parse_sse_stream(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<A2AStreamEvent, A2AWireError>> + Send>> {
    let byte_stream = response.bytes_stream();

    // State: accumulated data lines for the current event
    let initial_state = (byte_stream, String::new());

    let event_stream = stream::unfold(initial_state, |(mut byte_stream, mut buffer)| async move {
        loop {
            match byte_stream.next().await {
                Some(Ok(chunk)) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));

                    // Process the next complete event in the buffer
                    if let Some(event) = try_extract_sse_event(&mut buffer) {
                        match serde_json::from_str::<A2AStreamEvent>(&event) {
                            Ok(parsed) => {
                                return Some((Ok(parsed), (byte_stream, buffer)));
                            }
                            Err(e) => {
                                warn!(error = %e, data = %event, "Failed to parse SSE event");
                                return Some((
                                    Err(A2AWireError::Stream(format!(
                                        "Failed to parse SSE event: {}",
                                        e
                                    ))),
                                    (byte_stream, buffer),
                                ));
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    return Some((Err(A2AWireError::Http(e)), (byte_stream, buffer)));
                }
                None => return None,
            }
        }
    });

    Box::pin(event_stream)
}

/// Try to extract a complete SSE event from the buffer.
///
/// SSE events are delimited by blank lines. Data lines start with "data: ".
/// Returns the accumulated data payload if a complete event is found.
fn try_extract_sse_event(buffer: &mut String) -> Option<String> {
    // Look for a double newline (event boundary)
    let boundary = if let Some(pos) = buffer.find("\n\n") {
        pos
    } else if let Some(pos) = buffer.find("\r\n\r\n") {
        pos
    } else {
        return None;
    };

    let event_block = buffer[..boundary].to_string();
    // Remove the event block and delimiter from the buffer
    let skip = if buffer[boundary..].starts_with("\r\n\r\n") {
        boundary + 4
    } else {
        boundary + 2
    };
    *buffer = buffer[skip..].to_string();

    // Extract data lines
    let mut data_parts = Vec::new();
    for line in event_block.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            data_parts.push(data);
        } else if let Some(data) = line.strip_prefix("data:") {
            data_parts.push(data);
        }
    }

    if data_parts.is_empty() {
        return None;
    }

    Some(data_parts.join("\n"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_a2a_client_new() {
        let client = HttpA2AClient::new().unwrap();
        assert_eq!(client.a2a_version, "0.3");
    }

    #[test]
    fn test_jsonrpc_request_serialization() {
        let client = HttpA2AClient::new().unwrap();
        let req = client.build_jsonrpc("tasks/send", serde_json::json!({"key": "value"}));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "tasks/send");

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"tasks/send\""));
    }

    #[test]
    fn test_sse_event_parsing() {
        let mut buffer = "data: {\"type\":\"statusUpdate\",\"taskId\":\"t1\",\"status\":{\"state\":\"working\"},\"final\":false}\n\ndata: leftover".to_string();

        let event = try_extract_sse_event(&mut buffer);
        assert!(event.is_some());
        let data = event.unwrap();
        assert!(data.contains("statusUpdate"));
        assert_eq!(buffer, "data: leftover");
    }

    #[test]
    fn test_sse_no_complete_event() {
        let mut buffer = "data: partial".to_string();
        let event = try_extract_sse_event(&mut buffer);
        assert!(event.is_none());
        assert_eq!(buffer, "data: partial");
    }

    #[test]
    fn test_protocol_error() {
        let err = A2AWireError::Protocol {
            code: -32001,
            message: "Task not found".to_string(),
            data: None,
        };
        assert!(err.to_string().contains("-32001"));
        assert!(err.to_string().contains("Task not found"));
    }
}

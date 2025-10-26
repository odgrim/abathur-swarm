<<<<<<< HEAD
use super::errors::ClaudeApiError;
use super::types::{ContentBlock, Usage};
=======
//! Server-Sent Events (SSE) streaming support for Claude API
//!
//! This module provides parsing and streaming functionality for Claude API's streaming responses.
//! The API returns Server-Sent Events with the following event types:
//! - message_start: Initial message metadata
//! - content_block_start: Start of a content block
//! - content_block_delta: Incremental content update
//! - content_block_stop: End of a content block
//! - message_delta: Message-level updates (e.g., stop_reason)
//! - message_stop: End of message

use anyhow::{anyhow, Result};
>>>>>>> task_sse-streaming-parser_20251025-210007
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context, Poll};
<<<<<<< HEAD
use tracing::{debug, warn};

/// Streaming message chunk from Claude API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    /// Message start event
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },

    /// Content block start
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },

    /// Content block delta (incremental update)
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: Delta },

    /// Content block stop
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },

    /// Message delta (usage updates)
    #[serde(rename = "message_delta")]
    MessageDelta { delta: MessageDeltaData },

    /// Message stop event
    #[serde(rename = "message_stop")]
    MessageStop,

    /// Ping event (keepalive)
    #[serde(rename = "ping")]
    Ping,

    /// Error event
    #[serde(rename = "error")]
    Error { error: ErrorData },
}

/// Message start data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageStartData {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Usage,
}

/// Content delta for streaming updates
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Delta {
    /// Text delta
    #[serde(rename = "text_delta")]
    TextDelta { text: String },

    /// Input JSON delta
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

/// Message delta data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeltaData {
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
}

/// Error data from stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorData {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

/// Server-Sent Events parser for Claude API streaming responses
pub struct SseStreamParser {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    buffer: String,
}

impl SseStreamParser {
    /// Create a new SSE parser from a byte stream
    pub fn new(stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(stream),
            buffer: String::new(),
        }
    }

    /// Parse a single SSE line into an event
    fn parse_sse_line(&self, line: &str) -> Option<Result<StreamEvent, ClaudeApiError>> {
        // SSE format: "data: {json}\n\n"
        if !line.starts_with("data: ") {
            return None;
        }

        let data = &line[6..]; // Skip "data: " prefix

        if data.trim().is_empty() {
            return None;
        }

        // Special SSE events
        if data == "[DONE]" {
            return None; // End of stream
        }

        // Parse JSON event
        match serde_json::from_str::<StreamEvent>(data) {
            Ok(event) => Some(Ok(event)),
            Err(err) => {
                warn!("Failed to parse SSE event: {} - Data: {}", err, data);
                Some(Err(ClaudeApiError::JsonError(err)))
            }
        }
    }
}

impl Stream for SseStreamParser {
    type Item = Result<StreamEvent, ClaudeApiError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Try to parse events from buffer
            if let Some(line_end) = self.buffer.find("\n\n") {
                let line = self.buffer[..line_end].to_string();
                self.buffer = self.buffer[line_end + 2..].to_string();

                if let Some(event) = self.parse_sse_line(&line) {
                    debug!("Parsed SSE event: {:?}", event);
                    return Poll::Ready(Some(event));
                }
                continue;
            }

            // Need more data - poll inner stream
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    let chunk = String::from_utf8_lossy(&bytes);
                    self.buffer.push_str(&chunk);
                    continue;
                }
                Poll::Ready(Some(Err(err))) => {
                    return Poll::Ready(Some(Err(ClaudeApiError::NetworkError(err))));
                }
                Poll::Ready(None) => {
                    // Stream ended - check if buffer has remaining data
                    if !self.buffer.is_empty() {
                        warn!("Stream ended with unparsed data: {}", self.buffer);
=======

/// Represents a single Server-Sent Event from the Claude API streaming endpoint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    /// The type of event (e.g., "message_start", "content_block_delta")
    #[serde(rename = "type")]
    pub event_type: String,

    /// The event data as a generic JSON value
    /// Structure varies by event_type:
    /// - message_start: Contains message metadata
    /// - content_block_start: Contains content block metadata
    /// - content_block_delta: Contains incremental text delta
    /// - content_block_stop: Signals end of content block
    /// - message_delta: Contains message-level updates
    /// - message_stop: Signals end of message
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Parse a Server-Sent Event string into a StreamEvent
///
/// SSE format:
/// ```text
/// event: message_start
/// data: {"type":"message_start","message":{...}}
///
/// ```
///
/// # Arguments
/// * `text` - Raw SSE event string containing event and data fields
///
/// # Returns
/// * `Ok(StreamEvent)` - Successfully parsed event
/// * `Err(anyhow::Error)` - Invalid format or missing required fields
///
/// # Example
/// ```
/// use abathur::infrastructure::claude::streaming::parse_sse_event;
///
/// let sse_text = r#"event: message_start
/// data: {"type":"message_start","message":{"id":"msg_123","role":"assistant"}}"#;
///
/// let event = parse_sse_event(sse_text).unwrap();
/// assert_eq!(event.event_type, "message_start");
/// ```
pub fn parse_sse_event(text: &str) -> Result<StreamEvent> {
    let mut event_type: Option<String> = None;
    let mut data_line: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }

        if let Some(event_value) = trimmed.strip_prefix("event:") {
            event_type = Some(event_value.trim().to_string());
        } else if let Some(data_value) = trimmed.strip_prefix("data:") {
            data_line = Some(data_value.trim().to_string());
        }
    }

    match (event_type, data_line) {
        (Some(_), Some(data_str)) => {
            // Parse the data JSON
            let data: serde_json::Value = serde_json::from_str(&data_str)
                .map_err(|e| anyhow!("Failed to parse SSE data as JSON: {}", e))?;

            // Extract event_type from data (Claude API includes type in the data payload)
            let event_type_from_data = data.get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Missing 'type' field in SSE data"))?
                .to_string();

            Ok(StreamEvent {
                event_type: event_type_from_data,
                data,
            })
        }
        (None, Some(_)) => {
            Err(anyhow!("SSE event missing 'event:' field"))
        }
        (Some(_), None) => {
            Err(anyhow!("SSE event missing 'data:' field"))
        }
        (None, None) => {
            Err(anyhow!("Invalid SSE event format: missing both event and data fields"))
        }
    }
}

/// Wraps a byte stream and parses it into SSE events
///
/// This struct implements `Stream<Item = Result<StreamEvent>>` to provide
/// an async stream of parsed events from the raw HTTP response bytes.
pub struct SseEventStream {
    inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buffer: String,
}

impl SseEventStream {
    /// Create a new SSE event stream from a reqwest byte stream
    ///
    /// # Arguments
    /// * `byte_stream` - The raw byte stream from `reqwest::Response::bytes_stream()`
    ///
    /// # Returns
    /// A new `SseEventStream` that yields parsed `StreamEvent`s
    pub fn new(byte_stream: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(byte_stream),
            buffer: String::new(),
        }
    }
}

impl Stream for SseEventStream {
    type Item = Result<StreamEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Try to parse any complete event from the buffer
            if let Some(event_end) = self.buffer.find("\n\n") {
                let event_text = self.buffer[..event_end].to_string();
                self.buffer.drain(..event_end + 2);

                // Skip empty events
                if event_text.trim().is_empty() {
                    continue;
                }

                match parse_sse_event(&event_text) {
                    Ok(event) => return Poll::Ready(Some(Ok(event))),
                    Err(e) => return Poll::Ready(Some(Err(e))),
                }
            }

            // Need more data - poll the inner stream
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    // Append new bytes to buffer
                    match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => self.buffer.push_str(&text),
                        Err(e) => {
                            return Poll::Ready(Some(Err(anyhow!("Invalid UTF-8 in stream: {}", e))));
                        }
                    }
                    // Loop to try parsing again
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(anyhow!("Stream error: {}", e))));
                }
                Poll::Ready(None) => {
                    // Stream ended - parse any remaining buffered data
                    if !self.buffer.trim().is_empty() {
                        let remaining = self.buffer.clone();
                        self.buffer.clear();
                        match parse_sse_event(&remaining) {
                            Ok(event) => return Poll::Ready(Some(Ok(event))),
                            Err(e) => return Poll::Ready(Some(Err(e))),
                        }
>>>>>>> task_sse-streaming-parser_20251025-210007
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
<<<<<<< HEAD
    use futures::stream;
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn test_parse_message_start_event() {
        let sse_data = r#"data: {"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[],"model":"claude-3-5-sonnet-20241022","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}}

"#;

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(sse_data))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event = parser.next().await.unwrap().unwrap();

        if let StreamEvent::MessageStart { message } = event {
            assert_eq!(message.id, "msg_123");
            assert_eq!(message.role, "assistant");
            assert_eq!(message.usage.input_tokens, 10);
        } else {
            panic!("Expected MessageStart event");
        }
    }

    #[tokio::test]
    async fn test_parse_text_delta_event() {
        let sse_data = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

"#;

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(sse_data))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event = parser.next().await.unwrap().unwrap();

        if let StreamEvent::ContentBlockDelta { index, delta } = event {
            assert_eq!(index, 0);
            if let Delta::TextDelta { text } = delta {
                assert_eq!(text, "Hello");
            } else {
                panic!("Expected TextDelta");
            }
        } else {
            panic!("Expected ContentBlockDelta event");
        }
    }

    #[tokio::test]
    async fn test_parse_message_stop_event() {
        let sse_data = r#"data: {"type":"message_stop"}

"#;

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(sse_data))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event = parser.next().await.unwrap().unwrap();

        assert!(matches!(event, StreamEvent::MessageStop));
    }

    #[tokio::test]
    async fn test_parse_multiple_events() {
        let sse_data = r#"data: {"type":"message_start","message":{"id":"msg_123","type":"message","role":"assistant","content":[],"model":"claude-3-5-sonnet-20241022","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}

data: {"type":"message_stop"}

"#;

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(sse_data))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event1 = parser.next().await.unwrap().unwrap();
        assert!(matches!(event1, StreamEvent::MessageStart { .. }));

        let event2 = parser.next().await.unwrap().unwrap();
        assert!(matches!(event2, StreamEvent::ContentBlockDelta { .. }));

        let event3 = parser.next().await.unwrap().unwrap();
        assert!(matches!(event3, StreamEvent::MessageStop));

        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_parse_chunked_data() {
        let chunk1 = "data: {\"type\":\"message_";
        let chunk2 = "stop\"}\n\n";

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(chunk1)), Ok(Bytes::from(chunk2))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event = parser.next().await.unwrap().unwrap();
        assert!(matches!(event, StreamEvent::MessageStop));
    }

    #[tokio::test]
    async fn test_parse_ping_event() {
        let sse_data = r#"data: {"type":"ping"}

"#;

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(sse_data))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event = parser.next().await.unwrap().unwrap();
        assert!(matches!(event, StreamEvent::Ping));
    }

    #[tokio::test]
    async fn test_parse_error_event() {
        let sse_data = r#"data: {"type":"error","error":{"type":"invalid_request","message":"Bad request"}}

"#;

        let bytes_stream = stream::iter(vec![Ok(Bytes::from(sse_data))]);
        let mut parser = SseStreamParser::new(bytes_stream);

        let event = parser.next().await.unwrap().unwrap();

        if let StreamEvent::Error { error } = event {
            assert_eq!(error.error_type, "invalid_request");
            assert_eq!(error.message, "Bad request");
        } else {
            panic!("Expected Error event");
        }
=======

    #[test]
    fn test_parse_sse_event_message_start() {
        let sse_text = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_123","role":"assistant","model":"claude-3-5-sonnet-20241022"}}"#;

        let event = parse_sse_event(sse_text).unwrap();
        assert_eq!(event.event_type, "message_start");
        assert_eq!(event.data["message"]["id"], "msg_123");
        assert_eq!(event.data["message"]["role"], "assistant");
    }

    #[test]
    fn test_parse_sse_event_content_block_delta() {
        let sse_text = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;

        let event = parse_sse_event(sse_text).unwrap();
        assert_eq!(event.event_type, "content_block_delta");
        assert_eq!(event.data["delta"]["text"], "Hello");
        assert_eq!(event.data["index"], 0);
    }

    #[test]
    fn test_parse_sse_event_message_stop() {
        let sse_text = r#"event: message_stop
data: {"type":"message_stop"}"#;

        let event = parse_sse_event(sse_text).unwrap();
        assert_eq!(event.event_type, "message_stop");
    }

    #[test]
    fn test_parse_sse_event_missing_event_field() {
        let sse_text = r#"data: {"type":"message_start","message":{}}"#;

        let result = parse_sse_event(sse_text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'event:' field"));
    }

    #[test]
    fn test_parse_sse_event_missing_data_field() {
        let sse_text = r#"event: message_start"#;

        let result = parse_sse_event(sse_text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'data:' field"));
    }

    #[test]
    fn test_parse_sse_event_invalid_json() {
        let sse_text = r#"event: message_start
data: {invalid json}"#;

        let result = parse_sse_event(sse_text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse SSE data as JSON"));
    }

    #[test]
    fn test_parse_sse_event_missing_type_in_data() {
        let sse_text = r#"event: message_start
data: {"message":{"id":"msg_123"}}"#;

        let result = parse_sse_event(sse_text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing 'type' field"));
    }

    #[test]
    fn test_parse_sse_event_with_extra_whitespace() {
        let sse_text = r#"  event:  message_start
  data:  {"type":"message_start","message":{"id":"msg_123"}}  "#;

        let event = parse_sse_event(sse_text).unwrap();
        assert_eq!(event.event_type, "message_start");
    }

    #[test]
    fn test_parse_sse_event_with_comments() {
        let sse_text = r#": This is a comment
event: message_start
: Another comment
data: {"type":"message_start","message":{"id":"msg_123"}}"#;

        let event = parse_sse_event(sse_text).unwrap();
        assert_eq!(event.event_type, "message_start");
>>>>>>> task_sse-streaming-parser_20251025-210007
    }
}

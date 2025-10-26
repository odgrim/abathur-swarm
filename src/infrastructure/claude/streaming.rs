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
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context, Poll};

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
    }
}

use super::errors::ClaudeApiError;
use super::types::{ContentBlock, Usage};
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context, Poll};
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
    }
}

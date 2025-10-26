<<<<<<< HEAD
use serde::{Deserialize, Serialize};

/// Request to send a message to Claude
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRequest {
    /// The model to use (e.g., "claude-3-5-sonnet-20241022")
    pub model: String,

    /// Input messages (conversation history + new message)
    pub messages: Vec<Message>,

    /// Maximum tokens to generate in response
=======
/// Request and response types for Claude API
use serde::{Deserialize, Serialize};

/// Message request to send to Claude API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRequest {
    /// Model identifier (e.g., "claude-3-5-sonnet-20241022")
    pub model: String,

    /// Array of messages in the conversation
    pub messages: Vec<Message>,

    /// Maximum tokens to generate
>>>>>>> task_claude-api-integration-tests_20251025-210007
    pub max_tokens: u32,

    /// System prompt (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,

<<<<<<< HEAD
    /// Sampling temperature 0.0-1.0 (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Tools available for the model to use (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    /// Metadata for the request (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Response from Claude API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Unique message identifier
    pub id: String,

    /// Object type (always "message")
    #[serde(rename = "type")]
    pub response_type: String,

    /// Role of the responder (always "assistant")
    pub role: String,

    /// Response content blocks
    pub content: Vec<ContentBlock>,

    /// Model used for generation
    pub model: String,

    /// Stop reason
    pub stop_reason: Option<String>,

    /// Stop sequence that triggered stop
    pub stop_sequence: Option<String>,

    /// Token usage statistics
    pub usage: Usage,
}

/// A single message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "user" or "assistant"
    pub role: String,

    /// Message content (text or structured content blocks)
    pub content: MessageContent,
}

/// Message content can be simple text or structured content blocks
=======
    /// Temperature for sampling (0.0-1.0, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Tool definitions (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    /// Metadata (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Enable streaming (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

impl Default for MessageRequest {
    fn default() -> Self {
        Self {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: Vec::new(),
            max_tokens: 4096,
            system: None,
            temperature: None,
            tools: None,
            metadata: None,
            stream: None,
        }
    }
}

/// A single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message sender
    pub role: String,

    /// Content of the message (string or array of content blocks)
    #[serde(with = "message_content")]
    pub content: MessageContent,
}

/// Message content can be either a simple string or an array of content blocks
>>>>>>> task_claude-api-integration-tests_20251025-210007
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content
    Text(String),
<<<<<<< HEAD
    /// Structured content blocks (images, tool use, etc.)
    Blocks(Vec<ContentBlock>),
}

=======
    /// Array of content blocks (text, images, etc.)
    Blocks(Vec<ContentBlock>),
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

mod message_content {
    use super::MessageContent;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(content: &MessageContent, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match content {
            MessageContent::Text(s) => s.serialize(serializer),
            MessageContent::Blocks(blocks) => blocks.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<MessageContent, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Text(String),
            Blocks(Vec<super::ContentBlock>),
        }

        match Helper::deserialize(deserializer)? {
            Helper::Text(s) => Ok(MessageContent::Text(s)),
            Helper::Blocks(blocks) => Ok(MessageContent::Blocks(blocks)),
        }
    }
}

>>>>>>> task_claude-api-integration-tests_20251025-210007
/// Content block in a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
<<<<<<< HEAD
    /// Text content
    #[serde(rename = "text")]
    Text { text: String },

    /// Tool use request from assistant
=======
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
>>>>>>> task_claude-api-integration-tests_20251025-210007
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
<<<<<<< HEAD

    /// Tool result from user
=======
>>>>>>> task_claude-api-integration-tests_20251025-210007
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
<<<<<<< HEAD
    },

    /// Image content
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

/// Image source specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    /// Base64 encoded image
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
=======
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Image source for content blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    #[serde(rename = "base64")]
    Base64 {
        media_type: String,
        data: String,
    },
    #[serde(rename = "url")]
    Url {
        url: String,
    },
>>>>>>> task_claude-api-integration-tests_20251025-210007
}

/// Tool definition for function calling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name
    pub name: String,

    /// Tool description
    pub description: String,

<<<<<<< HEAD
    /// JSON schema for input parameters
    pub input_schema: serde_json::Value,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens in input
    pub input_tokens: u32,

    /// Tokens in output
    pub output_tokens: u32,
}

impl MessageRequest {
    /// Create a simple text message request
    ///
    /// # Example
    /// ```
    /// use abathur::infrastructure::claude::types::MessageRequest;
    ///
    /// let request = MessageRequest::simple_message(
    ///     "claude-3-5-sonnet-20241022".to_string(),
    ///     "Hello, Claude!".to_string(),
    ///     1024,
    /// );
    /// ```
    pub fn simple_message(model: String, user_message: String, max_tokens: u32) -> Self {
        Self {
            model,
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::Text(user_message),
            }],
            max_tokens,
            system: None,
            temperature: None,
            tools: None,
            metadata: None,
        }
    }
}

=======
    /// JSON schema for tool input
    pub input_schema: serde_json::Value,
}

/// Response from Claude API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Unique message ID
    pub id: String,

    /// Response type (always "message")
    #[serde(rename = "type")]
    pub response_type: String,

    /// Role of the responder (always "assistant")
    pub role: String,

    /// Array of content blocks in the response
    pub content: Vec<ContentBlock>,

    /// Model that generated the response
    pub model: String,

    /// Reason for stopping generation
    pub stop_reason: StopReason,

    /// Stop sequence that triggered the stop (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,

    /// Token usage statistics
    pub usage: Usage,
}

/// Reason why message generation stopped
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Natural end of turn
    EndTurn,
    /// Maximum tokens reached
    MaxTokens,
    /// Stop sequence encountered
    StopSequence,
    /// Tool use initiated
    ToolUse,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of input tokens
    pub input_tokens: u32,

    /// Number of output tokens
    pub output_tokens: u32,
}

>>>>>>> task_claude-api-integration-tests_20251025-210007
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
<<<<<<< HEAD
    fn test_simple_message_request_serialization() {
        let request = MessageRequest::simple_message(
            "claude-3-5-sonnet-20241022".to_string(),
            "Hello!".to_string(),
            100,
        );

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"claude-3-5-sonnet-20241022\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"max_tokens\":100"));
    }

    #[test]
    fn test_message_response_deserialization() {
        let json = r#"{
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "Hello! How can I help?"
                }
            ],
            "model": "claude-3-5-sonnet-20241022",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20
            }
        }"#;

        let response: MessageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, "msg_123");
        assert_eq!(response.response_type, "message");
        assert_eq!(response.role, "assistant");
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 20);

        if let ContentBlock::Text { text } = &response.content[0] {
            assert_eq!(text, "Hello! How can I help?");
        } else {
            panic!("Expected text content block");
        }
    }

    #[test]
    fn test_tool_use_content_block() {
        let json = r#"{
            "type": "tool_use",
            "id": "tool_123",
            "name": "get_weather",
            "input": {
                "location": "San Francisco"
            }
        }"#;

        let block: ContentBlock = serde_json::from_str(json).unwrap();
        if let ContentBlock::ToolUse { id, name, input } = block {
            assert_eq!(id, "tool_123");
            assert_eq!(name, "get_weather");
            assert_eq!(input["location"], "San Francisco");
        } else {
            panic!("Expected ToolUse content block");
        }
    }

    #[test]
    fn test_optional_fields_omitted() {
        let request = MessageRequest::simple_message(
            "claude-3-5-sonnet-20241022".to_string(),
            "Test".to_string(),
            100,
        );

        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("\"system\""));
        assert!(!json.contains("\"temperature\""));
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"metadata\""));
=======
    fn test_message_request_serialization() {
        let request = MessageRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Hello".into(),
            }],
            max_tokens: 100,
            system: None,
            temperature: Some(0.7),
            tools: None,
            metadata: None,
            stream: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("claude-3-5-sonnet-20241022"));
        assert!(json.contains("Hello"));
        assert!(json.contains("0.7"));
>>>>>>> task_claude-api-integration-tests_20251025-210007
    }

    #[test]
    fn test_message_content_text() {
<<<<<<< HEAD
        let content = MessageContent::Text("Hello".to_string());
        let json = serde_json::to_string(&content).unwrap();
        assert_eq!(json, "\"Hello\"");
    }

    #[test]
    fn test_message_content_blocks() {
        let content = MessageContent::Blocks(vec![ContentBlock::Text {
            text: "Hello".to_string(),
        }]);
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello\""));
=======
        let content: MessageContent = "test".into();
        assert!(matches!(content, MessageContent::Text(_)));
    }

    #[test]
    fn test_content_block_text() {
        let block = ContentBlock::Text {
            text: "test".to_string(),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains("test"));
>>>>>>> task_claude-api-integration-tests_20251025-210007
    }
}

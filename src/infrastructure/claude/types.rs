use serde::{Deserialize, Serialize};

/// Request to send a message to Claude
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRequest {
    /// The model to use (e.g., "claude-3-5-sonnet-20241022")
    pub model: String,

    /// Input messages (conversation history + new message)
    pub messages: Vec<Message>,

    /// Maximum tokens to generate in response
    pub max_tokens: u32,

    /// System prompt (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content
    Text(String),
    /// Structured content blocks (images, tool use, etc.)
    Blocks(Vec<ContentBlock>),
}

/// Content block in a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// Text content
    #[serde(rename = "text")]
    Text { text: String },

    /// Tool use request from assistant
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// Tool result from user
}

/// Tool definition for function calling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Tool name
    pub name: String,

    /// Tool description
    pub description: String,

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
    }

    #[test]
    fn test_message_content_text() {
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
    }
}

//! Anthropic API substrate implementation.
//!
//! Makes direct HTTP calls to the Anthropic Messages API as an alternative
//! to the Claude Code CLI substrate.

use async_trait::async_trait;
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::models::{
    SessionStatus, SubstrateOutput, SubstrateRequest, SubstrateSession,
};
use crate::domain::ports::Substrate;

/// Configuration for the Anthropic API substrate.
#[derive(Debug, Clone)]
pub struct AnthropicApiConfig {
    /// API key (will be read from ANTHROPIC_API_KEY env if not set).
    pub api_key: Option<String>,
    /// API base URL.
    pub base_url: String,
    /// Default model to use.
    pub default_model: String,
    /// API version header.
    pub api_version: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Max tokens to generate.
    pub max_tokens: u32,
    /// Whether to use streaming.
    pub stream: bool,
}

impl Default for AnthropicApiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: "https://api.anthropic.com".to_string(),
            default_model: "claude-opus-4-6-20250616".to_string(),
            api_version: "2023-06-01".to_string(),
            timeout_secs: 300,
            max_tokens: 4096,
            stream: true,
        }
    }
}

impl AnthropicApiConfig {
    /// Get API key from config or environment.
    pub fn get_api_key(&self) -> Option<String> {
        self.api_key.clone().or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    }

    /// Create config with explicit API key.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Create config with custom model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }
}

/// Message role in Anthropic API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
}

/// Cache control marker for Anthropic prompt caching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub control_type: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self { control_type: "ephemeral".to_string() }
    }
}

/// System prompt content block with optional cache_control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemContentBlock {
    /// Create a text block without caching.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a text block with ephemeral cache_control.
    pub fn cached_text(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

/// Content block in a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
}

/// Request to the Anthropic Messages API.
#[derive(Debug, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    /// System prompt as content block array (supports cache_control markers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemContentBlock>>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// Usage information from the API.
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

/// Response from the Anthropic Messages API.
#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

/// Streaming event from the API.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: ContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: DeltaBlock },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaData,
        usage: Usage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: ApiError },
}

#[derive(Debug, Deserialize)]
pub struct MessageStartData {
    pub id: String,
    pub model: String,
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub struct DeltaBlock {
    #[serde(rename = "type")]
    pub delta_type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub partial_json: String,
}

#[derive(Debug, Deserialize)]
pub struct MessageDeltaData {
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApiError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

/// Anthropic API substrate.
pub struct AnthropicApiSubstrate {
    config: AnthropicApiConfig,
    client: Client,
    sessions: Arc<RwLock<HashMap<Uuid, SubstrateSession>>>,
}

impl AnthropicApiSubstrate {
    /// Create a new Anthropic API substrate.
    pub fn new(config: AnthropicApiConfig) -> DomainResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config,
            client,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Create with default configuration.
    pub fn with_defaults() -> DomainResult<Self> {
        Self::new(AnthropicApiConfig::default())
    }

    /// Build the Messages API request from a substrate request.
    ///
    /// The system prompt is sent as a content block array with `cache_control`
    /// markers for prompt caching. The stable base prompt gets a cache breakpoint
    /// so subsequent calls with the same prefix get ~90% input token savings.
    fn build_request(&self, request: &SubstrateRequest) -> MessagesRequest {
        let model = request.config.model.clone()
            .unwrap_or_else(|| self.config.default_model.clone());

        let messages = vec![Message {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: request.user_prompt.clone(),
            }],
        }];

        let system = if request.system_prompt.is_empty() {
            None
        } else {
            // Send system prompt as content block array with cache_control.
            // The entire system prompt is marked as cacheable (ephemeral).
            // On subsequent calls with the same system prompt prefix,
            // Anthropic will serve from cache (~90% input token savings).
            Some(vec![SystemContentBlock::cached_text(&request.system_prompt)])
        };

        MessagesRequest {
            model,
            max_tokens: self.config.max_tokens,
            system,
            messages,
            stream: self.config.stream,
            temperature: None,
        }
    }

    /// Parse SSE stream events.
    fn parse_sse_event(line: &str) -> Option<StreamEvent> {
        if !line.starts_with("data: ") {
            return None;
        }

        let json_str = line.strip_prefix("data: ")?;
        if json_str == "[DONE]" {
            return None;
        }

        serde_json::from_str(json_str).ok()
    }

    /// Execute a non-streaming request.
    async fn execute_sync(&self, request: &SubstrateRequest) -> DomainResult<(String, Usage)> {
        let api_key = self.config.get_api_key()
            .ok_or_else(|| DomainError::ValidationFailed("ANTHROPIC_API_KEY not set".to_string()))?;

        let mut api_request = self.build_request(request);
        api_request.stream = false;

        let response = self.client
            .post(format!("{}/v1/messages", self.config.base_url))
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-api-key", &api_key)
            .header("anthropic-version", &self.config.api_version)
            .json(&api_request)
            .send()
            .await
            .map_err(|e| DomainError::ValidationFailed(format!("API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DomainError::ValidationFailed(format!(
                "API error {}: {}", status, body
            )));
        }

        let result: MessagesResponse = response.json().await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to parse response: {}", e)))?;

        // Extract text from content blocks
        let text = result.content.iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok((text, result.usage))
    }
}

#[async_trait]
impl Substrate for AnthropicApiSubstrate {
    fn name(&self) -> &'static str {
        "anthropic_api"
    }

    async fn is_available(&self) -> DomainResult<bool> {
        // Check if API key is available
        Ok(self.config.get_api_key().is_some())
    }

    async fn execute(&self, request: SubstrateRequest) -> DomainResult<SubstrateSession> {
        // Create session
        let mut session = SubstrateSession::new(
            request.task_id,
            &request.agent_template,
            request.config.clone(),
        );
        session.start(None);

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session.id, session.clone());
        }

        // Execute request
        match self.execute_sync(&request).await {
            Ok((text, usage)) => {
                session.input_tokens = usage.input_tokens;
                session.output_tokens = usage.output_tokens;
                session.cache_read_tokens = usage.cache_read_input_tokens;
                session.cache_write_tokens = usage.cache_creation_input_tokens;
                session.turns_completed = 1;
                session.complete(&text);
            }
            Err(e) => {
                session.fail(&e.to_string());
            }
        }

        // Update session store
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session.id, session.clone());
        }

        Ok(session)
    }

    async fn execute_streaming(
        &self,
        request: SubstrateRequest,
    ) -> DomainResult<(mpsc::Receiver<SubstrateOutput>, SubstrateSession)> {
        let api_key = self.config.get_api_key()
            .ok_or_else(|| DomainError::ValidationFailed("ANTHROPIC_API_KEY not set".to_string()))?;

        // Create session
        let mut session = SubstrateSession::new(
            request.task_id,
            &request.agent_template,
            request.config.clone(),
        );
        session.start(None);

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session.id, session.clone());
        }

        let api_request = self.build_request(&request);

        // Create output channel
        let (tx, rx) = mpsc::channel(100);

        let client = self.client.clone();
        let base_url = self.config.base_url.clone();
        let api_version = self.config.api_version.clone();
        let session_id = session.id;
        let sessions_clone = self.sessions.clone();

        // Spawn streaming task
        // Note: For simplicity, we fetch the full response and simulate streaming.
        // A full SSE implementation would require the `stream` feature in reqwest.
        tokio::spawn(async move {
            let response = client
                .post(format!("{}/v1/messages", base_url))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-api-key", &api_key)
                .header("anthropic-version", &api_version)
                .json(&api_request)
                .send()
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(SubstrateOutput::Error {
                        message: format!("Request failed: {}", e),
                    }).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let _ = tx.send(SubstrateOutput::Error {
                    message: format!("API error {}: {}", status, body),
                }).await;
                return;
            }

            // Read full response body
            let body = match response.text().await {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(SubstrateOutput::Error {
                        message: format!("Failed to read response: {}", e),
                    }).await;
                    return;
                }
            };

            let mut all_text = String::new();
            let mut total_input = 0u64;
            let mut total_output = 0u64;

            // Process SSE lines
            for line in body.lines() {
                if let Some(event) = Self::parse_sse_event(line) {
                    match event {
                        StreamEvent::ContentBlockDelta { delta, .. } => {
                            if !delta.text.is_empty() {
                                all_text.push_str(&delta.text);
                                let _ = tx.send(SubstrateOutput::AssistantText {
                                    content: delta.text,
                                }).await;
                            }
                        }
                        StreamEvent::MessageStart { message } => {
                            total_input = message.usage.input_tokens;
                        }
                        StreamEvent::MessageDelta { usage, .. } => {
                            total_output = usage.output_tokens;
                            let _ = tx.send(SubstrateOutput::TurnComplete {
                                turn_number: 1,
                                input_tokens: total_input,
                                output_tokens: total_output,
                            }).await;
                        }
                        StreamEvent::MessageStop => {
                            let _ = tx.send(SubstrateOutput::SessionComplete {
                                result: all_text.clone(),
                            }).await;
                        }
                        StreamEvent::Error { error } => {
                            let _ = tx.send(SubstrateOutput::Error {
                                message: error.message,
                            }).await;
                        }
                        StreamEvent::ContentBlockStart { content_block, .. } => {
                            if let ContentBlock::ToolUse { id, name, .. } = content_block {
                                let _ = tx.send(SubstrateOutput::ToolStart { name, id }).await;
                            }
                        }
                        _ => {}
                    }
                }
            }

            // If we didn't get streaming events, try parsing as a regular JSON response
            if all_text.is_empty() {
                if let Ok(result) = serde_json::from_str::<MessagesResponse>(&body) {
                    for block in &result.content {
                        if let ContentBlock::Text { text } = block {
                            all_text.push_str(text);
                            let _ = tx.send(SubstrateOutput::AssistantText {
                                content: text.clone(),
                            }).await;
                        }
                    }
                    total_input = result.usage.input_tokens;
                    total_output = result.usage.output_tokens;

                    let _ = tx.send(SubstrateOutput::TurnComplete {
                        turn_number: 1,
                        input_tokens: total_input,
                        output_tokens: total_output,
                    }).await;

                    let _ = tx.send(SubstrateOutput::SessionComplete {
                        result: all_text.clone(),
                    }).await;
                }
            }

            // Update session
            {
                let mut sessions = sessions_clone.write().await;
                if let Some(session) = sessions.get_mut(&session_id) {
                    session.input_tokens = total_input;
                    session.output_tokens = total_output;
                    session.turns_completed = 1;
                    if session.status == SessionStatus::Active {
                        session.complete(&all_text);
                    }
                }
            }
        });

        Ok((rx, session))
    }

    async fn resume(
        &self,
        session_id: Uuid,
        additional_prompt: Option<String>,
    ) -> DomainResult<SubstrateSession> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&session_id)
            .ok_or_else(|| DomainError::ValidationFailed(format!("Session {} not found", session_id)))?;

        if !session.status.is_terminal() {
            return Err(DomainError::ValidationFailed(
                "Cannot resume active session".to_string()
            ));
        }

        // Create a new request based on the original session
        let request = SubstrateRequest {
            task_id: session.task_id,
            agent_template: session.agent_template.clone(),
            system_prompt: String::new(),
            user_prompt: additional_prompt.unwrap_or_else(|| "Continue.".to_string()),
            config: session.config.clone(),
            resume_session: Some(session_id),
        };

        drop(sessions);
        self.execute(request).await
    }

    async fn terminate(&self, session_id: Uuid) -> DomainResult<()> {
        // For API substrate, we just mark the session as terminated
        // (we can't actually cancel an in-flight HTTP request easily)
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.terminate();
        }
        Ok(())
    }

    async fn get_session(&self, session_id: Uuid) -> DomainResult<Option<SubstrateSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&session_id).cloned())
    }

    async fn is_running(&self, session_id: Uuid) -> DomainResult<bool> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(&session_id)
            .map(|s| s.status == SessionStatus::Active)
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = AnthropicApiConfig::default();
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert!(config.stream);
        assert_eq!(config.max_tokens, 4096);
    }

    #[test]
    fn test_config_with_api_key() {
        let config = AnthropicApiConfig::default()
            .with_api_key("test-key");
        assert_eq!(config.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_config_with_model() {
        let config = AnthropicApiConfig::default()
            .with_model("claude-opus-4-5-20251101");
        assert_eq!(config.default_model, "claude-opus-4-5-20251101");
    }

    #[test]
    fn test_build_request() {
        let config = AnthropicApiConfig::default().with_api_key("test");
        let substrate = AnthropicApiSubstrate::new(config).unwrap();

        let request = SubstrateRequest::new(
            Uuid::new_v4(),
            "test-agent",
            "You are a helpful assistant",
            "Hello!",
        );

        let api_request = substrate.build_request(&request);
        assert_eq!(api_request.messages.len(), 1);
        assert!(api_request.system.is_some());
        assert!(api_request.stream);
    }

    #[test]
    fn test_parse_sse_event_text_delta() {
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event = AnthropicApiSubstrate::parse_sse_event(line);
        assert!(event.is_some());
    }

    #[test]
    fn test_parse_sse_event_message_stop() {
        let line = r#"data: {"type":"message_stop"}"#;
        let event = AnthropicApiSubstrate::parse_sse_event(line);
        assert!(matches!(event, Some(StreamEvent::MessageStop)));
    }

    #[test]
    fn test_parse_sse_event_done() {
        let line = "data: [DONE]";
        let event = AnthropicApiSubstrate::parse_sse_event(line);
        assert!(event.is_none());
    }

    #[test]
    fn test_parse_sse_event_non_data() {
        let line = "event: ping";
        let event = AnthropicApiSubstrate::parse_sse_event(line);
        assert!(event.is_none());
    }
}

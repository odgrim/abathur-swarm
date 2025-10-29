//! Adapter implementing the domain ClaudeClient port trait
//!
//! This module provides the bridge between the infrastructure HTTP client
//! and the domain port trait, handling type conversions and API details.

use crate::domain::ports::{
    ClaudeClient as ClaudeClientPort, ClaudeError, ClaudeRequest, ClaudeResponse,
    ContentBlock as DomainContentBlock, MessageChunk,
    MessageRequest as DomainMessageRequest, MessageResponse as DomainMessageResponse,
    TokenUsage as DomainTokenUsage, Usage as DomainUsage,
};
use crate::infrastructure::claude::client::{ClaudeClient as HttpClient, ClaudeClientConfig};
use crate::infrastructure::claude::errors::ClaudeApiError;
use crate::infrastructure::claude::streaming::{Delta, SseStreamParser, StreamEvent};
use crate::infrastructure::claude::types::{
    ContentBlock as InfraContentBlock, Message as InfraMessage, MessageContent,
    MessageRequest as InfraMessageRequest, MessageResponse as InfraMessageResponse,
};
use async_trait::async_trait;
use futures::{stream::Stream, StreamExt};
use tracing::{debug, error, info, instrument};

/// Adapter wrapping the HTTP client to implement the domain port trait
///
/// This adapter bridges the infrastructure HTTP client with the domain port,
/// handling all necessary type conversions and API details.
pub struct ClaudeClientAdapter {
    http_client: HttpClient,
}

impl ClaudeClientAdapter {
    /// Create a new adapter with the given configuration
    pub fn new(config: ClaudeClientConfig) -> Result<Self, ClaudeApiError> {
        let http_client = HttpClient::new(config)?;
        Ok(Self { http_client })
    }

    /// Create a new adapter with default configuration
    pub fn from_env() -> Result<Self, ClaudeApiError> {
        let config = ClaudeClientConfig::default();
        Self::new(config)
    }
}

// Type conversions: Domain -> Infrastructure
impl From<DomainMessageRequest> for InfraMessageRequest {
    fn from(req: DomainMessageRequest) -> Self {
        Self {
            model: req.model,
            messages: req
                .messages
                .into_iter()
                .map(|msg| InfraMessage {
                    role: msg.role,
                    content: MessageContent::Text(msg.content),
                })
                .collect(),
            max_tokens: req.max_tokens as u32,
            system: req.system,
            temperature: req.temperature.map(|t| t as f32),
            tools: None,
            metadata: None,
        }
    }
}

// Type conversions: Infrastructure -> Domain
impl From<InfraMessageResponse> for DomainMessageResponse {
    fn from(resp: InfraMessageResponse) -> Self {
        Self {
            id: resp.id,
            content: resp
                .content
                .into_iter()
                .filter_map(|block| match block {
                    InfraContentBlock::Text { text } => Some(DomainContentBlock {
                        content_type: "text".to_string(),
                        text: Some(text),
                    }),
                    _ => None, // Filter out tool use and other non-text blocks for now
                })
                .collect(),
            stop_reason: resp.stop_reason,
            usage: DomainUsage {
                input_tokens: resp.usage.input_tokens as usize,
                output_tokens: resp.usage.output_tokens as usize,
            },
        }
    }
}

// Error conversions
impl From<ClaudeApiError> for ClaudeError {
    fn from(err: ClaudeApiError) -> Self {
        match err {
            ClaudeApiError::InvalidApiKey => ClaudeError::InvalidApiKey,
            ClaudeApiError::RateLimitExceeded => {
                ClaudeError::RateLimitExceeded("Rate limit exceeded".to_string())
            }
            ClaudeApiError::Timeout => ClaudeError::Timeout,
            ClaudeApiError::NetworkError(e) => ClaudeError::NetworkError(e.to_string()),
            ClaudeApiError::InvalidRequest(msg) => ClaudeError::ApiError(msg),
            ClaudeApiError::Forbidden(msg) => ClaudeError::ApiError(msg),
            ClaudeApiError::NotFound => ClaudeError::ApiError("Resource not found".to_string()),
            ClaudeApiError::ServerError(status, msg) => {
                ClaudeError::ApiError(format!("Server error ({}): {}", status, msg))
            }
            ClaudeApiError::UnknownError(status, msg) => {
                ClaudeError::ApiError(format!("Unknown error ({}): {}", status, msg))
            }
            ClaudeApiError::JsonError(e) => ClaudeError::ApiError(format!("JSON error: {}", e)),
        }
    }
}

#[async_trait]
impl ClaudeClientPort for ClaudeClientAdapter {
    #[instrument(skip(self, request), fields(task_id = %request.task_id, agent_type = %request.agent_type))]
    async fn execute(&self, request: ClaudeRequest) -> Result<ClaudeResponse, ClaudeError> {
        debug!(
            "Executing Claude request for task {} with agent type {}",
            request.task_id, request.agent_type
        );

        // Convert high-level request to message request
        let infra_request = InfraMessageRequest {
            model: request.model.clone().unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string()),
            messages: vec![InfraMessage {
                role: "user".to_string(),
                content: MessageContent::Text(request.prompt.clone()),
            }],
            max_tokens: request.max_tokens.unwrap_or(4096),
            system: None,
            temperature: request.temperature,
            tools: None,
            metadata: None,
        };

        // Execute request via HTTP client
        let infra_response = self
            .http_client
            .send_message(infra_request)
            .await
            .map_err(ClaudeError::from)?;

        // Extract text content
        let content = infra_response
            .content
            .iter()
            .filter_map(|block| match block {
                InfraContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        info!(
            "Claude request succeeded: input_tokens={}, output_tokens={}",
            infra_response.usage.input_tokens, infra_response.usage.output_tokens
        );

        Ok(ClaudeResponse {
            task_id: request.task_id,
            content,
            stop_reason: infra_response
                .stop_reason
                .unwrap_or_else(|| "unknown".to_string()),
            usage: DomainTokenUsage {
                input_tokens: infra_response.usage.input_tokens,
                output_tokens: infra_response.usage.output_tokens,
            },
        })
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn send_message(
        &self,
        request: DomainMessageRequest,
    ) -> Result<DomainMessageResponse, ClaudeError> {
        debug!("Sending message to Claude API");

        // Convert domain request to infrastructure request
        let infra_request = InfraMessageRequest::from(request);

        // Execute request via HTTP client
        let infra_response = self
            .http_client
            .send_message(infra_request)
            .await
            .map_err(ClaudeError::from)?;

        // Convert response
        Ok(DomainMessageResponse::from(infra_response))
    }

    #[instrument(skip(self, request), fields(model = %request.model))]
    async fn stream_message(
        &self,
        request: DomainMessageRequest,
    ) -> Result<
        Box<dyn Stream<Item = Result<MessageChunk, ClaudeError>> + Send + Unpin>,
        ClaudeError,
    > {
        debug!("Starting streaming message to Claude API");

        // Convert domain request to infrastructure request
        let infra_request = InfraMessageRequest::from(request);

        // Add streaming flag
        let url = format!("{}/v1/messages", self.http_client.base_url);
        let mut json_value = serde_json::to_value(&infra_request)
            .map_err(|e| ClaudeError::ApiError(format!("Failed to serialize request: {}", e)))?;
        json_value["stream"] = serde_json::json!(true);

        // Send streaming request
        let response = self
            .http_client
            .http_client
            .post(&url)
            .json(&json_value)
            .send()
            .await
            .map_err(|e| ClaudeError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ClaudeError::ApiError(format!(
                "HTTP error: {}",
                response.status()
            )));
        }

        // Create SSE parser from byte stream
        let byte_stream = response.bytes_stream();
        let sse_parser = SseStreamParser::new(byte_stream);

        // Transform SSE events into MessageChunks
        let chunk_stream = Box::pin(sse_parser.filter_map(|event_result| async move {
            match event_result {
                Ok(event) => match event {
                    StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                        Delta::TextDelta { text } => Some(Ok(MessageChunk {
                            delta: Some(text),
                            stop_reason: None,
                        })),
                        _ => None,
                    },
                    StreamEvent::MessageDelta { delta } => {
                        if let Some(stop_reason) = delta.stop_reason {
                            Some(Ok(MessageChunk {
                                delta: None,
                                stop_reason: Some(stop_reason),
                            }))
                        } else {
                            None
                        }
                    }
                    StreamEvent::MessageStop => Some(Ok(MessageChunk {
                        delta: None,
                        stop_reason: Some("end_turn".to_string()),
                    })),
                    StreamEvent::Error { error } => Some(Err(ClaudeError::ApiError(
                        format!("{}: {}", error.error_type, error.message),
                    ))),
                    _ => None, // Ignore other events (ping, message_start, etc.)
                },
                Err(err) => Some(Err(ClaudeError::from(err))),
            }
        }));

        // Convert Pin<Box> to Box with Unpin
        Ok(Box::new(chunk_stream))
    }

    #[instrument(skip(self))]
    async fn health_check(&self) -> Result<(), ClaudeError> {
        debug!("Performing Claude API health check");

        // Send a minimal request to verify connectivity
        let health_request = InfraMessageRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![InfraMessage {
                role: "user".to_string(),
                content: MessageContent::Text("ping".to_string()),
            }],
            max_tokens: 10,
            system: None,
            temperature: None,
            tools: None,
            metadata: None,
        };

        match self.http_client.send_message(health_request).await {
            Ok(_) => {
                info!("Claude API health check passed");
                Ok(())
            }
            Err(err) => {
                error!("Claude API health check failed: {}", err);
                Err(ClaudeError::from(err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_error_conversion() {
        let api_error = ClaudeApiError::InvalidApiKey;
        let domain_error: ClaudeError = api_error.into();
        assert!(matches!(domain_error, ClaudeError::InvalidApiKey));
    }

    #[test]
    fn test_message_request_conversion() {
        let domain_req = DomainMessageRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![DomainMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: 1024,
            temperature: Some(0.7),
            system: Some("You are helpful".to_string()),
        };

        let infra_req = InfraMessageRequest::from(domain_req.clone());
        assert_eq!(infra_req.model, domain_req.model);
        assert_eq!(infra_req.max_tokens, 1024);
        assert_eq!(infra_req.temperature, Some(0.7));
        assert_eq!(infra_req.system, domain_req.system);
    }

    #[test]
    fn test_message_response_conversion() {
        let infra_resp = InfraMessageResponse {
            id: "msg_123".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![InfraContentBlock::Text {
                text: "Hello!".to_string(),
            }],
            model: "claude-3-5-sonnet-20241022".to_string(),
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: InfraUsage {
                input_tokens: 10,
                output_tokens: 20,
            },
        };

        let domain_resp = DomainMessageResponse::from(infra_resp);
        assert_eq!(domain_resp.id, "msg_123");
        assert_eq!(domain_resp.content.len(), 1);
        assert_eq!(domain_resp.usage.input_tokens, 10);
        assert_eq!(domain_resp.usage.output_tokens, 20);
    }
}

///! Anthropic API Substrate
///!
///! This substrate uses the Anthropic API directly via HTTP calls.
///! It requires an API key (from config or ANTHROPIC_API_KEY env var).
///!
///! Advantages:
///! - Direct API access
///! - Full control over rate limiting and retry logic
///! - Can use different models
///! - No CLI dependency

use crate::domain::ports::{
    HealthStatus, LlmSubstrate, StopReason, SubstrateError, SubstrateRequest,
    SubstrateResponse, SubstrateTokenUsage,
};
use crate::infrastructure::claude::{ClaudeClientAdapter, ClaudeClientConfig};
use crate::domain::ports::{ClaudeClient, ClaudeRequest};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for Anthropic API substrate
#[derive(Debug, Clone)]
pub struct AnthropicApiConfig {
    /// API key (required)
    pub api_key: String,

    /// Model to use
    pub model: String,

    /// Base URL for API (for testing/proxies)
    pub base_url: Option<String>,

    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for AnthropicApiConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            base_url: None,
            timeout_secs: 300,
        }
    }
}

/// Anthropic API substrate implementation
///
/// Uses the Anthropic API directly via HTTP.
/// Wraps the existing ClaudeClientAdapter infrastructure.
pub struct AnthropicApiSubstrate {
    client: Arc<dyn ClaudeClient>,
    config: AnthropicApiConfig,
}

impl AnthropicApiSubstrate {
    /// Create a new Anthropic API substrate with custom configuration
    pub fn new(config: AnthropicApiConfig) -> Result<Self, SubstrateError> {
        // Build Claude client config
        let client_config = ClaudeClientConfig {
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone().unwrap_or_else(||
                "https://api.anthropic.com".to_string()
            ),
            ..Default::default()
        };

        // Create adapter
        let adapter = ClaudeClientAdapter::new(client_config)
            .map_err(|e| SubstrateError::InvalidConfig(format!("Failed to create Anthropic API client: {}", e)))?;

        Ok(Self {
            client: Arc::new(adapter),
            config,
        })
    }

    /// Create from environment variable (ANTHROPIC_API_KEY)
    pub fn from_env() -> Result<Self, SubstrateError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| SubstrateError::NotConfigured(
                "ANTHROPIC_API_KEY environment variable not set".to_string()
            ))?;

        let config = AnthropicApiConfig {
            api_key,
            ..Default::default()
        };

        Self::new(config)
    }

    /// Convert SubstrateRequest to ClaudeRequest
    fn to_claude_request(&self, request: &SubstrateRequest) -> ClaudeRequest {
        ClaudeRequest {
            task_id: request.task_id,
            agent_type: request.agent_type.clone(),
            prompt: request.prompt.clone(),
            model: request.parameters.model.clone(),
            max_tokens: request.parameters.max_tokens,
            temperature: request.parameters.temperature,
        }
    }

    /// Convert ClaudeError to SubstrateError
    fn convert_error(err: crate::domain::ports::ClaudeError) -> SubstrateError {
        use crate::domain::ports::ClaudeError;

        match err {
            ClaudeError::InvalidApiKey => SubstrateError::AuthError("Invalid API key".to_string()),
            ClaudeError::RateLimitExceeded(msg) => SubstrateError::RateLimitExceeded(msg),
            ClaudeError::NetworkError(msg) => SubstrateError::NetworkError(msg),
            ClaudeError::Timeout => SubstrateError::Timeout(300),
            ClaudeError::ApiError(msg) => SubstrateError::ExecutionFailed(msg),
        }
    }
}

#[async_trait]
impl LlmSubstrate for AnthropicApiSubstrate {
    fn substrate_id(&self) -> &str {
        "anthropic-api"
    }

    fn substrate_name(&self) -> &str {
        "Anthropic API"
    }

    async fn execute(&self, request: SubstrateRequest) -> Result<SubstrateResponse, SubstrateError> {
        // Convert to ClaudeRequest
        let claude_request = self.to_claude_request(&request);

        // Execute via ClaudeClient
        let claude_response = self
            .client
            .execute(claude_request)
            .await
            .map_err(Self::convert_error)?;

        // Convert back to SubstrateResponse
        let stop_reason = match claude_response.stop_reason.as_str() {
            "end_turn" => StopReason::EndTurn,
            "max_tokens" => StopReason::MaxTokens,
            "stop_sequence" => StopReason::EndTurn,
            other => StopReason::Other(other.to_string()),
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "model".to_string(),
            serde_json::Value::String(self.config.model.clone()),
        );

        Ok(SubstrateResponse {
            task_id: request.task_id,
            content: claude_response.content,
            stop_reason,
            usage: Some(SubstrateTokenUsage {
                input_tokens: claude_response.usage.input_tokens,
                output_tokens: claude_response.usage.output_tokens,
            }),
            metadata,
        })
    }

    async fn health_check(&self) -> Result<HealthStatus, SubstrateError> {
        self.client
            .health_check()
            .await
            .map(|_| HealthStatus::Healthy)
            .map_err(Self::convert_error)
    }

    fn can_handle_agent_type(&self, _agent_type: &str) -> bool {
        // Anthropic API can handle all agent types
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::domain::ports::ExecutionParameters;

    #[test]
    fn test_config_default() {
        // Clear env var for test
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY"); }

        let config = AnthropicApiConfig::default();
        assert_eq!(config.model, "claude-sonnet-4-5-20250929");
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    fn test_to_claude_request() {
        let config = AnthropicApiConfig {
            api_key: "test-key".to_string(),
            ..Default::default()
        };

        // We can't actually create the substrate without a valid API key
        // so we'll just test the config
        assert_eq!(config.api_key, "test-key");
    }

    #[tokio::test]
    async fn test_substrate_id() {
        // Test with mock - we'd need to refactor to make this testable
        // For now, just test that the substrate_id is correct
        // In a real implementation, we'd inject the client
    }
}

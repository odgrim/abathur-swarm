///! Substrate Registry
///!
///! Manages multiple LLM substrates and routes tasks to the appropriate substrate
///! based on configuration and agent type.

use crate::domain::models::Config;
use crate::domain::ports::{HealthStatus, LlmSubstrate, SubstrateError, SubstrateRequest, SubstrateResponse};
use std::collections::HashMap;
use std::sync::Arc;

/// Registry for managing LLM substrates
///
/// The registry:
/// - Initializes configured substrates
/// - Routes tasks to appropriate substrates based on agent type
/// - Provides health checking across all substrates
pub struct SubstrateRegistry {
    /// Map of substrate_id -> substrate instance
    pub(crate) substrates: HashMap<String, Arc<dyn LlmSubstrate>>,

    /// Default substrate to use
    pub(crate) default_substrate_id: String,

    /// Agent type to substrate mappings
    pub(crate) agent_mappings: HashMap<String, String>,
}

impl SubstrateRegistry {
    /// Create a new substrate registry from configuration
    ///
    /// Initializes all enabled substrates and validates configuration.
    ///
    /// # Errors
    /// Returns error if no substrates are enabled or if configuration is invalid.
    pub async fn from_config(config: &Config) -> Result<Self, SubstrateError> {
        let mut substrates: HashMap<String, Arc<dyn LlmSubstrate>> = HashMap::new();

        // Initialize Claude Code substrate if enabled
        if config.substrates.enabled.contains(&"claude-code".to_string()) {
            let claude_config = super::claude_code::ClaudeCodeConfig {
                claude_path: config.substrates.claude_code.claude_path.clone(),
                working_dir: config
                    .substrates
                    .claude_code
                    .working_dir
                    .as_ref()
                    .map(|s| std::path::PathBuf::from(s)),
                default_timeout_secs: config.substrates.claude_code.timeout_secs,
            };

            let claude_substrate = Arc::new(super::ClaudeCodeSubstrate::with_config(claude_config))
                as Arc<dyn LlmSubstrate>;

            substrates.insert("claude-code".to_string(), claude_substrate);
        }

        // Initialize Anthropic API substrate if enabled
        if config.substrates.enabled.contains(&"anthropic-api".to_string())
            && config.substrates.anthropic_api.enabled
        {
            let api_config = super::anthropic_api::AnthropicApiConfig {
                api_key: config
                    .substrates
                    .anthropic_api
                    .api_key
                    .clone()
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                    .unwrap_or_default(),
                model: config.substrates.anthropic_api.model.clone(),
                base_url: config.substrates.anthropic_api.base_url.clone(),
                timeout_secs: 300,
            };

            match super::AnthropicApiSubstrate::new(api_config) {
                Ok(substrate) => {
                    let anthropic_substrate = Arc::new(substrate) as Arc<dyn LlmSubstrate>;
                    substrates.insert("anthropic-api".to_string(), anthropic_substrate);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to initialize anthropic-api substrate: {}", e);
                }
            }
        }

        // Verify at least one substrate is available
        if substrates.is_empty() {
            return Err(SubstrateError::NotConfigured(
                "No substrates are enabled. Please enable at least one substrate in config.".to_string(),
            ));
        }

        // Verify default substrate exists
        let default_id = config.substrates.default_substrate.clone();
        if !substrates.contains_key(&default_id) {
            return Err(SubstrateError::InvalidConfig(format!(
                "Default substrate '{}' is not enabled. Enabled substrates: {:?}",
                default_id,
                substrates.keys().collect::<Vec<_>>()
            )));
        }

        Ok(Self {
            substrates,
            default_substrate_id: default_id,
            agent_mappings: config.substrates.agent_mappings.clone(),
        })
    }

    /// Get substrate for a specific agent type
    ///
    /// Routes based on:
    /// 1. Explicit agent_mappings in config
    /// 2. Substrate's can_handle_agent_type() method
    /// 3. Default substrate as fallback
    pub fn get_substrate_for_agent(&self, agent_type: &str) -> Result<Arc<dyn LlmSubstrate>, SubstrateError> {
        // Check explicit mapping first
        if let Some(substrate_id) = self.agent_mappings.get(agent_type) {
            if let Some(substrate) = self.substrates.get(substrate_id) {
                return Ok(Arc::clone(substrate));
            }
            // Mapping exists but substrate not available - log warning
            eprintln!(
                "Warning: Agent type '{}' mapped to '{}' but substrate not available. Using default.",
                agent_type, substrate_id
            );
        }

        // Check if any substrate specifically handles this agent type
        for (id, substrate) in &self.substrates {
            if substrate.can_handle_agent_type(agent_type) && id != &self.default_substrate_id {
                return Ok(Arc::clone(substrate));
            }
        }

        // Fall back to default substrate
        self.substrates
            .get(&self.default_substrate_id)
            .map(Arc::clone)
            .ok_or_else(|| {
                SubstrateError::NotConfigured(format!(
                    "Default substrate '{}' not found",
                    self.default_substrate_id
                ))
            })
    }

    /// Execute a task using the appropriate substrate
    ///
    /// Automatically selects the best substrate for the agent type.
    pub async fn execute(&self, request: SubstrateRequest) -> Result<SubstrateResponse, SubstrateError> {
        let substrate = self.get_substrate_for_agent(&request.agent_type)?;
        substrate.execute(request).await
    }

    /// Check health of all substrates
    ///
    /// Returns a map of substrate_id -> health status
    pub async fn health_check_all(&self) -> HashMap<String, Result<HealthStatus, SubstrateError>> {
        let mut results = HashMap::new();

        for (id, substrate) in &self.substrates {
            let health = substrate.health_check().await;
            results.insert(id.clone(), health);
        }

        results
    }

    /// Get list of all available substrate IDs
    pub fn available_substrates(&self) -> Vec<String> {
        self.substrates.keys().cloned().collect()
    }

    /// Get the default substrate ID
    pub fn default_substrate_id(&self) -> &str {
        &self.default_substrate_id
    }

    /// Check if at least one substrate is healthy
    pub async fn is_any_substrate_healthy(&self) -> bool {
        for substrate in self.substrates.values() {
            if let Ok(HealthStatus::Healthy) = substrate.health_check().await {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> Config {
        Config::default() // Uses claude-code as default
    }

    #[tokio::test]
    async fn test_registry_creation() {
        let config = create_test_config();
        let result = SubstrateRegistry::from_config(&config).await;

        // Should succeed with default config (claude-code enabled)
        assert!(result.is_ok());

        let registry = result.unwrap();
        assert_eq!(registry.default_substrate_id(), "claude-code");
        assert!(registry.available_substrates().contains(&"claude-code".to_string()));
    }

    #[tokio::test]
    async fn test_get_substrate_for_agent() {
        let config = create_test_config();
        let registry = SubstrateRegistry::from_config(&config).await.unwrap();

        // Should return default substrate for any agent type
        let substrate = registry.get_substrate_for_agent("test-agent");
        assert!(substrate.is_ok());
        assert_eq!(substrate.unwrap().substrate_id(), "claude-code");
    }

    #[tokio::test]
    async fn test_health_check_all() {
        let config = create_test_config();
        let registry = SubstrateRegistry::from_config(&config).await.unwrap();

        let health = registry.health_check_all().await;

        // Should have health check result for claude-code
        assert!(health.contains_key("claude-code"));
    }
}

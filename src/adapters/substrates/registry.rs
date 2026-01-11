//! Substrate registry and factory.

use crate::domain::models::SubstrateType;
use crate::domain::ports::{Substrate, SubstrateFactory};

use super::anthropic_api::{AnthropicApiConfig, AnthropicApiSubstrate};
use super::claude_code::{ClaudeCodeConfig, ClaudeCodeSubstrate};
use super::mock::MockSubstrate;

/// Registry of available substrates.
pub struct SubstrateRegistry {
    claude_code_config: Option<ClaudeCodeConfig>,
    anthropic_api_config: Option<AnthropicApiConfig>,
}

impl SubstrateRegistry {
    pub fn new() -> Self {
        Self {
            claude_code_config: Some(ClaudeCodeConfig::default()),
            anthropic_api_config: Some(AnthropicApiConfig::default()),
        }
    }

    pub fn with_claude_code_config(mut self, config: ClaudeCodeConfig) -> Self {
        self.claude_code_config = Some(config);
        self
    }

    pub fn with_anthropic_api_config(mut self, config: AnthropicApiConfig) -> Self {
        self.anthropic_api_config = Some(config);
        self
    }

    /// Create a substrate by type.
    pub fn create_by_type(&self, substrate_type: SubstrateType) -> Box<dyn Substrate> {
        match substrate_type {
            SubstrateType::ClaudeCode => {
                let config = self.claude_code_config.clone().unwrap_or_default();
                Box::new(ClaudeCodeSubstrate::new(config))
            }
            SubstrateType::AnthropicApi => {
                let config = self.anthropic_api_config.clone().unwrap_or_default();
                // If we can't create the API substrate, fall back to Claude Code
                match AnthropicApiSubstrate::new(config) {
                    Ok(substrate) => Box::new(substrate),
                    Err(_) => {
                        let config = self.claude_code_config.clone().unwrap_or_default();
                        Box::new(ClaudeCodeSubstrate::new(config))
                    }
                }
            }
            SubstrateType::Mock => {
                Box::new(MockSubstrate::new())
            }
        }
    }

    /// Get the default substrate (Claude Code).
    pub fn default_substrate(&self) -> Box<dyn Substrate> {
        self.create_by_type(SubstrateType::ClaudeCode)
    }

    /// Get a mock substrate for testing.
    pub fn mock_substrate() -> Box<dyn Substrate> {
        Box::new(MockSubstrate::new())
    }
}

impl Default for SubstrateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SubstrateFactory for SubstrateRegistry {
    fn create(&self, substrate_type: &str) -> Option<Box<dyn Substrate>> {
        SubstrateType::from_str(substrate_type)
            .map(|t| self.create_by_type(t))
    }

    fn available_types(&self) -> Vec<&'static str> {
        vec!["claude_code", "anthropic_api", "mock"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_by_type() {
        let registry = SubstrateRegistry::new();

        let claude = registry.create_by_type(SubstrateType::ClaudeCode);
        assert_eq!(claude.name(), "claude_code");

        let mock = registry.create_by_type(SubstrateType::Mock);
        assert_eq!(mock.name(), "mock");
    }

    #[test]
    fn test_factory_interface() {
        let registry = SubstrateRegistry::new();

        let substrate = registry.create("claude_code");
        assert!(substrate.is_some());

        let substrate = registry.create("mock");
        assert!(substrate.is_some());

        let substrate = registry.create("invalid");
        assert!(substrate.is_none());
    }

    #[test]
    fn test_available_types() {
        let registry = SubstrateRegistry::new();
        let types = registry.available_types();

        assert!(types.contains(&"claude_code"));
        assert!(types.contains(&"mock"));
    }
}

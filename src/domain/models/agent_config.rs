///! Agent Configuration Loading
///!
///! Loads agent contracts and validation requirements from agents.yaml configuration.
///! This replaces hardcoded agent contracts with a dynamic, configurable system.

use super::task::{ValidationRequirement, WorkflowExpectations};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// Global agent configuration singleton
static AGENT_CONFIG: OnceLock<AgentConfiguration> = OnceLock::new();

/// Agent contract configuration loaded from YAML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfiguration {
    pub agents: HashMap<String, AgentContract>,
    #[serde(default)]
    pub defaults: DefaultConfig,
}

/// Individual agent contract definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContract {
    pub description: String,
    pub validation_type: ValidationType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<ContractValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub testing: Option<TestingValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_chain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Validation type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValidationType {
    None,
    Contract,
    Testing,
}

/// Contract validation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractValidation {
    pub must_spawn_children: bool,
    pub min_children: usize,
    pub expected_child_types: Vec<String>,
}

/// Testing validation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestingValidation {
    pub validator_agent: String,
    pub test_commands: Vec<String>,
    #[serde(default = "default_worktree_required")]
    pub worktree_required: bool,
    pub max_remediation_cycles: usize,
}

fn default_worktree_required() -> bool {
    true  // Most implementation agents need a worktree
}

/// Default configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultConfig {
    pub validation_type: ValidationType,
    #[serde(default = "default_model")]
    pub model: String,
    pub max_remediation_cycles: usize,
}

impl Default for DefaultConfig {
    fn default() -> Self {
        Self {
            validation_type: ValidationType::None,
            model: default_model(),
            max_remediation_cycles: 3,
        }
    }
}

fn default_model() -> String {
    "sonnet".to_string()
}

impl AgentConfiguration {
    /// Load agent configuration from YAML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .context("Failed to read agents.yaml")?;

        let config: AgentConfiguration = serde_yaml::from_str(&content)
            .context("Failed to parse agents.yaml")?;

        Ok(config)
    }

    /// Initialize global configuration (call once at startup)
    pub fn init_global<P: AsRef<Path>>(path: P) -> Result<()> {
        let config = Self::load_from_file(path)?;
        AGENT_CONFIG.set(config)
            .map_err(|_| anyhow::anyhow!("Agent configuration already initialized"))?;
        Ok(())
    }

    /// Get global configuration (after initialization)
    pub fn global() -> Option<&'static AgentConfiguration> {
        AGENT_CONFIG.get()
    }

    /// Get validation requirement for an agent type
    pub fn get_validation_requirement(&self, agent_type: &str) -> ValidationRequirement {
        // Look up agent in configuration
        if let Some(agent) = self.agents.get(agent_type) {
            match agent.validation_type {
                ValidationType::None => ValidationRequirement::None,

                ValidationType::Contract => {
                    if let Some(ref contract) = agent.contract {
                        ValidationRequirement::Contract {
                            must_spawn_children: contract.must_spawn_children,
                            expected_child_types: contract.expected_child_types.clone(),
                            min_children: contract.min_children,
                        }
                    } else {
                        // Config error: contract type but no contract details
                        ValidationRequirement::None
                    }
                }

                ValidationType::Testing => {
                    if let Some(ref testing) = agent.testing {
                        ValidationRequirement::Testing {
                            validator_agent: testing.validator_agent.clone(),
                            test_commands: testing.test_commands.clone(),
                            worktree_required: testing.worktree_required,
                            max_remediation_cycles: testing.max_remediation_cycles,
                        }
                    } else {
                        // Config error: testing type but no testing details
                        ValidationRequirement::None
                    }
                }
            }
        } else {
            // Unknown agent type - use default
            match self.defaults.validation_type {
                ValidationType::None => ValidationRequirement::None,
                _ => ValidationRequirement::None, // Default to none for unknown agents
            }
        }
    }

    /// Get workflow expectations for an agent type
    pub fn get_workflow_expectations(&self, agent_type: &str) -> Option<WorkflowExpectations> {
        match self.get_validation_requirement(agent_type) {
            ValidationRequirement::Contract {
                must_spawn_children,
                expected_child_types,
                min_children,
            } => Some(WorkflowExpectations {
                must_spawn_child: must_spawn_children,
                expected_child_types,
                min_children,
                max_children: None,
            }),
            _ => None,
        }
    }

    /// Check if an agent type requires validation
    pub fn requires_validation(&self, agent_type: &str) -> bool {
        !matches!(
            self.get_validation_requirement(agent_type),
            ValidationRequirement::None
        )
    }

    /// Check if an agent type requires test-based validation
    pub fn requires_testing(&self, agent_type: &str) -> bool {
        matches!(
            self.get_validation_requirement(agent_type),
            ValidationRequirement::Testing { .. }
        )
    }

    /// Check if an agent type requires contract validation
    pub fn requires_contract(&self, agent_type: &str) -> bool {
        matches!(
            self.get_validation_requirement(agent_type),
            ValidationRequirement::Contract { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_config() -> String {
        r#"
agents:
  test-orchestrator:
    description: "Test orchestrator"
    validation_type: contract
    contract:
      must_spawn_children: true
      min_children: 2
      expected_child_types:
        - test-worker
        - test-validator

  test-worker:
    description: "Test worker"
    validation_type: none

  test-specialist:
    description: "Test specialist"
    validation_type: testing
    testing:
      validator_agent: test-validator
      test_commands:
        - cargo test
        - cargo clippy
      worktree_required: true
      max_remediation_cycles: 3

defaults:
  validation_type: none
  model: sonnet
  max_remediation_cycles: 3
"#.to_string()
    }

    #[test]
    fn test_load_from_yaml() {
        let config_yaml = create_test_config();
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        let config = AgentConfiguration::load_from_file(temp_file.path()).unwrap();

        assert_eq!(config.agents.len(), 3);
        assert!(config.agents.contains_key("test-orchestrator"));
        assert!(config.agents.contains_key("test-worker"));
    }

    #[test]
    fn test_contract_validation() {
        let config_yaml = create_test_config();
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        let config = AgentConfiguration::load_from_file(temp_file.path()).unwrap();

        let req = config.get_validation_requirement("test-orchestrator");
        assert!(matches!(req, ValidationRequirement::Contract { .. }));

        if let ValidationRequirement::Contract {
            must_spawn_children,
            expected_child_types,
            min_children,
        } = req
        {
            assert!(must_spawn_children);
            assert_eq!(min_children, 2);
            assert_eq!(expected_child_types.len(), 2);
            assert!(expected_child_types.contains(&"test-worker".to_string()));
        }
    }

    #[test]
    fn test_testing_validation() {
        let config_yaml = create_test_config();
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        let config = AgentConfiguration::load_from_file(temp_file.path()).unwrap();

        let req = config.get_validation_requirement("test-specialist");
        assert!(matches!(req, ValidationRequirement::Testing { .. }));

        if let ValidationRequirement::Testing {
            validator_agent,
            test_commands,
            worktree_required,
            max_remediation_cycles,
        } = req
        {
            assert_eq!(validator_agent, "test-validator");
            assert_eq!(test_commands.len(), 2);
            assert!(worktree_required);
            assert_eq!(max_remediation_cycles, 3);
        }
    }

    #[test]
    fn test_no_validation() {
        let config_yaml = create_test_config();
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        let config = AgentConfiguration::load_from_file(temp_file.path()).unwrap();

        let req = config.get_validation_requirement("test-worker");
        assert!(matches!(req, ValidationRequirement::None));
    }

    #[test]
    fn test_unknown_agent_uses_default() {
        let config_yaml = create_test_config();
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        let config = AgentConfiguration::load_from_file(temp_file.path()).unwrap();

        let req = config.get_validation_requirement("unknown-agent");
        assert!(matches!(req, ValidationRequirement::None));
    }

    #[test]
    fn test_validation_helpers() {
        let config_yaml = create_test_config();
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        let config = AgentConfiguration::load_from_file(temp_file.path()).unwrap();

        assert!(config.requires_validation("test-orchestrator"));
        assert!(config.requires_contract("test-orchestrator"));
        assert!(!config.requires_testing("test-orchestrator"));

        assert!(config.requires_validation("test-specialist"));
        assert!(config.requires_testing("test-specialist"));
        assert!(!config.requires_contract("test-specialist"));

        assert!(!config.requires_validation("test-worker"));
        assert!(!config.requires_contract("test-worker"));
        assert!(!config.requires_testing("test-worker"));
    }
}

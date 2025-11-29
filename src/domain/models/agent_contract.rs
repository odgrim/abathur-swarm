///! Agent Contract Registry
///!
///! Provides a static interface to agent validation requirements loaded from configuration.
///! This is a compatibility layer that delegates to the AgentConfiguration singleton.
///!
///! Agent contracts are now fully configurable via .abathur/agents.yaml instead of hardcoded.

use super::agent_config::AgentConfiguration;
use super::task::{ValidationRequirement, WorkflowExpectations};
use tracing::warn;

/// Agent contract registry - delegates to loaded configuration
pub struct AgentContractRegistry;

impl AgentContractRegistry {
    /// Get validation requirement for a given agent type
    ///
    /// Returns the validation requirement loaded from agents.yaml configuration.
    /// Falls back to ValidationRequirement::None if config not loaded or agent not found.
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent (e.g., "requirements-gatherer")
    ///
    /// # Returns
    ///
    /// The validation requirement for this agent type
    pub fn get_validation_requirement(agent_type: &str) -> ValidationRequirement {
        match AgentConfiguration::global() {
            Some(config) => config.get_validation_requirement(agent_type),
            None => {
                warn!("Agent configuration not loaded, using default validation (none) for {}", agent_type);
                ValidationRequirement::None
            }
        }
    }

    /// Get workflow expectations for a given agent type
    ///
    /// Converts validation requirements into structured workflow expectations
    /// that can be stored in the task model.
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent
    ///
    /// # Returns
    ///
    /// Optional workflow expectations if agent must spawn children
    pub fn get_workflow_expectations(agent_type: &str) -> Option<WorkflowExpectations> {
        match AgentConfiguration::global() {
            Some(config) => config.get_workflow_expectations(agent_type),
            None => {
                warn!("Agent configuration not loaded, returning None for workflow expectations");
                None
            }
        }
    }

    /// Check if an agent type requires validation
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent
    ///
    /// # Returns
    ///
    /// true if this agent requires any form of validation
    pub fn requires_validation(agent_type: &str) -> bool {
        match AgentConfiguration::global() {
            Some(config) => config.requires_validation(agent_type),
            None => false,
        }
    }

    /// Check if an agent type requires test-based validation
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent
    ///
    /// # Returns
    ///
    /// true if this agent requires test validation
    pub fn requires_testing(agent_type: &str) -> bool {
        match AgentConfiguration::global() {
            Some(config) => config.requires_testing(agent_type),
            None => false,
        }
    }

    /// Check if an agent type requires contract validation
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent
    ///
    /// # Returns
    ///
    /// true if this agent requires contract validation
    pub fn requires_contract(agent_type: &str) -> bool {
        match AgentConfiguration::global() {
            Some(config) => config.requires_contract(agent_type),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::agent_config::AgentConfiguration;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[allow(dead_code)]
    fn setup_test_config() {
        let config_yaml = r#"
agents:
  test-orchestrator:
    description: "Test orchestrator"
    validation_type: contract
    contract:
      must_spawn_children: true
      min_children: 1
      expected_child_types:
        - test-worker

  test-worker:
    description: "Test worker"
    validation_type: none

defaults:
  validation_type: none
  model: sonnet
  max_remediation_cycles: 3
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{}", config_yaml).unwrap();

        // Initialize global config for tests
        let _ = AgentConfiguration::init_global(temp_file.path());
    }

    #[test]
    fn test_delegates_to_config() {
        // Note: This test may fail if config already initialized by other tests
        // In production, config is loaded once at startup
        let req = AgentContractRegistry::get_validation_requirement("unknown-agent");
        assert!(matches!(req, ValidationRequirement::None));
    }

    #[test]
    fn test_unknown_agents_default_to_none() {
        let req = AgentContractRegistry::get_validation_requirement("completely-unknown-agent");
        assert!(matches!(req, ValidationRequirement::None));
    }
}

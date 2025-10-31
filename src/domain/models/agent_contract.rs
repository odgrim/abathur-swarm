///! Agent Contract Registry
///!
///! Defines validation requirements and workflow expectations for different agent types.
///! This registry is the single source of truth for which agents must spawn children,
///! which agents require test validation, and what the expectations are for each.

use super::task::{ValidationRequirement, WorkflowExpectations};

/// Agent contract registry - maps agent types to their validation requirements
pub struct AgentContractRegistry;

impl AgentContractRegistry {
    /// Get validation requirement for a given agent type
    ///
    /// Returns the validation requirement that should be enforced after
    /// this agent type completes execution.
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent (e.g., "requirements-gatherer")
    ///
    /// # Returns
    ///
    /// The validation requirement for this agent type
    pub fn get_validation_requirement(agent_type: &str) -> ValidationRequirement {
        match agent_type {
            // ========================================
            // Workflow Orchestration Agents (Contract Validation)
            // ========================================

            "requirements-gatherer" => ValidationRequirement::Contract {
                must_spawn_children: true,
                expected_child_types: vec!["technical-architect".to_string()],
                min_children: 1,
            },

            "technical-architect" => ValidationRequirement::Contract {
                must_spawn_children: true,
                expected_child_types: vec![
                    "technical-requirements-specialist".to_string(),
                ],
                min_children: 1,
            },

            "task-planner" => ValidationRequirement::Contract {
                must_spawn_children: true,
                // Empty expected_child_types allows task-planner to spawn ANY agent type
                // Task planner determines which specialists are needed dynamically
                expected_child_types: vec![],
                min_children: 1,
            },

            // ========================================
            // Orchestration Support Agents (No Validation)
            // ========================================
            //
            // Note: Language-specific implementation agents (e.g., rust-*, python-*, etc.)
            // should NOT be hardcoded here. Their validation requirements should be
            // determined dynamically by the task-planner based on project configuration.
            // Only core orchestration agents are defined in this registry.

            "git-worktree-merge-orchestrator" |
            "git-branch-cleanup-specialist" |
            "technical-requirements-specialist" => {
                ValidationRequirement::None
            }

            // ========================================
            // Default: No Validation
            // ========================================
            _ => ValidationRequirement::None,
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
        match Self::get_validation_requirement(agent_type) {
            ValidationRequirement::Contract {
                must_spawn_children,
                expected_child_types,
                min_children,
            } => Some(WorkflowExpectations {
                must_spawn_child: must_spawn_children,
                expected_child_types,
                min_children,
                max_children: None, // Unlimited by default
            }),
            _ => None,
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
        !matches!(
            Self::get_validation_requirement(agent_type),
            ValidationRequirement::None
        )
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
        matches!(
            Self::get_validation_requirement(agent_type),
            ValidationRequirement::Testing { .. }
        )
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
        matches!(
            Self::get_validation_requirement(agent_type),
            ValidationRequirement::Contract { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requirements_gatherer_has_contract() {
        let req = AgentContractRegistry::get_validation_requirement("requirements-gatherer");
        assert!(matches!(req, ValidationRequirement::Contract { .. }));

        if let ValidationRequirement::Contract { must_spawn_children, expected_child_types, min_children } = req {
            assert!(must_spawn_children);
            assert_eq!(min_children, 1);
            assert!(expected_child_types.contains(&"technical-architect".to_string()));
        }
    }

    #[test]
    fn test_language_specific_agents_not_hardcoded() {
        // Language-specific agents should NOT be hardcoded in the registry
        // They should default to ValidationRequirement::None and have their
        // requirements determined dynamically by the task-planner
        let req = AgentContractRegistry::get_validation_requirement("rust-specialist");
        assert!(matches!(req, ValidationRequirement::None));

        let req = AgentContractRegistry::get_validation_requirement("python-specialist");
        assert!(matches!(req, ValidationRequirement::None));
    }

    #[test]
    fn test_orchestration_agents_have_no_validation() {
        let req = AgentContractRegistry::get_validation_requirement("git-worktree-merge-orchestrator");
        assert!(matches!(req, ValidationRequirement::None));

        let req = AgentContractRegistry::get_validation_requirement("technical-requirements-specialist");
        assert!(matches!(req, ValidationRequirement::None));
    }

    #[test]
    fn test_workflow_expectations_conversion() {
        let expectations = AgentContractRegistry::get_workflow_expectations("requirements-gatherer");
        assert!(expectations.is_some());

        let expectations = expectations.unwrap();
        assert!(expectations.must_spawn_child);
        assert_eq!(expectations.min_children, 1);
    }

    #[test]
    fn test_requires_validation_helpers() {
        assert!(AgentContractRegistry::requires_validation("requirements-gatherer"));
        assert!(AgentContractRegistry::requires_contract("requirements-gatherer"));
        assert!(!AgentContractRegistry::requires_testing("requirements-gatherer"));

        // Language-specific agents are not hardcoded, so they default to no validation
        assert!(!AgentContractRegistry::requires_validation("rust-specialist"));
        assert!(!AgentContractRegistry::requires_testing("rust-specialist"));
        assert!(!AgentContractRegistry::requires_contract("rust-specialist"));

        assert!(!AgentContractRegistry::requires_validation("git-worktree-merge-orchestrator"));
    }
}

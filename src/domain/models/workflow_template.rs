//! Workflow template abstraction for configurable workflow spines.
//!
//! A `WorkflowTemplate` defines a sequence of phases that the Overmind
//! follows when executing a task. It controls agent roles, tool grants,
//! read-only flags, and phase dependencies.

use serde::{Deserialize, Serialize};

/// Valid tool names that can be assigned to workflow phases.
const VALID_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "shell",
    "glob",
    "grep",
    "memory",
    "task_status",
    "tasks",
    "agents",
];

/// How a phase depends on previous phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseDependency {
    /// No dependency on other phases (runs first).
    Root,
    /// Depends on the immediately preceding phase.
    Sequential,
    /// Depends on all previous phases completing.
    AllPrevious,
}

impl Default for PhaseDependency {
    fn default() -> Self {
        Self::Sequential
    }
}

/// A single phase within a workflow template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPhase {
    /// Phase name (e.g., "research", "plan", "implement", "review").
    pub name: String,
    /// Description of what this phase does.
    pub description: String,
    /// Role description for the agent executing this phase.
    pub role: String,
    /// Tools granted to the agent for this phase.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether the agent is read-only (produces findings via memory, not code).
    #[serde(default)]
    pub read_only: bool,
    /// How this phase depends on previous phases.
    #[serde(default)]
    pub dependency: PhaseDependency,
}

/// A workflow template defining the phase sequence for task execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTemplate {
    /// Unique name for this workflow (e.g., "code", "docs", "review-only").
    pub name: String,
    /// Description of when to use this workflow.
    #[serde(default)]
    pub description: String,
    /// Ordered list of phases in this workflow.
    pub phases: Vec<WorkflowPhase>,
}

impl WorkflowTemplate {
    /// Returns the built-in 4-phase code workflow (research -> plan -> implement -> review).
    pub fn default_code_workflow() -> Self {
        Self {
            name: "code".to_string(),
            description: "Standard 4-phase code workflow: research, plan, implement, review"
                .to_string(),
            phases: vec![
                WorkflowPhase {
                    name: "research".to_string(),
                    description: "Explore the codebase, understand existing patterns, identify files that need to change".to_string(),
                    role: "Read-only research agent that explores codebases and reports findings".to_string(),
                    tools: vec!["read".to_string(), "glob".to_string(), "grep".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                },
                WorkflowPhase {
                    name: "plan".to_string(),
                    description: "Draft a concrete implementation plan based on research findings".to_string(),
                    role: "Domain-specific planning agent that designs implementation approach".to_string(),
                    tools: vec!["read".to_string(), "glob".to_string(), "grep".to_string(), "memory".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Sequential,
                },
                WorkflowPhase {
                    name: "implement".to_string(),
                    description: "Execute the implementation plan with specific code changes".to_string(),
                    role: "Implementation specialist that writes clean, idiomatic code".to_string(),
                    tools: vec!["read".to_string(), "write".to_string(), "edit".to_string(), "shell".to_string(), "glob".to_string(), "grep".to_string(), "memory".to_string()],
                    read_only: false,
                    dependency: PhaseDependency::Sequential,
                },
                WorkflowPhase {
                    name: "review".to_string(),
                    description: "Review for correctness, edge cases, test coverage, and adherence to the plan".to_string(),
                    role: "Code review specialist that validates implementation quality".to_string(),
                    tools: vec!["read".to_string(), "glob".to_string(), "grep".to_string(), "shell".to_string(), "memory".to_string()],
                    read_only: false,
                    dependency: PhaseDependency::Sequential,
                },
            ],
        }
    }

    /// Validate the workflow template.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Workflow template name cannot be empty".to_string());
        }

        if self.phases.is_empty() {
            return Err(format!(
                "Workflow template '{}' must have at least one phase",
                self.name
            ));
        }

        for (i, phase) in self.phases.iter().enumerate() {
            if phase.name.is_empty() {
                return Err(format!(
                    "Phase {} in workflow '{}' has an empty name",
                    i, self.name
                ));
            }

            for tool in &phase.tools {
                if !VALID_TOOLS.contains(&tool.as_str()) {
                    return Err(format!(
                        "Phase '{}' in workflow '{}' has invalid tool '{}'. Valid tools: {}",
                        phase.name,
                        self.name,
                        tool,
                        VALID_TOOLS.join(", ")
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_code_workflow() {
        let wf = WorkflowTemplate::default_code_workflow();
        assert_eq!(wf.name, "code");
        assert_eq!(wf.phases.len(), 4);
        assert_eq!(wf.phases[0].name, "research");
        assert_eq!(wf.phases[1].name, "plan");
        assert_eq!(wf.phases[2].name, "implement");
        assert_eq!(wf.phases[3].name, "review");
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_phase_dependencies() {
        let wf = WorkflowTemplate::default_code_workflow();
        assert_eq!(wf.phases[0].dependency, PhaseDependency::Root);
        assert_eq!(wf.phases[1].dependency, PhaseDependency::Sequential);
        assert_eq!(wf.phases[2].dependency, PhaseDependency::Sequential);
        assert_eq!(wf.phases[3].dependency, PhaseDependency::Sequential);
    }

    #[test]
    fn test_read_only_flags() {
        let wf = WorkflowTemplate::default_code_workflow();
        assert!(wf.phases[0].read_only);  // research
        assert!(wf.phases[1].read_only);  // plan
        assert!(!wf.phases[2].read_only); // implement
        assert!(!wf.phases[3].read_only); // review
    }

    #[test]
    fn test_validate_empty_name() {
        let wf = WorkflowTemplate {
            name: String::new(),
            description: String::new(),
            phases: vec![WorkflowPhase {
                name: "test".to_string(),
                description: "test".to_string(),
                role: "test".to_string(),
                tools: vec!["read".to_string()],
                read_only: false,
                dependency: PhaseDependency::Root,
            }],
        };
        assert!(wf.validate().is_err());
    }

    #[test]
    fn test_validate_no_phases() {
        let wf = WorkflowTemplate {
            name: "empty".to_string(),
            description: String::new(),
            phases: vec![],
        };
        assert!(wf.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_tool() {
        let wf = WorkflowTemplate {
            name: "bad-tools".to_string(),
            description: String::new(),
            phases: vec![WorkflowPhase {
                name: "test".to_string(),
                description: "test".to_string(),
                role: "test".to_string(),
                tools: vec!["invalid_tool".to_string()],
                read_only: false,
                dependency: PhaseDependency::Root,
            }],
        };
        assert!(wf.validate().is_err());
        let err = wf.validate().unwrap_err();
        assert!(err.contains("invalid_tool"));
    }

    #[test]
    fn test_validate_empty_phase_name() {
        let wf = WorkflowTemplate {
            name: "test".to_string(),
            description: String::new(),
            phases: vec![WorkflowPhase {
                name: String::new(),
                description: "test".to_string(),
                role: "test".to_string(),
                tools: vec!["read".to_string()],
                read_only: false,
                dependency: PhaseDependency::Root,
            }],
        };
        assert!(wf.validate().is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let wf = WorkflowTemplate::default_code_workflow();
        let json = serde_json::to_string(&wf).unwrap();
        let deserialized: WorkflowTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(wf, deserialized);
    }

    #[test]
    fn test_phase_dependency_default() {
        let dep: PhaseDependency = Default::default();
        assert_eq!(dep, PhaseDependency::Sequential);
    }

    #[test]
    fn test_serde_defaults() {
        // Verify that omitted fields get defaults
        let json = r#"{"name":"test","description":"","role":"tester","tools":["read"]}"#;
        let phase: WorkflowPhase = serde_json::from_str(json).unwrap();
        assert!(!phase.read_only);
        assert_eq!(phase.dependency, PhaseDependency::Sequential);
    }

    #[test]
    fn test_custom_workflow() {
        let wf = WorkflowTemplate {
            name: "docs".to_string(),
            description: "Documentation-only workflow".to_string(),
            phases: vec![
                WorkflowPhase {
                    name: "research".to_string(),
                    description: "Research the codebase".to_string(),
                    role: "Codebase researcher".to_string(),
                    tools: vec!["read".to_string(), "glob".to_string(), "grep".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                },
                WorkflowPhase {
                    name: "write-docs".to_string(),
                    description: "Write documentation".to_string(),
                    role: "Documentation writer".to_string(),
                    tools: vec!["read".to_string(), "write".to_string(), "edit".to_string()],
                    read_only: false,
                    dependency: PhaseDependency::Sequential,
                },
            ],
        };
        assert!(wf.validate().is_ok());
    }
}

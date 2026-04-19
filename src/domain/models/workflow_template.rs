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
    "bash",
    "glob",
    "grep",
    "memory",
    "task_status",
    "tasks",
    "agents",
];

/// What kind of workspace to provision for tasks in this workflow.
///
/// Controls whether the orchestrator creates a git worktree, a plain temp
/// directory, or no workspace at all before launching an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// A git worktree is provisioned (default — code-producing workflows).
    #[default]
    Worktree,
    /// A temporary directory that is not a git checkout.
    TempDir,
    /// No workspace is provisioned; the agent works in a read-only view.
    None,
}

/// How output is delivered at the end of a successful workflow.
///
/// Governs what `run_post_completion_workflow` does after an agent finishes:
/// create a PR, merge directly, or skip git operations entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputDelivery {
    /// Create a pull request from the feature branch (default).
    #[default]
    PullRequest,
    /// Merge the feature branch directly without opening a PR.
    DirectMerge,
    /// Store findings in swarm memory; no git output required.
    MemoryOnly,
}

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
    /// Whether to run intent verification after this phase completes.
    ///
    /// When `true`, the workflow engine parks at `Verifying` after phase
    /// subtasks finish, runs LLM-based intent verification, and auto-reworks
    /// if verification fails (up to `max_verification_retries`).
    #[serde(default)]
    pub verify: bool,
    /// Whether this phase is a gate phase.
    ///
    /// Gate phases park at `PhaseGate` after completion and require an
    /// overmind verdict before proceeding to the next phase. Typically used
    /// for triage, validation, and review phases.
    #[serde(default)]
    pub gate: bool,
}

/// A workflow template defining the phase sequence for task execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkflowTemplate {
    /// Unique name for this workflow (e.g., "code", "docs", "analysis").
    pub name: String,
    /// Description of when to use this workflow.
    #[serde(default)]
    pub description: String,
    /// Ordered list of phases in this workflow.
    pub phases: Vec<WorkflowPhase>,
    /// What kind of workspace to provision for tasks in this workflow.
    #[serde(default)]
    pub workspace_kind: WorkspaceKind,
    /// Template-level tool grants applied to all phases (merged with phase-level tools).
    #[serde(default)]
    pub tool_grants: Vec<String>,
    /// How completed work is delivered at the end of a successful workflow.
    #[serde(default)]
    pub output_delivery: OutputDelivery,
    /// Maximum number of verification retries before escalating to a gate.
    ///
    /// Applies to phases with `verify: true`. When verification fails and
    /// retries are below this limit, the phase auto-reworks. When retries
    /// are exhausted, the engine escalates to a `PhaseGate`.
    #[serde(default = "default_max_verification_retries")]
    pub max_verification_retries: u32,
}

fn default_max_verification_retries() -> u32 {
    2
}

/// Default workflow YAML definitions embedded at compile time.
///
/// Used by `abathur init` to scaffold a starter `.abathur/workflows/` directory
/// and by tests to load a known set of templates. At runtime, workflows are
/// loaded exclusively from the `workflows_dir` configured in `abathur.toml`
/// (or the default `.abathur/workflows/`); these embedded strings are never
/// read by the engine.
pub const DEFAULT_WORKFLOW_YAMLS: &[(&str, &str)] = &[
    (
        "code",
        include_str!("../../../.abathur/workflows/code.yaml"),
    ),
    (
        "analysis",
        include_str!("../../../.abathur/workflows/analysis.yaml"),
    ),
    (
        "docs",
        include_str!("../../../.abathur/workflows/docs.yaml"),
    ),
    (
        "review",
        include_str!("../../../.abathur/workflows/review.yaml"),
    ),
    (
        "pr-review",
        include_str!("../../../.abathur/workflows/pr-review.yaml"),
    ),
    (
        "external",
        include_str!("../../../.abathur/workflows/external.yaml"),
    ),
];

impl WorkflowTemplate {
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

        // Validate template-level tool grants
        for tool in &self.tool_grants {
            if !VALID_TOOLS.contains(&tool.as_str()) {
                return Err(format!(
                    "Workflow '{}' has invalid tool_grant '{}'. Valid tools: {}",
                    self.name,
                    tool,
                    VALID_TOOLS.join(", ")
                ));
            }
        }

        Ok(())
    }

    /// Serialize this workflow template to YAML.
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(self)
            .map_err(|e| format!("Failed to serialize workflow to YAML: {}", e))
    }

    /// Parse one of the embedded default workflow YAMLs by name.
    ///
    /// Returns `Err` if no embedded workflow has the given name or if its
    /// YAML fails to parse. This is the fallible counterpart to the raw
    /// `DEFAULT_WORKFLOW_YAMLS` lookup and should be preferred by callers
    /// that would otherwise panic on parse failure.
    pub fn parse_embedded_default(name: &str) -> Result<Self, String> {
        let (_, yaml) = DEFAULT_WORKFLOW_YAMLS
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or_else(|| format!("no embedded workflow named '{}'", name))?;
        serde_yaml::from_str(yaml)
            .map_err(|e| format!("Failed to parse embedded workflow '{}': {}", name, e))
    }

    /// Parse all embedded default workflow YAMLs into a name→template map.
    ///
    /// Returns `Err` on the first parse failure. Callers that want to keep
    /// going after a failure should iterate `DEFAULT_WORKFLOW_YAMLS` directly
    /// and call `parse_embedded_default` per entry.
    pub fn parse_all_embedded_defaults() -> Result<std::collections::HashMap<String, Self>, String>
    {
        let mut templates = std::collections::HashMap::new();
        for (name, yaml) in DEFAULT_WORKFLOW_YAMLS.iter() {
            let tpl: Self = serde_yaml::from_str(yaml)
                .map_err(|e| format!("Failed to parse embedded workflow '{}': {}", name, e))?;
            templates.insert((*name).to_string(), tpl);
        }
        Ok(templates)
    }

    /// Load workflow templates from YAML files in a directory.
    ///
    /// Reads all `*.yaml` and `*.yml` files from the given directory path.
    /// Returns an empty map if the directory does not exist (graceful fallback).
    /// Returns an error only if the directory exists but a file cannot be parsed.
    pub fn load_from_directory(
        dir: impl AsRef<std::path::Path>,
    ) -> Result<std::collections::HashMap<String, Self>, String> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(std::collections::HashMap::new());
        }

        let entries = std::fs::read_dir(dir).map_err(|e| {
            format!(
                "Failed to read workflow directory '{}': {}",
                dir.display(),
                e
            )
        })?;

        let mut templates = std::collections::HashMap::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "yaml" && ext != "yml" {
                continue;
            }

            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
            let template: Self = serde_yaml::from_str(&content)
                .map_err(|e| format!("Failed to parse '{}': {}", path.display(), e))?;
            template.validate()?;
            templates.insert(template.name.clone(), template);
        }

        Ok(templates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse one of the embedded default workflow YAMLs by name.
    fn embedded_workflow(name: &str) -> WorkflowTemplate {
        WorkflowTemplate::parse_embedded_default(name)
            .expect("embedded test fixture must parse")
    }

    /// Load all embedded default workflow YAMLs into a name→template map.
    fn embedded_workflows() -> std::collections::HashMap<String, WorkflowTemplate> {
        WorkflowTemplate::parse_all_embedded_defaults()
            .expect("embedded test fixture must parse")
    }

    #[test]
    fn test_default_code_workflow() {
        let wf = embedded_workflow("code");
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
        let wf = embedded_workflow("code");
        assert_eq!(wf.phases[0].dependency, PhaseDependency::Root);
        assert_eq!(wf.phases[1].dependency, PhaseDependency::Sequential);
        assert_eq!(wf.phases[2].dependency, PhaseDependency::Sequential);
        assert_eq!(wf.phases[3].dependency, PhaseDependency::Sequential);
    }

    #[test]
    fn test_read_only_flags() {
        let wf = embedded_workflow("code");
        assert!(wf.phases[0].read_only); // research
        assert!(wf.phases[1].read_only); // plan
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
                verify: false,
                gate: false,
            }],
            ..Default::default()
        };
        assert!(wf.validate().is_err());
    }

    #[test]
    fn test_validate_no_phases() {
        let wf = WorkflowTemplate {
            name: "empty".to_string(),
            description: String::new(),
            phases: vec![],
            ..Default::default()
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
                verify: false,
                gate: false,
            }],
            ..Default::default()
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
                verify: false,
                gate: false,
            }],
            ..Default::default()
        };
        assert!(wf.validate().is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let wf = embedded_workflow("code");
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
            name: "custom-docs".to_string(),
            description: "Documentation-only workflow".to_string(),
            phases: vec![
                WorkflowPhase {
                    name: "research".to_string(),
                    description: "Research the codebase".to_string(),
                    role: "Codebase researcher".to_string(),
                    tools: vec!["read".to_string(), "glob".to_string(), "grep".to_string()],
                    read_only: true,
                    dependency: PhaseDependency::Root,
                    verify: false,
                    gate: false,
                },
                WorkflowPhase {
                    name: "write-docs".to_string(),
                    description: "Write documentation".to_string(),
                    role: "Documentation writer".to_string(),
                    tools: vec!["read".to_string(), "write".to_string(), "edit".to_string()],
                    read_only: false,
                    dependency: PhaseDependency::Sequential,
                    verify: false,
                    gate: false,
                },
            ],
            ..Default::default()
        };
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_analysis_workflow() {
        let wf = embedded_workflow("analysis");
        assert_eq!(wf.name, "analysis");
        assert_eq!(wf.phases.len(), 3);
        assert!(wf.phases.iter().all(|p| p.read_only));
        assert_eq!(wf.workspace_kind, WorkspaceKind::None);
        assert_eq!(wf.output_delivery, OutputDelivery::MemoryOnly);
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_docs_workflow() {
        let wf = embedded_workflow("docs");
        assert_eq!(wf.name, "docs");
        assert_eq!(wf.phases.len(), 3);
        assert_eq!(wf.workspace_kind, WorkspaceKind::Worktree);
        assert_eq!(wf.output_delivery, OutputDelivery::PullRequest);
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_review_only_workflow() {
        let wf = embedded_workflow("review");
        assert_eq!(wf.name, "review");
        assert_eq!(wf.phases.len(), 1);
        assert_eq!(wf.workspace_kind, WorkspaceKind::None);
        assert_eq!(wf.output_delivery, OutputDelivery::MemoryOnly);
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_new_fields_default_in_code_workflow() {
        let wf = embedded_workflow("code");
        assert_eq!(wf.workspace_kind, WorkspaceKind::Worktree);
        assert_eq!(wf.output_delivery, OutputDelivery::PullRequest);
        assert!(wf.tool_grants.is_empty());
    }

    #[test]
    fn test_validate_invalid_tool_grant() {
        let wf = WorkflowTemplate {
            name: "test".to_string(),
            phases: vec![WorkflowPhase {
                name: "do".to_string(),
                description: "do it".to_string(),
                role: "doer".to_string(),
                tools: vec!["read".to_string()],
                read_only: false,
                verify: false,
                dependency: PhaseDependency::Root,
                gate: false,
            }],
            tool_grants: vec!["not_a_real_tool".to_string()],
            ..Default::default()
        };
        let err = wf.validate().unwrap_err();
        assert!(err.contains("not_a_real_tool"));
    }

    #[test]
    fn test_pr_review_workflow() {
        let wf = embedded_workflow("pr-review");
        assert_eq!(wf.name, "pr-review");
        assert_eq!(wf.phases.len(), 1);
        assert_eq!(wf.workspace_kind, WorkspaceKind::None);
        assert_eq!(wf.output_delivery, OutputDelivery::MemoryOnly);
        assert!(wf.phases[0].read_only);
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn test_pr_review_workflow_no_shell_tool() {
        let wf = embedded_workflow("pr-review");
        // Security invariant: PR review workflow must NEVER include shell tool.
        for phase in &wf.phases {
            assert!(
                !phase.tools.contains(&"shell".to_string()),
                "PR review workflow phase '{}' must not have shell tool",
                phase.name
            );
        }
        assert!(
            !wf.tool_grants.contains(&"shell".to_string()),
            "PR review workflow must not have shell in tool_grants"
        );
    }

    #[test]
    fn test_default_workflows_include_pr_review() {
        let templates = embedded_workflows();
        assert!(templates.contains_key("pr-review"));
    }

    #[test]
    fn test_workspace_kind_defaults() {
        let kind: WorkspaceKind = Default::default();
        assert_eq!(kind, WorkspaceKind::Worktree);
        let delivery: OutputDelivery = Default::default();
        assert_eq!(delivery, OutputDelivery::PullRequest);
    }

    #[test]
    fn test_all_default_workflows_validate() {
        let templates = embedded_workflows();
        assert!(
            !templates.is_empty(),
            "DEFAULT_WORKFLOW_YAMLS must contain at least one template"
        );
        for (name, template) in &templates {
            template.validate().unwrap_or_else(|e| {
                panic!("Default workflow '{}' failed validation: {}", name, e);
            });
        }
    }

    #[test]
    fn test_yaml_roundtrip_all_defaults() {
        let templates = embedded_workflows();
        for (name, original) in &templates {
            let yaml = original.to_yaml().unwrap_or_else(|e| {
                panic!("Failed to serialize '{}' to YAML: {}", name, e);
            });
            let deserialized: WorkflowTemplate = serde_yaml::from_str(&yaml).unwrap_or_else(|e| {
                panic!("Failed to deserialize '{}' from YAML: {}", name, e);
            });
            assert_eq!(
                original, &deserialized,
                "YAML round-trip failed for workflow '{}'",
                name
            );
        }
    }

    #[test]
    fn test_load_from_directory_missing_dir() {
        let result = WorkflowTemplate::load_from_directory("/nonexistent/path/to/workflows");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_load_from_directory_with_files() {
        let dir = std::env::temp_dir().join("abathur_test_workflows");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let wf = embedded_workflow("code");
        let yaml = wf.to_yaml().unwrap();
        std::fs::write(dir.join("code.yaml"), &yaml).unwrap();

        let loaded = WorkflowTemplate::load_from_directory(&dir).unwrap();
        assert!(loaded.contains_key("code"));
        assert_eq!(loaded["code"], wf);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gate_field_defaults_false() {
        // Ensure gate defaults to false when missing from YAML/JSON
        let yaml = r#"
name: test
description: test
role: tester
tools:
  - read
"#;
        let phase: WorkflowPhase = serde_yaml::from_str(yaml).unwrap();
        assert!(!phase.gate);
    }

    #[test]
    fn test_parse_embedded_default_unknown_name_returns_err() {
        let result = WorkflowTemplate::parse_embedded_default("does-not-exist");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("does-not-exist"),
            "error should mention the unknown name, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_embedded_default_missing_name_is_err() {
        // Empty-string name must also be rejected without panicking.
        let result = WorkflowTemplate::parse_embedded_default("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_malformed_yaml_returns_err_without_panic() {
        // Exercise the same code path on a malformed YAML string. We can't
        // inject into DEFAULT_WORKFLOW_YAMLS, so we drive the underlying
        // serde_yaml::from_str the way parse_embedded_default does.
        let malformed = "name: [unterminated\nphases: : :";
        let result: Result<WorkflowTemplate, _> = serde_yaml::from_str(malformed);
        assert!(
            result.is_err(),
            "malformed YAML must return Err, not panic"
        );
    }

    #[test]
    fn test_parse_all_embedded_defaults_ok() {
        let templates = WorkflowTemplate::parse_all_embedded_defaults()
            .expect("all embedded defaults must parse");
        assert!(!templates.is_empty());
        // All entries from DEFAULT_WORKFLOW_YAMLS should be represented.
        for (name, _) in DEFAULT_WORKFLOW_YAMLS.iter() {
            assert!(
                templates.contains_key(*name),
                "parse_all_embedded_defaults missing '{}'",
                name
            );
        }
    }

    #[test]
    fn test_default_workflow_gate_phases() {
        let code = embedded_workflow("code");
        assert!(!code.phases[0].gate); // research
        assert!(!code.phases[1].gate); // plan
        assert!(!code.phases[2].gate); // implement
        assert!(code.phases[3].gate); // review

        let ext = embedded_workflow("external");
        assert!(ext.phases[0].gate); // triage
        assert!(ext.phases[1].gate); // validation
        assert!(!ext.phases[2].gate); // research
        assert!(!ext.phases[3].gate); // plan
        assert!(!ext.phases[4].gate); // implement
        assert!(ext.phases[5].gate); // review
    }
}

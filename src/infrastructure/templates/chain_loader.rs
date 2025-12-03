//! Chain template loader from YAML files

use crate::domain::models::hook::HookAction;
use crate::domain::models::prompt_chain::{
    BranchConfig, DecompositionConfig, OnDecompositionComplete, OutputFormat, PerItemConfig,
    PromptChain, PromptStep, TaskSpawnConfig, ValidationRule, ValidationType,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Template structure for loading chains from YAML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainTemplate {
    pub name: String,
    pub description: String,
    pub steps: Vec<StepTemplate>,
    #[serde(default)]
    pub validation_rules: Vec<ValidationRuleTemplate>,
}

/// Template for a chain step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTemplate {
    pub id: String,
    pub role: String,
    pub prompt: String,
    pub expected_output: OutputFormatTemplate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_hooks: Vec<HookAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_hooks: Vec<HookAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_branch: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name_template: Option<String>,
    // DEPRECATED: For backwards compatibility with old configs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_task_branch: Option<bool>,
    /// Decomposition configuration for fan-out pattern
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decomposition: Option<DecompositionConfigTemplate>,
}

/// Template for decomposition configuration (fan-out pattern)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionConfigTemplate {
    /// JSON path to items array in step output
    pub items_path: String,
    /// Configuration for each item
    pub per_item: PerItemConfigTemplate,
    /// Behavior after spawning
    #[serde(default)]
    pub on_complete: OnDecompositionCompleteTemplate,
}

/// Template for per-item configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerItemConfigTemplate {
    /// Branch configuration
    pub branch: BranchConfigTemplate,
    /// Task to spawn
    pub task: TaskSpawnConfigTemplate,
}

/// Template for branch configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchConfigTemplate {
    /// Branch name template
    pub template: String,
    /// Parent branch
    #[serde(default = "default_branch_parent_template")]
    pub parent: String,
}

fn default_branch_parent_template() -> String {
    "main".to_string()
}

/// Template for task spawn configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpawnConfigTemplate {
    /// Agent type
    pub agent_type: String,
    /// Summary template
    pub summary: String,
    /// Description template
    pub description: String,
    /// Priority
    #[serde(default = "default_priority_template")]
    pub priority: u8,
    /// Continue chain in spawned task
    #[serde(default)]
    pub continue_chain: bool,
    /// Step to continue at
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continue_at_step: Option<String>,
}

fn default_priority_template() -> u8 {
    5
}

/// Template for on_complete configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OnDecompositionCompleteTemplate {
    /// Wait for children to complete
    #[serde(default = "default_wait_for_children_template")]
    pub wait_for_children: bool,
}

fn default_wait_for_children_template() -> bool {
    true
}

/// Template for output format specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OutputFormatTemplate {
    Json {
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
    },
    Markdown,
    Plain,
}

/// Template for validation rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRuleTemplate {
    pub step_id: String,
    pub rule_type: ValidationTypeTemplate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    pub error_message: String,
}

/// Template for validation type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValidationTypeTemplate {
    JsonSchema,
    RegexMatch { pattern: String },
    CustomValidator { name: String },
}

/// Loader for chain templates
pub struct ChainLoader {
    template_dir: std::path::PathBuf,
}

impl ChainLoader {
    /// Create a new chain loader
    pub fn new<P: AsRef<Path>>(template_dir: P) -> Self {
        Self {
            template_dir: template_dir.as_ref().to_path_buf(),
        }
    }

    /// Load a chain from a YAML file
    pub fn load_from_file<P: AsRef<Path>>(&self, filename: P) -> Result<PromptChain> {
        let path = self.template_dir.join(filename);
        debug!("Loading chain template from: {}", path.display());

        let content = std::fs::read_to_string(&path)
            .context(format!("Failed to read template file: {}", path.display()))?;

        self.load_from_yaml(&content)
    }

    /// Load a chain from YAML string
    pub fn load_from_yaml(&self, yaml: &str) -> Result<PromptChain> {
        let template: ChainTemplate = serde_yaml::from_str(yaml)
            .map_err(|e| {
                error!("YAML parsing error: {}", e);
                if let Some(location) = e.location() {
                    error!("Error at line {}, column {}", location.line(), location.column());
                }
                e
            })
            .context("Failed to parse YAML template")?;

        info!("Loading chain template: {}", template.name);

        self.template_to_chain(template)
    }

    /// Convert a template to a PromptChain
    fn template_to_chain(&self, template: ChainTemplate) -> Result<PromptChain> {
        let mut chain = PromptChain::new(template.name, template.description);

        // Convert steps
        for step_template in template.steps {
            let step = self.template_to_step(step_template)?;
            chain.add_step(step);
        }

        // Convert validation rules
        for rule_template in template.validation_rules {
            let rule = self.template_to_validation_rule(rule_template)?;
            chain.add_validation_rule(rule);
        }

        // Validate the chain structure
        chain.validate().context("Chain validation failed")?;

        Ok(chain)
    }

    /// Convert a step template to a PromptStep
    fn template_to_step(&self, template: StepTemplate) -> Result<PromptStep> {
        let output_format = match template.expected_output {
            OutputFormatTemplate::Json { schema } => OutputFormat::Json { schema },
            OutputFormatTemplate::Markdown => OutputFormat::Markdown,
            OutputFormatTemplate::Plain => OutputFormat::Plain,
        };

        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        debug!(
            step_id = %template.id,
            timeout_secs = ?template.timeout_secs,
            "ChainLoader: Loading step template from YAML"
        );

        let mut step = PromptStep::new(
            template.id.clone(),
            template.prompt,
            template.role,
            output_format,
        );

        if let Some(next_id) = template.next {
            step = step.with_next_step(next_id);
        }

        if let Some(timeout_secs) = template.timeout_secs {
            // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
            info!(
                step_id = %template.id,
                timeout_secs = timeout_secs,
                "ChainLoader: Setting step timeout from YAML"
            );
            step = step.with_timeout(Duration::from_secs(timeout_secs));

            // TODO: TEMPORARY DEBUG - Verify timeout was actually set
            info!(
                step_id = %template.id,
                step_timeout_after_set = ?step.timeout,
                "ChainLoader: Step timeout after with_timeout() call"
            );
        } else {
            // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
            warn!(
                step_id = %template.id,
                "ChainLoader: No timeout_secs in YAML, step will use default"
            );
        }

        if !template.pre_hooks.is_empty() {
            step = step.with_pre_hooks(template.pre_hooks);
        }

        if !template.post_hooks.is_empty() {
            step = step.with_post_hooks(template.post_hooks);
        }

        if let Some(working_dir) = template.working_directory {
            step = step.with_working_directory(working_dir);
        }

        // Handle new branch system
        if let Some(needs_branch) = template.needs_branch {
            step = step.with_needs_branch(needs_branch);
        } else if let Some(needs_task_branch) = template.needs_task_branch {
            // DEPRECATED: Auto-migrate old needs_task_branch to new system
            tracing::warn!(
                step_id = %template.id,
                "Step uses deprecated 'needs_task_branch' field. Please migrate to 'needs_branch' with 'branch_parent' and 'branch_name_template'"
            );
            step = step.with_needs_branch(needs_task_branch);
            // Auto-set defaults for backwards compatibility
            if needs_task_branch {
                if template.branch_parent.is_none() {
                    step = step.with_branch_parent("feature_branch".to_string());
                }
                if template.branch_name_template.is_none() {
                    step = step.with_branch_name_template("task/{feature_name}/{step_id}".to_string());
                }
            }
        }

        if let Some(branch_parent) = template.branch_parent {
            step = step.with_branch_parent(branch_parent);
        }

        if let Some(branch_name_template) = template.branch_name_template {
            step = step.with_branch_name_template(branch_name_template);
        }

        // Handle decomposition configuration (fan-out pattern)
        if let Some(decomp_template) = template.decomposition {
            let decomposition = DecompositionConfig {
                items_path: decomp_template.items_path,
                per_item: PerItemConfig {
                    branch: BranchConfig {
                        template: decomp_template.per_item.branch.template,
                        parent: decomp_template.per_item.branch.parent,
                    },
                    task: TaskSpawnConfig {
                        agent_type: decomp_template.per_item.task.agent_type,
                        summary: decomp_template.per_item.task.summary,
                        description: decomp_template.per_item.task.description,
                        priority: decomp_template.per_item.task.priority,
                        continue_chain: decomp_template.per_item.task.continue_chain,
                        continue_at_step: decomp_template.per_item.task.continue_at_step,
                    },
                },
                on_complete: OnDecompositionComplete {
                    wait_for_children: decomp_template.on_complete.wait_for_children,
                },
            };
            step = step.with_decomposition(decomposition);
            info!(
                step_id = %template.id,
                items_path = %step.decomposition.as_ref().unwrap().items_path,
                "ChainLoader: Step configured with decomposition (fan-out pattern)"
            );
        }

        Ok(step)
    }

    /// Convert a validation rule template to a ValidationRule
    fn template_to_validation_rule(
        &self,
        template: ValidationRuleTemplate,
    ) -> Result<ValidationRule> {
        let rule_type = match template.rule_type {
            ValidationTypeTemplate::JsonSchema => ValidationType::JsonSchema,
            ValidationTypeTemplate::RegexMatch { pattern } => {
                ValidationType::RegexMatch { pattern }
            }
            ValidationTypeTemplate::CustomValidator { name } => {
                ValidationType::CustomValidator { name }
            }
        };

        Ok(ValidationRule {
            step_id: template.step_id,
            rule_type,
            schema: template.schema,
            error_message: template.error_message,
        })
    }

    /// List all available chain templates in the template directory
    pub fn list_templates(&self) -> Result<Vec<String>> {
        let mut templates = Vec::new();

        if !self.template_dir.exists() {
            return Ok(templates);
        }

        for entry in std::fs::read_dir(&self.template_dir)
            .context("Failed to read template directory")?
        {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("yaml")
                || path.extension().and_then(|s| s.to_str()) == Some("yml")
            {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    templates.push(name.to_string());
                }
            }
        }

        templates.sort();
        Ok(templates)
    }

    /// Save a chain as a YAML template
    pub fn save_template<P: AsRef<Path>>(
        &self,
        chain: &PromptChain,
        filename: P,
    ) -> Result<()> {
        let template = self.chain_to_template(chain)?;
        let yaml = serde_yaml::to_string(&template)
            .context("Failed to serialize chain to YAML")?;

        let path = self.template_dir.join(filename);
        std::fs::write(&path, yaml)
            .context(format!("Failed to write template to: {}", path.display()))?;

        info!("Saved chain template to: {}", path.display());
        Ok(())
    }

    /// Convert a PromptChain to a template
    fn chain_to_template(&self, chain: &PromptChain) -> Result<ChainTemplate> {
        let steps: Vec<StepTemplate> = chain
            .steps
            .iter()
            .map(|step| self.step_to_template(step))
            .collect::<Result<Vec<_>>>()?;

        let validation_rules: Vec<ValidationRuleTemplate> = chain
            .validation_rules
            .iter()
            .map(|rule| self.validation_rule_to_template(rule))
            .collect::<Result<Vec<_>>>()?;

        Ok(ChainTemplate {
            name: chain.name.clone(),
            description: chain.description.clone(),
            steps,
            validation_rules,
        })
    }

    /// Convert a PromptStep to a template
    fn step_to_template(&self, step: &PromptStep) -> Result<StepTemplate> {
        let expected_output = match &step.expected_output {
            OutputFormat::Json { schema } => OutputFormatTemplate::Json {
                schema: schema.clone(),
            },
            OutputFormat::Markdown => OutputFormatTemplate::Markdown,
            OutputFormat::Plain => OutputFormatTemplate::Plain,
        };

        Ok(StepTemplate {
            id: step.id.clone(),
            role: step.role.clone(),
            prompt: step.prompt_template.clone(),
            expected_output,
            next: step.next_step.clone(),
            timeout_secs: step.timeout.map(|d| d.as_secs()),
            pre_hooks: step.pre_hooks.clone(),
            post_hooks: step.post_hooks.clone(),
            working_directory: step.working_directory.clone(),
            needs_branch: step.needs_branch,
            branch_parent: step.branch_parent.clone(),
            branch_name_template: step.branch_name_template.clone(),
            needs_task_branch: None, // Never serialize deprecated field
            decomposition: step.decomposition.as_ref().map(|d| DecompositionConfigTemplate {
                items_path: d.items_path.clone(),
                per_item: PerItemConfigTemplate {
                    branch: BranchConfigTemplate {
                        template: d.per_item.branch.template.clone(),
                        parent: d.per_item.branch.parent.clone(),
                    },
                    task: TaskSpawnConfigTemplate {
                        agent_type: d.per_item.task.agent_type.clone(),
                        summary: d.per_item.task.summary.clone(),
                        description: d.per_item.task.description.clone(),
                        priority: d.per_item.task.priority,
                        continue_chain: d.per_item.task.continue_chain,
                        continue_at_step: d.per_item.task.continue_at_step.clone(),
                    },
                },
                on_complete: OnDecompositionCompleteTemplate {
                    wait_for_children: d.on_complete.wait_for_children,
                },
            }),
        })
    }

    /// Convert a ValidationRule to a template
    fn validation_rule_to_template(
        &self,
        rule: &ValidationRule,
    ) -> Result<ValidationRuleTemplate> {
        let rule_type = match &rule.rule_type {
            ValidationType::JsonSchema => ValidationTypeTemplate::JsonSchema,
            ValidationType::RegexMatch { pattern } => {
                ValidationTypeTemplate::RegexMatch {
                    pattern: pattern.clone(),
                }
            }
            ValidationType::CustomValidator { name } => {
                ValidationTypeTemplate::CustomValidator { name: name.clone() }
            }
        };

        Ok(ValidationRuleTemplate {
            step_id: rule.step_id.clone(),
            rule_type,
            schema: rule.schema.clone(),
            error_message: rule.error_message.clone(),
        })
    }
}

impl Default for ChainLoader {
    fn default() -> Self {
        Self::new(".abathur/chains")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_from_yaml_simple() {
        let yaml = r#"
name: test_chain
description: A test chain
steps:
  - id: step1
    role: Tester
    prompt: "Test prompt"
    expected_output:
      type: json
"#;

        let loader = ChainLoader::default();
        let result = loader.load_from_yaml(yaml);
        assert!(result.is_ok());

        let chain = result.unwrap();
        assert_eq!(chain.name, "test_chain");
        assert_eq!(chain.steps.len(), 1);
    }

    #[test]
    fn test_load_from_yaml_with_schema() {
        let yaml = r#"
name: validation_chain
description: Chain with JSON schema validation
steps:
  - id: step1
    role: Validator
    prompt: "Validate this: {data}"
    expected_output:
      type: json
      schema:
        type: object
        properties:
          result:
            type: string
        required:
          - result
"#;

        let loader = ChainLoader::default();
        let result = loader.load_from_yaml(yaml);
        assert!(result.is_ok());

        let chain = result.unwrap();
        if let OutputFormat::Json { schema } = &chain.steps[0].expected_output {
            assert!(schema.is_some());
        } else {
            panic!("Expected JSON output format");
        }
    }

    #[test]
    fn test_load_from_yaml_multi_step() {
        let yaml = r#"
name: multi_step
description: Multi-step chain
steps:
  - id: step1
    role: Extractor
    prompt: "Extract from {source}"
    expected_output:
      type: json
    next: step2

  - id: step2
    role: Transformer
    prompt: "Transform {previous_output}"
    expected_output:
      type: json
"#;

        let loader = ChainLoader::default();
        let result = loader.load_from_yaml(yaml);
        assert!(result.is_ok());

        let chain = result.unwrap();
        assert_eq!(chain.steps.len(), 2);
        assert_eq!(chain.steps[0].next_step, Some("step2".to_string()));
    }

    #[test]
    fn test_roundtrip_conversion() {
        let mut chain = PromptChain::new(
            "roundtrip".to_string(),
            "Test roundtrip".to_string(),
        );

        let step = PromptStep::new(
            "step1".to_string(),
            "Test {input}".to_string(),
            "Tester".to_string(),
            OutputFormat::Json { schema: None },
        );
        chain.add_step(step);

        let loader = ChainLoader::default();

        // Convert to template
        let template = loader.chain_to_template(&chain).unwrap();

        // Convert back to chain
        let restored_chain = loader.template_to_chain(template).unwrap();

        assert_eq!(chain.name, restored_chain.name);
        assert_eq!(chain.steps.len(), restored_chain.steps.len());
        assert_eq!(chain.steps[0].id, restored_chain.steps[0].id);
    }
}

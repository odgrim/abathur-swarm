//! Chain template loader from YAML files

use crate::domain::models::hook::HookAction;
use crate::domain::models::prompt_chain::{
    OutputFormat, PromptChain, PromptStep, ValidationRule, ValidationType,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info};

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
}

/// Template for output format specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OutputFormatTemplate {
    Json {
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
    },
    Xml {
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
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
    XmlSchema,
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
            OutputFormatTemplate::Xml { schema } => OutputFormat::Xml { schema },
            OutputFormatTemplate::Markdown => OutputFormat::Markdown,
            OutputFormatTemplate::Plain => OutputFormat::Plain,
        };

        let mut step = PromptStep::new(
            template.id,
            template.prompt,
            template.role,
            output_format,
        );

        if let Some(next_id) = template.next {
            step = step.with_next_step(next_id);
        }

        if let Some(timeout_secs) = template.timeout_secs {
            step = step.with_timeout(Duration::from_secs(timeout_secs));
        }

        if !template.pre_hooks.is_empty() {
            step = step.with_pre_hooks(template.pre_hooks);
        }

        if !template.post_hooks.is_empty() {
            step = step.with_post_hooks(template.post_hooks);
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
            ValidationTypeTemplate::XmlSchema => ValidationType::XmlSchema,
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
            OutputFormat::Xml { schema } => OutputFormatTemplate::Xml {
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
        })
    }

    /// Convert a ValidationRule to a template
    fn validation_rule_to_template(
        &self,
        rule: &ValidationRule,
    ) -> Result<ValidationRuleTemplate> {
        let rule_type = match &rule.rule_type {
            ValidationType::JsonSchema => ValidationTypeTemplate::JsonSchema,
            ValidationType::XmlSchema => ValidationTypeTemplate::XmlSchema,
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
        Self::new("template/chains")
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

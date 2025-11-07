//! Output validation for prompt chain results

use crate::domain::models::prompt_chain::{OutputFormat, ValidationType};
use anyhow::{Context, Result};
use jsonschema::{Draft, JSONSchema};
use regex::Regex;
use std::sync::Arc;

/// Validates outputs from prompt chain steps
#[derive(Clone)]
pub struct OutputValidator {
    custom_validators: Arc<std::collections::HashMap<String, Box<dyn CustomValidator>>>,
}

impl OutputValidator {
    /// Create a new output validator
    pub fn new() -> Self {
        Self {
            custom_validators: Arc::new(std::collections::HashMap::new()),
        }
    }

    /// Register a custom validator
    pub fn register_custom_validator(
        &mut self,
        name: String,
        validator: Box<dyn CustomValidator>,
    ) {
        Arc::get_mut(&mut self.custom_validators)
            .expect("Cannot modify validators after cloning")
            .insert(name, validator);
    }

    /// Strip markdown code blocks from output
    ///
    /// Many LLMs wrap JSON in markdown code blocks even when instructed not to.
    /// This helper strips those blocks to get to the actual content.
    ///
    /// Handles formats like:
    /// - ```json\n{...}\n```
    /// - ```\n{...}\n```
    fn strip_markdown_code_blocks(output: &str) -> String {
        let trimmed = output.trim();

        // Check if wrapped in markdown code block
        if trimmed.starts_with("```") && trimmed.ends_with("```") {
            tracing::warn!(
                input_length = trimmed.len(),
                "Stripping markdown code blocks from output"
            );

            // Find the first newline after ```
            let start = if let Some(pos) = trimmed.find('\n') {
                pos + 1
            } else {
                3 // Just skip the ```
            };

            // Find the last ``` and go back to the newline before it
            let end = trimmed.rfind("\n```").unwrap_or(trimmed.len() - 3);

            let result = trimmed[start..end].trim().to_string();
            tracing::warn!(
                result_length = result.len(),
                result_preview = &result[..result.len().min(200)],
                "Stripped markdown: first 200 chars of result"
            );
            return result;
        }

        tracing::warn!(
            input_length = trimmed.len(),
            starts_with_backticks = trimmed.starts_with("```"),
            ends_with_backticks = trimmed.ends_with("```"),
            input_preview = &trimmed[..trimmed.len().min(200)],
            "Output NOT wrapped in markdown, returning as-is"
        );
        output.to_string()
    }

    /// Validate output against the expected format
    pub fn validate(&self, output: &str, format: &OutputFormat) -> Result<bool> {
        match format {
            OutputFormat::Json { schema } => {
                // Strip markdown code blocks before validating JSON
                let cleaned = Self::strip_markdown_code_blocks(output);

                if let Some(schema_value) = schema {
                    self.validate_json(&cleaned, schema_value)
                } else {
                    // Just verify it's valid JSON
                    serde_json::from_str::<serde_json::Value>(&cleaned)
                        .context("Invalid JSON output")?;
                    Ok(true)
                }
            }
            OutputFormat::Markdown | OutputFormat::Plain => {
                // No validation needed for plain formats
                Ok(true)
            }
        }
    }

    /// Validate JSON output against a schema
    pub fn validate_json(&self, output: &str, schema: &serde_json::Value) -> Result<bool> {
        // Log what we're validating for debugging
        tracing::error!(
            output_length = output.len(),
            output_preview = &output[..output.len().min(500)],
            "Validating JSON output against schema"
        );

        // Parse the output as JSON
        let instance: serde_json::Value = serde_json::from_str(output)
            .context("Failed to parse output as JSON")?;

        // Compile the schema - we need to own the schema to satisfy lifetime requirements
        let compiled_schema = JSONSchema::options()
            .with_draft(Draft::Draft7)
            .compile(schema)
            .map_err(|e| anyhow::anyhow!("Failed to compile JSON schema: {}", e))?;

        // Validate
        match compiled_schema.validate(&instance) {
            Ok(_) => Ok(true),
            Err(errors) => {
                let error_messages: Vec<String> = errors
                    .map(|e| format!("{}: {}", e.instance_path, e))
                    .collect();
                anyhow::bail!(
                    "JSON validation failed: {}",
                    error_messages.join(", ")
                );
            }
        }
    }

    /// Validate using a validation rule
    pub fn validate_with_rule(
        &self,
        output: &str,
        rule_type: &ValidationType,
        schema: Option<&serde_json::Value>,
    ) -> Result<bool> {
        match rule_type {
            ValidationType::JsonSchema => {
                if let Some(schema_value) = schema {
                    self.validate_json(output, schema_value)
                } else {
                    anyhow::bail!("JSON schema validation requires a schema");
                }
            }
            ValidationType::RegexMatch { pattern } => {
                self.validate_regex(output, pattern)
            }
            ValidationType::CustomValidator { name } => {
                self.validate_custom(output, name)
            }
        }
    }

    /// Validate using a regex pattern
    pub fn validate_regex(&self, output: &str, pattern: &str) -> Result<bool> {
        let regex = Regex::new(pattern)
            .context(format!("Invalid regex pattern: {}", pattern))?;

        if regex.is_match(output) {
            Ok(true)
        } else {
            anyhow::bail!("Output does not match regex pattern: {}", pattern);
        }
    }

    /// Validate using a custom validator
    pub fn validate_custom(&self, output: &str, validator_name: &str) -> Result<bool> {
        let validator = self
            .custom_validators
            .get(validator_name)
            .context(format!("Custom validator not found: {}", validator_name))?;

        validator.validate(output)
    }

    /// Extract a field from JSON output using JSON path
    pub fn extract_json_field(&self, output: &str, path: &str) -> Result<serde_json::Value> {
        let value: serde_json::Value = serde_json::from_str(output)
            .context("Failed to parse output as JSON")?;

        // Simple JSON path implementation (supports dot notation)
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = &value;

        for part in parts {
            current = current
                .get(part)
                .context(format!("Field not found: {}", part))?;
        }

        Ok(current.clone())
    }

    /// Extract multiple fields from JSON output
    pub fn extract_json_fields(
        &self,
        output: &str,
        paths: &[&str],
    ) -> Result<Vec<serde_json::Value>> {
        paths
            .iter()
            .map(|path| self.extract_json_field(output, path))
            .collect()
    }
}

impl Default for OutputValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for custom validators
pub trait CustomValidator: Send + Sync {
    /// Validate the output
    fn validate(&self, output: &str) -> Result<bool>;

    /// Get the validator name
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_json_valid() {
        let validator = OutputValidator::new();
        let output = r#"{"name": "Alice", "age": 30}"#;
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            },
            "required": ["name", "age"]
        });

        assert!(validator.validate_json(output, &schema).is_ok());
    }

    #[test]
    fn test_validate_json_invalid() {
        let validator = OutputValidator::new();
        let output = r#"{"name": "Alice"}"#; // Missing required 'age'
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            },
            "required": ["name", "age"]
        });

        assert!(validator.validate_json(output, &schema).is_err());
    }

    #[test]
    fn test_validate_regex() {
        let validator = OutputValidator::new();
        let output = "Hello, World!";
        let pattern = r"^Hello,\s\w+!$";

        assert!(validator.validate_regex(output, pattern).is_ok());

        let invalid_pattern = r"^\d+$";
        assert!(validator.validate_regex(output, invalid_pattern).is_err());
    }

    #[test]
    fn test_extract_json_field() {
        let validator = OutputValidator::new();
        let output = r#"{"user": {"name": "Alice", "profile": {"age": 30}}}"#;

        let result = validator.extract_json_field(output, "user.name").unwrap();
        assert_eq!(result, json!("Alice"));

        let result = validator.extract_json_field(output, "user.profile.age").unwrap();
        assert_eq!(result, json!(30));
    }

    #[test]
    fn test_extract_json_fields() {
        let validator = OutputValidator::new();
        let output = r#"{"name": "Alice", "age": 30, "city": "NYC"}"#;

        let results = validator
            .extract_json_fields(output, &["name", "age"])
            .unwrap();
        assert_eq!(results, vec![json!("Alice"), json!(30)]);
    }

    #[test]
    fn test_validate_output_format_json() {
        let validator = OutputValidator::new();
        let output = r#"{"test": "value"}"#;
        let format = OutputFormat::Json { schema: None };

        assert!(validator.validate(output, &format).is_ok());
    }

    #[test]
    fn test_validate_output_format_plain() {
        let validator = OutputValidator::new();
        let output = "Any text content";
        let format = OutputFormat::Plain;

        assert!(validator.validate(output, &format).is_ok());
    }
}

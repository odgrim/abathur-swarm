//! Prompt Chain Domain Models
//!
//! Models for breaking complex tasks into sequential sub-prompts with
//! structured output validation between steps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::hook::HookAction;

/// Output format specification for chain steps
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OutputFormat {
    /// JSON output with optional schema validation
    Json {
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
    },
    /// Markdown formatted output
    Markdown,
    /// Plain text output
    Plain,
}

/// A complete prompt chain definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptChain {
    /// Unique identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of the chain's purpose
    pub description: String,
    /// Ordered sequence of steps
    pub steps: Vec<PromptStep>,
    /// Validation rules applied across steps
    pub validation_rules: Vec<ValidationRule>,
    /// Creation timestamp
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

impl PromptChain {
    /// Create a new prompt chain
    pub fn new(name: String, description: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            description,
            steps: Vec::new(),
            validation_rules: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Add a step to the chain
    pub fn add_step(&mut self, step: PromptStep) {
        self.steps.push(step);
        self.updated_at = Utc::now();
    }

    /// Add a validation rule
    pub fn add_validation_rule(&mut self, rule: ValidationRule) {
        self.validation_rules.push(rule);
        self.updated_at = Utc::now();
    }

    /// Get a step by ID
    pub fn get_step(&self, step_id: &str) -> Option<&PromptStep> {
        self.steps.iter().find(|s| s.id == step_id)
    }

    /// Validate the chain structure
    pub fn validate(&self) -> Result<(), ChainValidationError> {
        if self.steps.is_empty() {
            return Err(ChainValidationError::EmptyChain);
        }

        // Verify all next_step references are valid
        for step in &self.steps {
            if let Some(next_id) = &step.next_step {
                if !self.steps.iter().any(|s| &s.id == next_id) {
                    return Err(ChainValidationError::InvalidNextStep {
                        step_id: step.id.clone(),
                        next_id: next_id.clone(),
                    });
                }
            }
        }

        // Check for cycles
        if self.has_cycle() {
            return Err(ChainValidationError::CycleDetected);
        }

        Ok(())
    }

    /// Check if the chain has a cycle
    fn has_cycle(&self) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut current = self.steps.first().map(|s| &s.id);

        while let Some(step_id) = current {
            if !visited.insert(step_id) {
                return true; // Cycle detected
            }

            current = self
                .get_step(step_id)
                .and_then(|s| s.next_step.as_ref());
        }

        false
    }
}

/// A single step in a prompt chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptStep {
    /// Unique step identifier
    pub id: String,
    /// Prompt template with variable placeholders
    pub prompt_template: String,
    /// Role context for this step (e.g., "Market Analyst", "Code Reviewer")
    pub role: String,
    /// Expected output format
    pub expected_output: OutputFormat,
    /// ID of the next step, if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    /// Maximum execution time for this step
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration",
        rename = "timeout_secs"
    )]
    pub timeout: Option<Duration>,
    /// Hook actions to execute before this step
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_hooks: Vec<HookAction>,
    /// Hook actions to execute after this step
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_hooks: Vec<HookAction>,
    /// Optional working directory for agent execution
    /// If specified, the agent will have its PWD set to this directory
    /// Supports variable substitution like {task_id}
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

impl PromptStep {
    /// Create a new prompt step
    pub fn new(
        id: String,
        prompt_template: String,
        role: String,
        expected_output: OutputFormat,
    ) -> Self {
        Self {
            id,
            prompt_template,
            role,
            expected_output,
            next_step: None,
            timeout: None,
            pre_hooks: Vec::new(),
            post_hooks: Vec::new(),
            working_directory: None,
        }
    }

    /// Set the next step
    pub fn with_next_step(mut self, next_id: String) -> Self {
        self.next_step = Some(next_id);
        self
    }

    /// Set the timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Add a pre-hook action
    pub fn with_pre_hook(mut self, hook: HookAction) -> Self {
        self.pre_hooks.push(hook);
        self
    }

    /// Add a post-hook action
    pub fn with_post_hook(mut self, hook: HookAction) -> Self {
        self.post_hooks.push(hook);
        self
    }

    /// Add multiple pre-hooks
    pub fn with_pre_hooks(mut self, hooks: Vec<HookAction>) -> Self {
        self.pre_hooks.extend(hooks);
        self
    }

    /// Add multiple post-hooks
    pub fn with_post_hooks(mut self, hooks: Vec<HookAction>) -> Self {
        self.post_hooks.extend(hooks);
        self
    }

    /// Build the actual prompt by replacing variables
    pub fn build_prompt(&self, variables: &serde_json::Value) -> anyhow::Result<String> {
        let mut prompt = self.prompt_template.clone();

        if let Some(vars) = variables.as_object() {
            for (key, value) in vars {
                let placeholder = format!("{{{}}}", key);
                let replacement = match value {
                    serde_json::Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                prompt = prompt.replace(&placeholder, &replacement);
            }
        }

        Ok(prompt)
    }
}

/// Validation rule for chain outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    /// Step this rule applies to
    pub step_id: String,
    /// Type of validation to perform
    pub rule_type: ValidationType,
    /// Optional schema for validation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// Error message if validation fails
    pub error_message: String,
}

/// Type of validation to perform
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValidationType {
    /// Validate against JSON schema
    JsonSchema,
    /// Validate using regex pattern
    RegexMatch { pattern: String },
    /// Custom validator function
    CustomValidator { name: String },
}

/// Execution state of a chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainExecution {
    /// Unique execution ID
    pub id: String,
    /// ID of the chain being executed
    pub chain_id: String,
    /// Associated task ID
    pub task_id: String,
    /// Current step index
    pub current_step: usize,
    /// Results from completed steps
    pub step_results: Vec<StepResult>,
    /// Current execution status
    pub status: ChainStatus,
    /// When execution started
    pub started_at: DateTime<Utc>,
    /// When execution completed (if finished)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl ChainExecution {
    /// Create a new chain execution
    pub fn new(chain_id: String, task_id: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            chain_id,
            task_id,
            current_step: 0,
            step_results: Vec::new(),
            status: ChainStatus::Running,
            started_at: Utc::now(),
            completed_at: None,
        }
    }

    /// Add a step result
    pub fn add_result(&mut self, result: StepResult) {
        self.step_results.push(result);
        self.current_step += 1;
    }

    /// Mark execution as completed
    pub fn complete(&mut self) {
        self.status = ChainStatus::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Mark execution as failed
    pub fn fail(&mut self, error: String) {
        self.status = ChainStatus::Failed(error);
        self.completed_at = Some(Utc::now());
    }

    /// Mark validation failure
    pub fn validation_failed(&mut self, error: String) {
        self.status = ChainStatus::ValidationFailed(error);
        self.completed_at = Some(Utc::now());
    }

    /// Get the duration of execution
    pub fn duration(&self) -> Option<Duration> {
        self.completed_at.map(|completed| {
            let duration = completed.signed_duration_since(self.started_at);
            Duration::from_secs(duration.num_seconds() as u64)
        })
    }
}

/// Result of a single step execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step identifier
    pub step_id: String,
    /// Output produced by the step
    pub output: String,
    /// Whether output passed validation
    pub validated: bool,
    /// Time taken to execute
    #[serde(
        serialize_with = "serialize_duration_required",
        deserialize_with = "deserialize_duration_required"
    )]
    pub duration: Duration,
    /// Number of retry attempts
    pub retry_count: u32,
    /// Timestamp of execution
    pub executed_at: DateTime<Utc>,
}

impl StepResult {
    /// Create a new step result
    pub fn new(step_id: String, output: String, validated: bool, duration: Duration) -> Self {
        Self {
            step_id,
            output,
            validated,
            duration,
            retry_count: 0,
            executed_at: Utc::now(),
        }
    }

    /// Increment retry count
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }
}

/// Status of chain execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum ChainStatus {
    /// Chain is currently executing
    Running,
    /// Chain completed successfully
    Completed,
    /// Chain execution failed
    Failed(String),
    /// Output validation failed
    ValidationFailed(String),
}

impl std::fmt::Display for ChainStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainStatus::Running => write!(f, "running"),
            ChainStatus::Completed => write!(f, "completed"),
            ChainStatus::Failed(err) => write!(f, "failed: {}", err),
            ChainStatus::ValidationFailed(err) => write!(f, "validation_failed: {}", err),
        }
    }
}

/// Chain validation errors
#[derive(Debug, thiserror::Error)]
pub enum ChainValidationError {
    #[error("Chain must have at least one step")]
    EmptyChain,
    #[error("Step {step_id} references invalid next step: {next_id}")]
    InvalidNextStep { step_id: String, next_id: String },
    #[error("Chain contains a cycle")]
    CycleDetected,
}

// Helper functions for Duration serialization/deserialization (Option<Duration>)
fn serialize_duration<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match duration {
        Some(d) => serializer.serialize_u64(d.as_secs()),
        None => serializer.serialize_none(),
    }
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let secs: Option<u64> = Option::deserialize(deserializer)?;
    Ok(secs.map(Duration::from_secs))
}

// Helper functions for Duration serialization/deserialization (non-optional)
fn serialize_duration_required<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(duration.as_secs())
}

fn deserialize_duration_required<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let secs: u64 = u64::deserialize(deserializer)?;
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_chain_creation() {
        let mut chain = PromptChain::new("Test Chain".to_string(), "A test chain".to_string());
        assert_eq!(chain.steps.len(), 0);

        let step = PromptStep::new(
            "step1".to_string(),
            "Test prompt".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        );
        chain.add_step(step);
        assert_eq!(chain.steps.len(), 1);
    }

    #[test]
    fn test_prompt_variable_replacement() {
        let step = PromptStep::new(
            "step1".to_string(),
            "Hello {name}, your age is {age}".to_string(),
            "Greeter".to_string(),
            OutputFormat::Plain,
        );

        let vars = serde_json::json!({
            "name": "Alice",
            "age": 30
        });

        let result = step.build_prompt(&vars).unwrap();
        assert_eq!(result, "Hello Alice, your age is 30");
    }

    #[test]
    fn test_chain_validation_empty() {
        let chain = PromptChain::new("Empty".to_string(), "Empty chain".to_string());
        assert!(matches!(
            chain.validate(),
            Err(ChainValidationError::EmptyChain)
        ));
    }

    #[test]
    fn test_chain_validation_invalid_next() {
        let mut chain = PromptChain::new("Invalid".to_string(), "Invalid chain".to_string());
        let step = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_next_step("nonexistent".to_string());
        chain.add_step(step);

        assert!(matches!(
            chain.validate(),
            Err(ChainValidationError::InvalidNextStep { .. })
        ));
    }

    #[test]
    fn test_chain_validation_cycle() {
        let mut chain = PromptChain::new("Cycle".to_string(), "Cyclic chain".to_string());
        let step1 = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_next_step("step2".to_string());
        let step2 = PromptStep::new(
            "step2".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_next_step("step1".to_string());

        chain.add_step(step1);
        chain.add_step(step2);

        assert!(matches!(
            chain.validate(),
            Err(ChainValidationError::CycleDetected)
        ));
    }

    #[test]
    fn test_chain_execution_lifecycle() {
        let mut execution = ChainExecution::new("chain1".to_string(), "task1".to_string());
        assert_eq!(execution.status, ChainStatus::Running);
        assert_eq!(execution.current_step, 0);

        let result =
            StepResult::new("step1".to_string(), "output".to_string(), true, Duration::from_secs(1));
        execution.add_result(result);
        assert_eq!(execution.current_step, 1);

        execution.complete();
        assert_eq!(execution.status, ChainStatus::Completed);
        assert!(execution.completed_at.is_some());
    }
}

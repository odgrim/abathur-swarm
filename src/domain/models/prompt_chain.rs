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
    /// Maximum number of steps to execute in parallel (default: unlimited)
    /// Controls DAG workflow execution concurrency
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<usize>,
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
            max_parallelism: None,
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

            // Verify all depends_on references are valid
            if let Some(deps) = &step.depends_on {
                for dep_id in deps {
                    if !self.steps.iter().any(|s| &s.id == dep_id) {
                        return Err(ChainValidationError::InvalidDependency {
                            step_id: step.id.clone(),
                            dependency_id: dep_id.clone(),
                        });
                    }

                    // Check for self-dependency
                    if dep_id == &step.id {
                        return Err(ChainValidationError::SelfDependency {
                            step_id: step.id.clone(),
                        });
                    }
                }
            }
        }

        // Check for cycles (handles both next_step and depends_on)
        if self.has_cycle() {
            return Err(ChainValidationError::CycleDetected);
        }

        Ok(())
    }

    /// Check if the chain has a cycle using DFS
    /// Handles both next_step (sequential) and depends_on (DAG) relationships
    fn has_cycle(&self) -> bool {
        use std::collections::{HashMap, HashSet};

        // Build adjacency list for all edges
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();

        for step in &self.steps {
            let edges = graph.entry(step.id.as_str()).or_insert_with(Vec::new);

            // Add next_step edge
            if let Some(next_id) = &step.next_step {
                edges.push(next_id.as_str());
            }

            // Add depends_on edges (reverse direction: if A depends_on B, then B -> A)
            if let Some(deps) = &step.depends_on {
                for dep_id in deps {
                    graph
                        .entry(dep_id.as_str())
                        .or_insert_with(Vec::new)
                        .push(step.id.as_str());
                }
            }
        }

        // DFS cycle detection with three colors
        let mut white: HashSet<&str> = self.steps.iter().map(|s| s.id.as_str()).collect();
        let mut gray: HashSet<&str> = HashSet::new();
        let mut black: HashSet<&str> = HashSet::new();

        fn visit_dfs<'a>(
            node: &'a str,
            graph: &HashMap<&'a str, Vec<&'a str>>,
            white: &mut HashSet<&'a str>,
            gray: &mut HashSet<&'a str>,
            black: &mut HashSet<&'a str>,
        ) -> bool {
            // Move from white to gray
            white.remove(node);
            gray.insert(node);

            // Visit all neighbors
            if let Some(neighbors) = graph.get(node) {
                for &neighbor in neighbors {
                    if black.contains(neighbor) {
                        continue; // Already processed
                    }
                    if gray.contains(neighbor) {
                        return true; // Back edge = cycle
                    }
                    if visit_dfs(neighbor, graph, white, gray, black) {
                        return true;
                    }
                }
            }

            // Move from gray to black
            gray.remove(node);
            black.insert(node);
            false
        }

        // Check all connected components
        while let Some(&node) = white.iter().next() {
            if visit_dfs(node, &graph, &mut white, &mut gray, &mut black) {
                return true;
            }
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
    /// Whether this step should create its own git branch/worktree
    /// If true, creates a branch according to branch_name_template
    /// If false, inherits branch from parent task
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_branch: Option<bool>,
    /// What to branch from (used when needs_branch is true)
    /// - "main": Branch from main/master
    /// - "feature_branch": Branch from task's feature_branch field
    /// - "parent_branch": Branch from parent task's branch field
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_parent: Option<String>,
    /// Branch name template with variable substitution
    /// Examples:
    ///   "feature/{feature_name}"
    ///   "task/{feature_name}/{step_id}"
    /// Variables available: feature_name, step_id, task_id, and any from step output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name_template: Option<String>,
    /// Decomposition configuration for spawning child tasks from step output
    /// This enables fan-out patterns where a step can create multiple branches
    /// and spawn tasks that continue the chain in parallel
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decomposition: Option<DecompositionConfig>,
    /// Memory storage configuration for step output
    /// When set, step output is stored to the memory system after successful execution
    /// This is a core feature - no shell hooks required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_in_memory: Option<StepMemoryConfig>,
    /// List of step IDs that must complete before this step can execute
    /// Enables DAG-based workflow execution with explicit dependencies
    /// If None, defaults to sequential execution (depends on previous step)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
}

/// Configuration for storing step output in memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepMemoryConfig {
    /// Memory key to store the output under
    /// Stored in namespace "step:{task_id}:{step_id}" with this key
    /// If not provided, defaults to "output"
    #[serde(default = "default_memory_key")]
    pub key: String,
    /// Memory type (semantic, episodic, procedural)
    /// Defaults to "semantic"
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
    /// Optional custom namespace template
    /// Supports variables: {task_id}, {step_id}, {feature_name}
    /// Defaults to "step:{task_id}:{step_id}"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_template: Option<String>,
}

fn default_memory_key() -> String {
    "output".to_string()
}

fn default_memory_type() -> String {
    "semantic".to_string()
}

/// Configuration for spawning tasks from step output (fan-out pattern)
///
/// Enables a step to dynamically create branches and spawn child tasks based
/// on its output. The parent task waits for all children before proceeding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionConfig {
    /// JSON path to extract items array from step output
    /// Example: "decomposition.subprojects" extracts from output.decomposition.subprojects
    pub items_path: String,

    /// Configuration applied to each item in the array
    pub per_item: PerItemConfig,

    /// Behavior after spawning all child tasks
    #[serde(default)]
    pub on_complete: OnDecompositionComplete,
}

/// Configuration for each item in the decomposition array
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerItemConfig {
    /// Branch configuration (required - each item gets its own branch)
    pub branch: BranchConfig,

    /// Task to spawn for this item
    pub task: TaskSpawnConfig,
}

/// Configuration for branch creation during decomposition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchConfig {
    /// Template for branch name
    /// Supports {item.*} substitution for item properties
    /// Example: "feature/{item.name}" with item {"name": "auth"} -> "feature/auth"
    pub template: String,

    /// Parent branch to create from
    /// - "main": Branch from main/master
    /// - "current": Branch from current task's branch
    /// - explicit name: Branch from specified branch
    #[serde(default = "default_branch_parent")]
    pub parent: String,
}

fn default_branch_parent() -> String {
    "main".to_string()
}

/// Configuration for task spawning during decomposition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpawnConfig {
    /// Agent type for the spawned task
    /// Can be templated: "{item.agent_type}" or hardcoded: "technical-requirements-specialist"
    pub agent_type: String,

    /// Summary template for the spawned task
    /// Supports {item.*} and {parent_task_id} substitution
    pub summary: String,

    /// Description template for the spawned task
    /// Supports {item.*} and {parent_task_id} substitution
    pub description: String,

    /// Priority for spawned tasks (default 5)
    #[serde(default = "default_priority")]
    pub priority: u8,

    /// Whether spawned tasks continue the chain execution
    /// If true, the child task continues at `continue_at_step`
    #[serde(default)]
    pub continue_chain: bool,

    /// Step ID to continue at in spawned tasks
    /// Required if continue_chain is true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continue_at_step: Option<String>,
}

fn default_priority() -> u8 {
    5
}

/// Behavior after decomposition completes
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OnDecompositionComplete {
    /// Wait for all spawned children before proceeding to next step
    /// If true, parent enters AwaitingChildren status until all complete
    /// If false, parent continues immediately (fire-and-forget pattern)
    #[serde(default = "default_wait_for_children")]
    pub wait_for_children: bool,
}

fn default_wait_for_children() -> bool {
    true
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
            needs_branch: None,
            branch_parent: None,
            branch_name_template: None,
            decomposition: None,
            store_in_memory: None,
            depends_on: None,
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

    /// Set the working directory for this step
    pub fn with_working_directory(mut self, working_directory: String) -> Self {
        self.working_directory = Some(working_directory);
        self
    }

    /// Set whether this step needs a branch
    pub fn with_needs_branch(mut self, needs_branch: bool) -> Self {
        self.needs_branch = Some(needs_branch);
        self
    }

    /// Set the branch parent
    pub fn with_branch_parent(mut self, branch_parent: String) -> Self {
        self.branch_parent = Some(branch_parent);
        self
    }

    /// Set the branch name template
    pub fn with_branch_name_template(mut self, branch_name_template: String) -> Self {
        self.branch_name_template = Some(branch_name_template);
        self
    }

    /// Set the decomposition configuration for fan-out pattern
    pub fn with_decomposition(mut self, decomposition: DecompositionConfig) -> Self {
        self.decomposition = Some(decomposition);
        self
    }

    /// Set the step dependencies for DAG-based execution
    pub fn with_depends_on(mut self, depends_on: Vec<String>) -> Self {
        self.depends_on = Some(depends_on);
        self
    }

    /// Build the actual prompt by replacing variables
    ///
    /// Automatically appends format instructions based on `expected_output` type.
    /// For JSON output, appends strict formatting requirements to ensure the agent
    /// outputs valid JSON that can be parsed by the validation system.
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

        // Append format instructions based on expected output type
        let format_instructions = self.get_format_instructions();
        if !format_instructions.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&format_instructions);
        }

        Ok(prompt)
    }

    /// Get format instructions based on the expected output type
    fn get_format_instructions(&self) -> String {
        match &self.expected_output {
            OutputFormat::Json { .. } => r#"
---
## OUTPUT FORMAT REQUIREMENT (CRITICAL)

Your response MUST be ONLY valid JSON. The system will parse your output as JSON.

RULES:
1. Output ONLY the JSON object - no prose, explanations, or summaries
2. Wrap your JSON in ```json code blocks
3. Do NOT write anything before or after the JSON block
4. Do NOT explain what you did or summarize results in text

CORRECT FORMAT:
```json
{
  "your": "data",
  "goes": "here"
}
```

INCORRECT (will cause validation failure):
- "Here is the result: {...}"
- "Task complete. The JSON is: {...}"
- Any text outside the JSON block"#
                .to_string(),
            OutputFormat::Markdown => String::new(), // No special instructions needed
            OutputFormat::Plain => String::new(),    // No special instructions needed
        }
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
    #[error("Step {step_id} references invalid dependency: {dependency_id}")]
    InvalidDependency {
        step_id: String,
        dependency_id: String,
    },
    #[error("Step {step_id} has a self-dependency")]
    SelfDependency { step_id: String },
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

        let result = StepResult::new(
            "step1".to_string(),
            "output".to_string(),
            true,
            Duration::from_secs(1),
        );
        execution.add_result(result);
        assert_eq!(execution.current_step, 1);

        execution.complete();
        assert_eq!(execution.status, ChainStatus::Completed);
        assert!(execution.completed_at.is_some());
    }

    #[test]
    fn test_build_prompt_appends_json_format_instructions() {
        let step = PromptStep::new(
            "step1".to_string(),
            "Analyze the data: {data}".to_string(),
            "Analyst".to_string(),
            OutputFormat::Json { schema: None },
        );

        let vars = serde_json::json!({
            "data": "test data"
        });

        let result = step.build_prompt(&vars).unwrap();

        // Should contain the original prompt with variable replaced
        assert!(result.contains("Analyze the data: test data"));

        // Should contain JSON format instructions
        assert!(result.contains("OUTPUT FORMAT REQUIREMENT"));
        assert!(result.contains("Your response MUST be ONLY valid JSON"));
        assert!(result.contains("```json"));
    }

    #[test]
    fn test_build_prompt_no_format_instructions_for_plain() {
        let step = PromptStep::new(
            "step1".to_string(),
            "Describe the problem".to_string(),
            "Assistant".to_string(),
            OutputFormat::Plain,
        );

        let vars = serde_json::json!({});
        let result = step.build_prompt(&vars).unwrap();

        // Should contain original prompt
        assert!(result.contains("Describe the problem"));

        // Should NOT contain JSON format instructions
        assert!(!result.contains("OUTPUT FORMAT REQUIREMENT"));
        assert!(!result.contains("MUST be ONLY valid JSON"));
    }

    #[test]
    fn test_step_memory_config_defaults() {
        // Test YAML parsing with minimal config
        let yaml = r#"
            key: "requirements"
        "#;
        let config: StepMemoryConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.key, "requirements");
        assert_eq!(config.memory_type, "semantic"); // default
        assert!(config.namespace_template.is_none());
    }

    #[test]
    fn test_step_memory_config_full() {
        let yaml = r#"
            key: "architecture"
            memory_type: "episodic"
            namespace_template: "task:{task_id}:arch"
        "#;
        let config: StepMemoryConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.key, "architecture");
        assert_eq!(config.memory_type, "episodic");
        assert_eq!(
            config.namespace_template,
            Some("task:{task_id}:arch".to_string())
        );
    }

    #[test]
    fn test_prompt_step_with_store_in_memory() {
        // Create step programmatically with store_in_memory
        let mut step = PromptStep::new(
            "gather_requirements".to_string(),
            "Gather requirements for {task}".to_string(),
            "requirements-gatherer".to_string(),
            OutputFormat::Json { schema: None },
        );
        step.store_in_memory = Some(StepMemoryConfig {
            key: "requirements".to_string(),
            memory_type: "semantic".to_string(),
            namespace_template: Some("task:{task_id}:requirements".to_string()),
        });

        assert_eq!(step.id, "gather_requirements");
        assert!(step.store_in_memory.is_some());

        let config = step.store_in_memory.unwrap();
        assert_eq!(config.key, "requirements");
        assert_eq!(config.memory_type, "semantic");
        assert_eq!(
            config.namespace_template,
            Some("task:{task_id}:requirements".to_string())
        );
    }

    #[test]
    fn test_prompt_step_without_store_in_memory() {
        // Create step programmatically without store_in_memory
        let step = PromptStep::new(
            "simple_step".to_string(),
            "Do something".to_string(),
            "worker".to_string(),
            OutputFormat::Plain,
        );

        assert_eq!(step.id, "simple_step");
        assert!(step.store_in_memory.is_none());
    }

    #[test]
    fn test_depends_on_builder_pattern() {
        let step = PromptStep::new(
            "step3".to_string(),
            "Execute step 3".to_string(),
            "Worker".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step1".to_string(), "step2".to_string()]);

        assert_eq!(
            step.depends_on,
            Some(vec!["step1".to_string(), "step2".to_string()])
        );
    }

    #[test]
    fn test_depends_on_serialization() {
        let step = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step0".to_string()]);

        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains("depends_on"));

        let deserialized: PromptStep = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.depends_on, Some(vec!["step0".to_string()]));
    }

    #[test]
    fn test_depends_on_backward_compatibility() {
        // Old JSON without depends_on field should deserialize with None
        let json = r#"{
            "id": "step1",
            "prompt_template": "Test",
            "role": "Tester",
            "expected_output": {"type": "plain"},
            "next_step": null,
            "timeout": null,
            "pre_hooks": [],
            "post_hooks": [],
            "working_directory": null,
            "needs_branch": null,
            "branch_parent": null,
            "branch_name_template": null,
            "decomposition": null,
            "store_in_memory": null
        }"#;

        let step: PromptStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.depends_on, None);
    }

    #[test]
    fn test_chain_validation_invalid_dependency() {
        let mut chain = PromptChain::new("Test".to_string(), "Test chain".to_string());
        let step = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["nonexistent".to_string()]);
        chain.add_step(step);

        assert!(matches!(
            chain.validate(),
            Err(ChainValidationError::InvalidDependency { .. })
        ));
    }

    #[test]
    fn test_chain_validation_self_dependency() {
        let mut chain = PromptChain::new("Test".to_string(), "Test chain".to_string());
        let step = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step1".to_string()]); // Self-dependency
        chain.add_step(step);

        assert!(matches!(
            chain.validate(),
            Err(ChainValidationError::SelfDependency { .. })
        ));
    }

    #[test]
    fn test_dag_cycle_detection() {
        let mut chain = PromptChain::new("DAG".to_string(), "DAG with cycle".to_string());

        // Create a cycle: step1 -> step2 -> step3 -> step1 (via depends_on)
        let step1 = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step3".to_string()]);

        let step2 = PromptStep::new(
            "step2".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step1".to_string()]);

        let step3 = PromptStep::new(
            "step3".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step2".to_string()]);

        chain.add_step(step1);
        chain.add_step(step2);
        chain.add_step(step3);

        assert!(matches!(
            chain.validate(),
            Err(ChainValidationError::CycleDetected)
        ));
    }

    #[test]
    fn test_dag_no_cycle_parallel_execution() {
        let mut chain = PromptChain::new("DAG".to_string(), "Valid DAG".to_string());

        // Create a valid DAG:
        // step1 and step2 have no dependencies (can run in parallel)
        // step3 depends on both step1 and step2
        let step1 = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        );

        let step2 = PromptStep::new(
            "step2".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        );

        let step3 = PromptStep::new(
            "step3".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        )
        .with_depends_on(vec!["step1".to_string(), "step2".to_string()]);

        chain.add_step(step1);
        chain.add_step(step2);
        chain.add_step(step3);

        assert!(chain.validate().is_ok());
    }

    #[test]
    fn test_max_parallelism_serialization() {
        let mut chain = PromptChain::new("Test".to_string(), "Test chain".to_string());
        chain.max_parallelism = Some(4);

        let json = serde_json::to_string(&chain).unwrap();
        assert!(json.contains("max_parallelism"));

        let deserialized: PromptChain = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_parallelism, Some(4));
    }

    #[test]
    fn test_max_parallelism_backward_compatibility() {
        // Old JSON without max_parallelism should deserialize with None
        let json = r#"{
            "id": "chain1",
            "name": "Test",
            "description": "Test chain",
            "steps": [],
            "validation_rules": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let chain: PromptChain = serde_json::from_str(json).unwrap();
        assert_eq!(chain.max_parallelism, None);
    }
}

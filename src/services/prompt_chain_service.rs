//! Prompt Chain Execution Service
//!
//! Coordinates execution of multi-step prompt chains with validation
//! and error handling between steps.

use crate::domain::models::prompt_chain::{
    ChainExecution, PromptChain, PromptStep, StepResult, ValidationRule,
};
use crate::domain::models::{HookContext, Task};
use crate::domain::ports::{ExecutionParameters, SubstrateRequest};
use crate::infrastructure::substrates::SubstrateRegistry;
use crate::infrastructure::validators::OutputValidator;
use crate::services::HookExecutor;
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, instrument, warn};

/// Service for executing prompt chains
///
/// Coordinates the execution of multi-step prompt chains with validation,
/// retry logic, and error handling.
///
/// # Examples
///
/// ```no_run
/// use abathur_cli::services::PromptChainService;
/// use abathur_cli::domain::models::prompt_chain::PromptChain;
/// use std::sync::Arc;
///
/// # async fn example() -> anyhow::Result<()> {
/// let service = PromptChainService::new();
///
/// let chain = PromptChain::new(
///     "research_chain".to_string(),
///     "Multi-step research process".to_string()
/// );
///
/// let initial_input = serde_json::json!({
///     "topic": "Rust async programming"
/// });
///
/// let execution = service.execute_chain(&chain, initial_input).await?;
/// # Ok(())
/// # }
/// ```
pub struct PromptChainService {
    validator: Arc<OutputValidator>,
    hook_executor: Option<Arc<HookExecutor>>,
    substrate_registry: Option<Arc<SubstrateRegistry>>,
    max_retries: u32,
    default_timeout: Duration,
}

impl PromptChainService {
    /// Create a new prompt chain service
    pub fn new() -> Self {
        Self {
            validator: Arc::new(OutputValidator::new()),
            hook_executor: None,
            substrate_registry: None,
            max_retries: 3,
            default_timeout: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Create a service with custom validator
    pub fn with_validator(validator: Arc<OutputValidator>) -> Self {
        Self {
            validator,
            hook_executor: None,
            substrate_registry: None,
            max_retries: 3,
            default_timeout: Duration::from_secs(300),
        }
    }

    /// Create a service with hook executor
    pub fn with_hook_executor(mut self, hook_executor: Arc<HookExecutor>) -> Self {
        self.hook_executor = Some(hook_executor);
        self
    }

    /// Set the substrate registry for executing prompts via LLM substrates
    pub fn with_substrate_registry(mut self, substrate_registry: Arc<SubstrateRegistry>) -> Self {
        self.substrate_registry = Some(substrate_registry);
        self
    }

    /// Set maximum retry attempts for failed steps
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Set default timeout for step execution
    pub fn with_default_timeout(mut self, timeout_secs: u64) -> Self {
        self.default_timeout = Duration::from_secs(timeout_secs);
        self
    }

    /// Execute a complete prompt chain
    ///
    /// Executes all steps in sequence, validating outputs and passing
    /// results between steps. Optionally executes pre/post hooks for each step.
    ///
    /// # Arguments
    /// * `chain` - The prompt chain to execute
    /// * `initial_input` - Initial variables for the first step
    ///
    /// # Returns
    /// * `Ok(ChainExecution)` - Execution results with all step outputs
    /// * `Err(_)` - If execution or validation fails
    #[instrument(skip(self, chain, initial_input), fields(chain_id = %chain.id, chain_name = %chain.name))]
    pub async fn execute_chain(
        &self,
        chain: &PromptChain,
        initial_input: serde_json::Value,
    ) -> Result<ChainExecution> {
        self.execute_chain_with_task(chain, initial_input, None).await
    }

    /// Execute a chain with an associated task for hook context
    ///
    /// This variant allows passing a task for hook execution context.
    ///
    /// # Arguments
    /// * `chain` - The prompt chain to execute
    /// * `initial_input` - Initial variables for the first step
    /// * `task` - Optional task for hook context
    ///
    /// # Returns
    /// * `Ok(ChainExecution)` - Execution results with all step outputs
    /// * `Err(_)` - If execution or validation fails
    #[instrument(skip(self, chain, initial_input, task), fields(chain_id = %chain.id, chain_name = %chain.name))]
    pub async fn execute_chain_with_task(
        &self,
        chain: &PromptChain,
        initial_input: serde_json::Value,
        task: Option<&Task>,
    ) -> Result<ChainExecution> {
        // Validate chain structure first
        chain.validate().context("Chain validation failed")?;

        info!(
            "Starting execution of chain '{}' with {} steps",
            chain.name,
            chain.steps.len()
        );

        let mut execution = ChainExecution::new(
            chain.id.clone(),
            uuid::Uuid::new_v4().to_string(), // Generate task ID
        );

        let mut current_input = initial_input;

        for (index, step) in chain.steps.iter().enumerate() {
            info!(
                "Executing step {}/{}: {} (role: {})",
                index + 1,
                chain.steps.len(),
                step.id,
                step.role
            );

            // Execute pre-hooks if any
            if !step.pre_hooks.is_empty() {
                info!("Executing {} pre-hooks for step {}", step.pre_hooks.len(), step.id);
                self.execute_hooks(&step.pre_hooks, task, &step.id, "pre").await?;
            }

            // Build the prompt with current variables
            let prompt = step
                .build_prompt(&current_input)
                .context(format!("Failed to build prompt for step {}", step.id))?;

            debug!("Built prompt for step {}: {}", step.id, prompt);

            // Execute the step with retries
            let result = self.execute_step_with_retry(step, &prompt).await?;

            // Validate output
            let validation_result =
                self.validate_step_output(step, &result, &chain.validation_rules)?;

            if !validation_result {
                error!("Validation failed for step {}", step.id);
                execution.validation_failed(format!("Step {} validation failed", step.id));
                return Ok(execution);
            }

            info!(
                "Step {} completed successfully in {:?}",
                step.id, result.duration
            );

            // Execute post-hooks if any
            if !step.post_hooks.is_empty() {
                info!("Executing {} post-hooks for step {}", step.post_hooks.len(), step.id);
                self.execute_hooks(&step.post_hooks, task, &step.id, "post").await?;
            }

            // Parse the output as the input for the next step
            current_input = self.prepare_next_input(&result)?;

            // Store the result
            execution.add_result(result);
        }

        execution.complete();
        info!(
            "Chain '{}' completed successfully in {:?}",
            chain.name,
            execution.duration().unwrap_or_default()
        );

        Ok(execution)
    }

    /// Execute a single step with retry logic
    #[instrument(skip(self, step, prompt), fields(step_id = %step.id))]
    async fn execute_step_with_retry(
        &self,
        step: &PromptStep,
        prompt: &str,
    ) -> Result<StepResult> {
        let mut retry_count = 0;
        let mut last_error = None;

        while retry_count <= self.max_retries {
            match self.execute_step(step, prompt).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    warn!(
                        "Step {} execution failed (attempt {}/{}): {}",
                        step.id,
                        retry_count + 1,
                        self.max_retries + 1,
                        e
                    );
                    last_error = Some(e);
                    retry_count += 1;

                    if retry_count <= self.max_retries {
                        // Exponential backoff
                        let delay = Duration::from_secs(2_u64.pow(retry_count));
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("Step execution failed after {} retries", self.max_retries)
        }))
    }

    /// Execute a single step
    #[instrument(skip(self, step, prompt), fields(step_id = %step.id, role = %step.role))]
    async fn execute_step(&self, step: &PromptStep, prompt: &str) -> Result<StepResult> {
        let start = Instant::now();

        // Get the timeout for this step
        let step_timeout = step.timeout.unwrap_or(self.default_timeout);

        debug!(
            "Executing step {} with timeout of {:?}",
            step.id, step_timeout
        );

        // Execute the prompt (this would call the actual LLM API)
        // For now, we'll simulate execution
        let output = match timeout(step_timeout, self.execute_prompt(prompt, &step.role)).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Step {} timed out after {:?}",
                    step.id,
                    step_timeout
                ));
            }
        };

        let duration = start.elapsed();

        // Validate output format
        let validated = match self.validator.validate(&output, &step.expected_output) {
            Ok(_) => true,
            Err(e) => {
                error!(
                    "Output format validation failed for step {}: {}",
                    step.id, e
                );
                false
            }
        };

        Ok(StepResult::new(
            step.id.clone(),
            output,
            validated,
            duration,
        ))
    }

    /// Execute a prompt via LLM substrate (Claude Code CLI or Anthropic API)
    async fn execute_prompt(&self, prompt: &str, role: &str) -> Result<String> {
        debug!("Executing prompt with role: {}", role);

        // Check if substrate registry is configured
        let Some(ref registry) = self.substrate_registry else {
            warn!("No substrate registry configured, returning mock response");
            // Fallback to mock response if no substrate available (for tests)
            return Ok(serde_json::json!({
                "role": role,
                "response": format!("Mock response (no substrate): {}", prompt),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })
            .to_string());
        };

        // Create a substrate request
        let request = SubstrateRequest {
            task_id: uuid::Uuid::new_v4(), // Generate ephemeral task ID for this step
            agent_type: role.to_string(),
            prompt: prompt.to_string(),
            context: None,
            parameters: ExecutionParameters {
                model: None, // Use default model for role
                max_tokens: Some(4096),
                temperature: Some(0.7),
                timeout_secs: None, // Use default timeout
                extra: std::collections::HashMap::new(),
            },
        };

        // Execute via substrate
        info!(
            role = %role,
            prompt_length = prompt.len(),
            "Executing prompt chain step via substrate"
        );

        let response = match registry.execute(request).await {
            Ok(resp) => resp,
            Err(e) => {
                error!(
                    role = %role,
                    prompt_length = prompt.len(),
                    error = %e,
                    "Substrate execution failed"
                );
                return Err(anyhow::anyhow!(
                    "Failed to execute prompt via substrate: {}",
                    e
                ));
            }
        };

        info!(
            role = %role,
            output_length = response.content.len(),
            stop_reason = ?response.stop_reason,
            "Prompt chain step completed"
        );

        Ok(response.content)
    }

    /// Validate step output against validation rules
    fn validate_step_output(
        &self,
        step: &PromptStep,
        result: &StepResult,
        rules: &[ValidationRule],
    ) -> Result<bool> {
        // First check if basic format validation passed
        if !result.validated {
            return Ok(false);
        }

        // Apply any additional validation rules for this step
        let step_rules: Vec<&ValidationRule> = rules.iter().filter(|r| r.step_id == step.id).collect();

        for rule in step_rules {
            match self
                .validator
                .validate_with_rule(&result.output, &rule.rule_type, rule.schema.as_ref())
            {
                Ok(true) => continue,
                Ok(false) => {
                    error!("Validation rule failed: {}", rule.error_message);
                    return Ok(false);
                }
                Err(e) => {
                    error!(
                        "Validation error for step {}: {} - {}",
                        step.id, rule.error_message, e
                    );
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// Prepare input for the next step from the current result
    fn prepare_next_input(&self, result: &StepResult) -> Result<serde_json::Value> {
        // Try to parse output as JSON for the next step
        match serde_json::from_str(&result.output) {
            Ok(value) => Ok(value),
            Err(_) => {
                // If not JSON, wrap in a generic structure
                Ok(serde_json::json!({
                    "previous_output": result.output,
                    "previous_step": result.step_id
                }))
            }
        }
    }

    /// Execute a list of hook actions
    ///
    /// # Arguments
    /// * `hooks` - The hook actions to execute
    /// * `task` - Optional task context
    /// * `step_id` - ID of the current step (for context)
    /// * `hook_type` - Type of hook ("pre" or "post") for logging
    #[instrument(skip(self, hooks, task))]
    async fn execute_hooks(
        &self,
        hooks: &[crate::domain::models::HookAction],
        task: Option<&Task>,
        step_id: &str,
        hook_type: &str,
    ) -> Result<()> {
        let Some(executor) = &self.hook_executor else {
            warn!("Hook executor not configured, skipping {} hooks for step {}", hook_type, step_id);
            return Ok(());
        };

        let Some(task) = task else {
            warn!("No task context provided for hooks, skipping {} hooks for step {}", hook_type, step_id);
            return Ok(());
        };

        // Build hook context with variables for template substitution
        let mut variables = std::collections::HashMap::new();
        variables.insert("task_id".to_string(), task.id.to_string());
        variables.insert("chain_step_id".to_string(), step_id.to_string());
        variables.insert("hook_type".to_string(), hook_type.to_string());
        variables.insert("agent_type".to_string(), task.agent_type.clone());

        if let Some(parent_id) = &task.parent_task_id {
            variables.insert("parent_task_id".to_string(), parent_id.to_string());
        }

        let context = HookContext {
            variables,
            task_id: Some(task.id),
            branch_context: None,
        };

        // Execute each hook in sequence
        for (idx, hook) in hooks.iter().enumerate() {
            debug!(
                "Executing {} hook {}/{} for step {}",
                hook_type,
                idx + 1,
                hooks.len(),
                step_id
            );

            match executor.execute_action(hook, task, &context).await {
                Ok(result) => {
                    info!(
                        "Hook {}/{} executed successfully: {:?}",
                        idx + 1,
                        hooks.len(),
                        result
                    );
                }
                Err(e) => {
                    error!(
                        "Hook {}/{} failed for step {}: {}",
                        idx + 1,
                        hooks.len(),
                        step_id,
                        e
                    );
                    // Continue with other hooks even if one fails
                    // You could make this configurable to fail fast if needed
                }
            }
        }

        Ok(())
    }

    /// Resume a failed chain execution from a specific step
    #[instrument(skip(self, chain, execution, resume_from_step))]
    pub async fn resume_execution(
        &self,
        chain: &PromptChain,
        execution: &mut ChainExecution,
        resume_from_step: usize,
    ) -> Result<()> {
        if resume_from_step >= chain.steps.len() {
            return Err(anyhow::anyhow!(
                "Invalid resume step: {} (chain has {} steps)",
                resume_from_step,
                chain.steps.len()
            ));
        }

        info!(
            "Resuming chain execution from step {}/{}",
            resume_from_step + 1,
            chain.steps.len()
        );

        // Get the last successful result to use as input
        let current_input = if let Some(last_result) = execution.step_results.last() {
            self.prepare_next_input(last_result)?
        } else {
            return Err(anyhow::anyhow!(
                "Cannot resume: no previous results available"
            ));
        };

        // Continue execution from the specified step
        let mut current_input = current_input;
        for (index, step) in chain.steps.iter().enumerate().skip(resume_from_step) {
            info!(
                "Executing step {}/{}: {}",
                index + 1,
                chain.steps.len(),
                step.id
            );

            let prompt = step.build_prompt(&current_input)?;
            let result = self.execute_step_with_retry(step, &prompt).await?;

            let validation_result =
                self.validate_step_output(step, &result, &chain.validation_rules)?;

            if !validation_result {
                execution.validation_failed(format!("Step {} validation failed", step.id));
                return Ok(());
            }

            current_input = self.prepare_next_input(&result)?;
            execution.add_result(result);
        }

        execution.complete();
        Ok(())
    }
}

impl Default for PromptChainService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::prompt_chain::{OutputFormat, PromptChain, PromptStep};

    #[tokio::test]
    async fn test_execute_simple_chain() {
        let service = PromptChainService::new();

        let mut chain = PromptChain::new(
            "test_chain".to_string(),
            "Test chain".to_string(),
        );

        let step1 = PromptStep::new(
            "step1".to_string(),
            "Process {input}".to_string(),
            "Processor".to_string(),
            OutputFormat::Json { schema: None },
        );

        chain.add_step(step1);

        let input = serde_json::json!({
            "input": "test data"
        });

        let result = service.execute_chain(&chain, input).await;
        assert!(result.is_ok());

        let execution = result.unwrap();
        assert_eq!(execution.status, ChainStatus::Completed);
        assert_eq!(execution.step_results.len(), 1);
    }

    #[tokio::test]
    async fn test_execute_multi_step_chain() {
        let service = PromptChainService::new();

        let mut chain = PromptChain::new(
            "multi_step".to_string(),
            "Multi-step chain".to_string(),
        );

        let step1 = PromptStep::new(
            "step1".to_string(),
            "Extract data from {source}".to_string(),
            "Extractor".to_string(),
            OutputFormat::Json { schema: None },
        )
        .with_next_step("step2".to_string());

        let step2 = PromptStep::new(
            "step2".to_string(),
            "Transform {previous_output}".to_string(),
            "Transformer".to_string(),
            OutputFormat::Json { schema: None },
        );

        chain.add_step(step1);
        chain.add_step(step2);

        let input = serde_json::json!({
            "source": "test.txt"
        });

        let result = service.execute_chain(&chain, input).await;
        assert!(result.is_ok());

        let execution = result.unwrap();
        assert_eq!(execution.status, ChainStatus::Completed);
        assert_eq!(execution.step_results.len(), 2);
    }

    #[test]
    fn test_prepare_next_input_json() {
        let service = PromptChainService::new();
        let result = StepResult::new(
            "step1".to_string(),
            r#"{"key": "value"}"#.to_string(),
            true,
            Duration::from_secs(1),
        );

        let next_input = service.prepare_next_input(&result).unwrap();
        assert_eq!(next_input["key"], "value");
    }

    #[test]
    fn test_prepare_next_input_plain() {
        let service = PromptChainService::new();
        let result = StepResult::new(
            "step1".to_string(),
            "Plain text output".to_string(),
            true,
            Duration::from_secs(1),
        );

        let next_input = service.prepare_next_input(&result).unwrap();
        assert_eq!(next_input["previous_output"], "Plain text output");
        assert_eq!(next_input["previous_step"], "step1");
    }
}

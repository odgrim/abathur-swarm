//! Prompt Chain Execution Service
//!
//! Coordinates execution of multi-step prompt chains with validation
//! and error handling between steps.

use crate::domain::models::prompt_chain::{
    ChainExecution, PromptChain, PromptStep, StepResult, ValidationRule,
};
use crate::domain::models::{AgentMetadata, AgentMetadataRegistry, HookContext, Task};
use crate::domain::ports::{ExecutionParameters, SubstrateRequest, TaskQueueService};
use crate::infrastructure::substrates::SubstrateRegistry;
use crate::infrastructure::validators::OutputValidator;
use crate::services::HookExecutor;
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
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
    agent_metadata_registry: Option<Arc<Mutex<AgentMetadataRegistry>>>,
    task_queue_service: Option<Arc<dyn TaskQueueService>>,
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
            agent_metadata_registry: None,
            task_queue_service: None,
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
            agent_metadata_registry: None,
            task_queue_service: None,
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

    /// Set the agent metadata registry for loading agent definitions
    pub fn with_agent_metadata_registry(mut self, agent_metadata_registry: Arc<Mutex<AgentMetadataRegistry>>) -> Self {
        self.agent_metadata_registry = Some(agent_metadata_registry);
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

    /// Set the task queue service for spawning child tasks
    pub fn with_task_queue_service(mut self, task_queue_service: Arc<dyn TaskQueueService>) -> Self {
        self.task_queue_service = Some(task_queue_service);
        self
    }

    /// Submit a task to the task queue (wrapper for AgentExecutor to use)
    pub async fn submit_task(&self, task: Task) -> Result<uuid::Uuid> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        task_queue.submit_task(task).await
    }

    /// Execute a single step of a prompt chain
    ///
    /// This is called by AgentExecutor for each chain step task.
    /// It executes one step and returns the result.
    ///
    /// # Arguments
    /// * `chain` - The full chain definition
    /// * `step` - The specific step to execute
    /// * `input` - Input data for this step
    /// * `task` - Optional task context for hooks
    ///
    /// # Returns
    /// * `Ok(StepResult)` - Step execution result
    /// * `Err(_)` - If execution or validation fails
    #[instrument(skip(self, chain, step, input, task), fields(step_id = %step.id, role = %step.role))]
    pub async fn execute_single_step(
        &self,
        chain: &PromptChain,
        step: &PromptStep,
        input: &serde_json::Value,
        task: Option<&Task>,
    ) -> Result<StepResult> {
        info!(
            step_id = %step.id,
            role = %step.role,
            "Executing single chain step"
        );

        // Execute pre-hooks if any
        if !step.pre_hooks.is_empty() {
            info!("Executing {} pre-hooks for step {}", step.pre_hooks.len(), step.id);
            self.execute_hooks(&step.pre_hooks, task, &step.id, "pre").await?;
        }

        // Build the prompt with current variables
        let prompt = step
            .build_prompt(input)
            .context(format!("Failed to build prompt for step {}", step.id))?;

        debug!("Built prompt for step {}: {}", step.id, prompt);

        // Execute the step with retries
        let result = self.execute_step_with_retry(step, &prompt).await?;

        // Validate output
        if let Err(e) = self.validate_step_output(step, &result, &chain.validation_rules) {
            let error_msg = format!("Step {} validation failed: {}", step.id, e);
            error!("{}", error_msg);
            anyhow::bail!("{}", error_msg);
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

        // Check if this step should spawn implementation tasks
        if self.should_spawn_tasks(step) {
            info!(
                step_id = %step.id,
                "Step configured to spawn tasks, parsing output"
            );

            if let Err(e) = self.spawn_tasks_from_output(&result, task).await {
                error!(
                    step_id = %step.id,
                    error = ?e,
                    "Failed to spawn tasks from step output"
                );
                // Don't fail the step if task spawning fails, just log it
            }
        }

        Ok(result)
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

            // Validate output - returns Err with detailed message if validation fails
            if let Err(e) = self.validate_step_output(step, &result, &chain.validation_rules) {
                let error_msg = format!("Step {} validation failed: {}", step.id, e);
                error!("{}", error_msg);
                execution.validation_failed(error_msg);
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
            execution.add_result(result.clone());

            // Check if this step should spawn tasks
            if self.should_spawn_tasks(step) {
                info!(
                    step_id = %step.id,
                    "Step configured to spawn tasks, parsing output"
                );

                if let Err(e) = self.spawn_tasks_from_output(&result, task).await {
                    error!(
                        step_id = %step.id,
                        error = ?e,
                        "Failed to spawn tasks from step output"
                    );
                    // Don't fail the chain if task spawning fails, just log it
                    // The monitoring step will detect missing tasks
                }
            }
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

        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        info!(
            step_id = %step.id,
            step_timeout = ?step.timeout,
            step_timeout_secs = step.timeout.as_ref().map(|d| d.as_secs()),
            default_timeout_secs = self.default_timeout.as_secs(),
            "PromptChainService: Step timeout before unwrap_or"
        );

        // Get the timeout for this step
        let step_timeout = step.timeout.unwrap_or(self.default_timeout);

        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        info!(
            step_id = %step.id,
            final_timeout_secs = step_timeout.as_secs(),
            used_default = step.timeout.is_none(),
            "PromptChainService: Final timeout that will be used for execution"
        );

        debug!(
            "Executing step {} with timeout of {:?}",
            step.id, step_timeout
        );

        // Execute the prompt (this would call the actual LLM API)
        // Pass the timeout to the prompt execution
        let output = match timeout(step_timeout, self.execute_prompt(prompt, &step.role, step_timeout.as_secs())).await {
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
        let (validated, validation_error) = match self.validator.validate(&output, &step.expected_output) {
            Ok(_) => (true, None),
            Err(e) => {
                let error_msg = format!("Output format validation failed: {}", e);
                error!(
                    "Output format validation failed for step {}: {}",
                    step.id, e
                );
                (false, Some(error_msg))
            }
        };

        // If validation failed, return error with details for retry feedback
        if !validated {
            return Err(anyhow::anyhow!(
                "Validation failed for step {}: {}",
                step.id,
                validation_error.unwrap_or_else(|| "Unknown validation error".to_string())
            ));
        }

        Ok(StepResult::new(
            step.id.clone(),
            output,
            validated,
            duration,
        ))
    }

    /// Build complete prompt by combining agent definition with step-specific prompt
    ///
    /// This matches the behavior of AgentExecutor.build_prompt() to ensure consistent
    /// prompt construction across single-agent and chain executions.
    fn build_chain_step_prompt(&self, step_prompt: &str, role: &str) -> String {
        let mut full_prompt = String::new();

        // Load the agent definition if registry is available
        if let Some(ref registry) = self.agent_metadata_registry {
            if let Ok(agent_file_path) = registry
                .lock()
                .unwrap()
                .get_agent_file_path(role)
            {
                // Read the agent file and extract the prompt content
                match std::fs::read_to_string(&agent_file_path) {
                    Ok(file_content) => {
                        match AgentMetadata::extract_prompt_content(&file_content) {
                            Ok(agent_prompt) => {
                                // Add the full agent definition as base prompt
                                let _ = write!(full_prompt, "{}\n\n", agent_prompt);

                                debug!(
                                    role = %role,
                                    agent_prompt_length = agent_prompt.len(),
                                    "Loaded agent definition for chain step"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    role = %role,
                                    error = %e,
                                    "Failed to extract agent prompt content, using step prompt only"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            role = %role,
                            error = %e,
                            "Failed to read agent file, using step prompt only"
                        );
                    }
                }
            } else {
                warn!(
                    role = %role,
                    "Could not find agent file, using step prompt only"
                );
            }
        } else {
            debug!(
                role = %role,
                "No agent metadata registry configured, using step prompt only"
            );
        }

        // Add the step-specific prompt (task instructions from chain YAML)
        let _ = write!(full_prompt, "{}", step_prompt);

        full_prompt
    }

    /// Execute a prompt via LLM substrate (Claude Code CLI or Anthropic API)
    async fn execute_prompt(&self, prompt: &str, role: &str, timeout_secs: u64) -> Result<String> {
        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        info!(
            role = %role,
            timeout_secs = timeout_secs,
            "PromptChainService: execute_prompt called with timeout"
        );

        debug!("Executing prompt with role: {}", role);

        // Build complete prompt with agent definition + step prompt
        let full_prompt = self.build_chain_step_prompt(prompt, role);

        debug!(
            role = %role,
            step_prompt_length = prompt.len(),
            full_prompt_length = full_prompt.len(),
            "Built complete prompt with agent definition"
        );

        // Check if substrate registry is configured
        let Some(ref registry) = self.substrate_registry else {
            warn!("No substrate registry configured, returning mock response");
            // Fallback to mock response if no substrate available (for tests)
            return Ok(serde_json::json!({
                "role": role,
                "response": format!("Mock response (no substrate): {}", full_prompt),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })
            .to_string());
        };

        // Create a substrate request with full prompt
        let request = SubstrateRequest {
            task_id: uuid::Uuid::new_v4(), // Generate ephemeral task ID for this step
            agent_type: role.to_string(),
            prompt: full_prompt,
            context: None,
            parameters: ExecutionParameters {
                model: None, // Use default model for role
                max_tokens: Some(4096),
                temperature: Some(0.7),
                timeout_secs: Some(timeout_secs), // Use configured timeout
                extra: std::collections::HashMap::new(),
            },
        };

        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        info!(
            role = %role,
            request_timeout_secs = request.parameters.timeout_secs,
            "PromptChainService: SubstrateRequest created with timeout"
        );

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
    /// Returns Ok(()) if validation passed, or Err with detailed message if failed
    fn validate_step_output(
        &self,
        step: &PromptStep,
        result: &StepResult,
        rules: &[ValidationRule],
    ) -> Result<()> {
        // First check if basic format validation passed
        if !result.validated {
            anyhow::bail!(
                "Step {} failed basic format validation. Output may not match expected format.",
                step.id
            );
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
                    anyhow::bail!("Validation rule failed: {}", rule.error_message);
                }
                Err(e) => {
                    error!(
                        "Validation error for step {}: {} - {}",
                        step.id, rule.error_message, e
                    );
                    anyhow::bail!(
                        "Validation error for step {}: {} - {}",
                        step.id,
                        rule.error_message,
                        e
                    );
                }
            }
        }

        Ok(())
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

            // Validate output - returns Err with detailed message if validation fails
            if let Err(e) = self.validate_step_output(step, &result, &chain.validation_rules) {
                let error_msg = format!("Step {} validation failed: {}", step.id, e);
                error!("{}", error_msg);
                execution.validation_failed(error_msg);
                return Ok(());
            }

            current_input = self.prepare_next_input(&result)?;
            execution.add_result(result);
        }

        execution.complete();
        Ok(())
    }

    /// Check if a step should spawn tasks based on its role or ID
    fn should_spawn_tasks(&self, step: &PromptStep) -> bool {
        // Only spawn tasks if we have a task queue service configured
        if self.task_queue_service.is_none() {
            return false;
        }

        // Steps that create task plans should spawn tasks
        matches!(
            step.id.as_str(),
            "create_task_plan" | "plan_tasks" | "spawn_tasks"
        ) || step.role == "task-planner"
    }

    /// Parse and spawn tasks from step output
    async fn spawn_tasks_from_output(
        &self,
        result: &StepResult,
        parent_task: Option<&Task>,
    ) -> Result<()> {
        let Some(ref task_queue) = self.task_queue_service else {
            warn!("Task queue service not configured, cannot spawn tasks");
            return Ok(());
        };

        // Parse the output as JSON
        let output_json: serde_json::Value = serde_json::from_str(&result.output)
            .context("Failed to parse step output as JSON")?;

        // Extract the tasks array
        let tasks_array = output_json
            .get("tasks")
            .and_then(|v| v.as_array())
            .context("Step output missing 'tasks' array")?;

        if tasks_array.is_empty() {
            info!("No tasks found in step output");
            return Ok(());
        }

        info!(
            task_count = tasks_array.len(),
            "Parsing {} tasks from step output",
            tasks_array.len()
        );

        let mut spawned_count = 0;
        let mut failed_count = 0;

        // Parse and enqueue each task
        for (idx, task_def) in tasks_array.iter().enumerate() {
            match self.parse_and_enqueue_task(task_def, parent_task, task_queue.as_ref()).await {
                Ok(task_id) => {
                    info!(
                        index = idx,
                        task_id = %task_id,
                        "Successfully enqueued task"
                    );
                    spawned_count += 1;
                }
                Err(e) => {
                    error!(
                        index = idx,
                        error = ?e,
                        task_def = ?task_def,
                        "Failed to enqueue task"
                    );
                    failed_count += 1;
                }
            }
        }

        info!(
            spawned = spawned_count,
            failed = failed_count,
            total = tasks_array.len(),
            "Task spawning complete"
        );

        if failed_count > 0 {
            warn!(
                "{} out of {} tasks failed to enqueue",
                failed_count,
                tasks_array.len()
            );
        }

        Ok(())
    }

    /// Parse a task definition and enqueue it
    async fn parse_and_enqueue_task(
        &self,
        task_def: &serde_json::Value,
        parent_task: Option<&Task>,
        task_queue: &dyn TaskQueueService,
    ) -> Result<uuid::Uuid> {
        use crate::domain::models::{DependencyType, TaskSource, TaskStatus};

        // Extract required fields
        let summary = task_def
            .get("summary")
            .and_then(|v| v.as_str())
            .context("Task missing 'summary' field")?
            .to_string();

        let description = task_def
            .get("description")
            .and_then(|v| v.as_str())
            .context("Task missing 'description' field")?
            .to_string();

        let agent_type = task_def
            .get("agent_type")
            .and_then(|v| v.as_str())
            .context("Task missing 'agent_type' field")?
            .to_string();

        // Extract optional fields
        let priority = task_def
            .get("priority")
            .and_then(|v| v.as_u64())
            .map(|v| v as u8)
            .unwrap_or(5);

        let dependencies = task_def
            .get("dependencies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(|s| uuid::Uuid::parse_str(s).ok())
                    .collect::<Vec<_>>()
            });

        let needs_worktree = task_def
            .get("needs_worktree")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let feature_branch = if needs_worktree {
            parent_task.and_then(|t| t.feature_branch.clone())
        } else {
            None
        };

        let input_data = task_def.get("input_data").cloned();

        let now = chrono::Utc::now();

        // Create the task
        let task = Task {
            id: uuid::Uuid::new_v4(),
            summary,
            description,
            agent_type,
            priority,
            calculated_priority: f64::from(priority),
            status: TaskStatus::Pending,
            dependencies,
            dependency_type: DependencyType::Sequential,
            dependency_depth: 0,
            input_data,
            result_data: None,
            error_message: None,
            retry_count: 0,
            max_retries: 3,
            max_execution_timeout_seconds: 3600,
            submitted_at: now,
            started_at: None,
            completed_at: None,
            last_updated_at: now,
            created_by: Some("prompt-chain".to_string()),
            parent_task_id: parent_task.map(|t| t.id),
            session_id: parent_task.and_then(|t| t.session_id),
            source: TaskSource::AgentPlanner,
            deadline: None,
            estimated_duration_seconds: None,
            feature_branch,
            task_branch: None,
            worktree_path: None,
            validation_requirement: crate::domain::models::ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
            chain_step_index: 0,
        };

        // Submit to task queue
        let task_id = task_queue.submit_task(task).await?;

        Ok(task_id)
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
    use crate::domain::models::prompt_chain::{ChainStatus, OutputFormat, PromptChain, PromptStep};

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

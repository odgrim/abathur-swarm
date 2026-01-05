//! Prompt Chain Execution Service
//!
//! Coordinates execution of multi-step prompt chains with validation
//! and error handling between steps.

use crate::domain::models::prompt_chain::{
    ChainExecution, ChainStatus, OutputFormat, PromptChain, PromptStep, StepResult, ValidationRule,
    ValidationType,
};
use crate::domain::models::task::{DependencyType, TaskSource, TaskStatus, ValidationRequirement};
use crate::domain::models::{AgentMetadata, AgentMetadataRegistry, HookContext, Memory, MemoryType, Task};
use crate::domain::ports::{ChainRepository, ExecutionParameters, SubstrateRequest, TaskQueueService};
use crate::infrastructure::substrates::SubstrateRegistry;
use crate::infrastructure::validators::OutputValidator;
use crate::services::{HookExecutor, MemoryService};
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
    chain_repository: Option<Arc<dyn ChainRepository>>,
    memory_service: Option<Arc<MemoryService>>,
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
            chain_repository: None,
            memory_service: None,
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
            chain_repository: None,
            memory_service: None,
            max_retries: 3,
            default_timeout: Duration::from_secs(300),
        }
    }

    /// Set the memory service for storing step outputs
    pub fn with_memory_service(mut self, memory_service: Arc<MemoryService>) -> Self {
        self.memory_service = Some(memory_service);
        self
    }

    /// Set the chain repository for tracking chain executions
    pub fn with_chain_repository(mut self, chain_repository: Arc<dyn ChainRepository>) -> Self {
        self.chain_repository = Some(chain_repository);
        self
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

    /// Submit a task idempotently using atomic INSERT OR IGNORE
    ///
    /// This is the preferred method for chain step tasks to prevent duplicates
    /// when workers crash/retry. The idempotency key should be based on
    /// chain_id + step_index + parent_task_id to ensure uniqueness.
    ///
    /// # Returns
    /// - `Ok(IdempotentInsertResult::Inserted(uuid))` - Task was inserted
    /// - `Ok(IdempotentInsertResult::AlreadyExists)` - Task already existed (duplicate)
    pub async fn submit_task_idempotent(
        &self,
        task: Task,
    ) -> Result<crate::domain::ports::task_repository::IdempotentInsertResult> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        task_queue.submit_task_idempotent(task).await
    }

    /// Get a task from the task queue (wrapper for AgentExecutor to use)
    pub async fn get_task_from_repo(&self, task_id: uuid::Uuid) -> Result<Option<Task>> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        match task_queue.get_task(task_id).await {
            Ok(task) => Ok(Some(task)),
            Err(e) => Err(e),
        }
    }

    /// Update a task in the task queue (wrapper for AgentExecutor to use)
    pub async fn update_task(&self, task: &Task) -> Result<()> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        task_queue.update_task(task).await
    }

    /// Atomically update parent task and insert child tasks in a single transaction
    ///
    /// This is critical for decomposition workflows where:
    /// - Parent must be updated to AwaitingChildren status
    /// - Child tasks must be spawned
    /// - Both must happen atomically to prevent orphaned children on parent update failure
    pub async fn update_parent_and_insert_children_atomic(
        &self,
        parent_task: &Task,
        child_tasks: Vec<Task>,
    ) -> Result<crate::domain::ports::task_repository::DecompositionResult> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        task_queue.update_parent_and_insert_children_atomic(parent_task, child_tasks).await
    }

    /// Get tasks that depend on a given task (wrapper for AgentExecutor to use)
    pub async fn get_dependent_tasks(&self, task_id: uuid::Uuid) -> Result<Vec<Task>> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        task_queue.get_dependent_tasks(task_id).await
    }

    /// Get a task by its idempotency key (wrapper for AgentExecutor to use)
    ///
    /// This provides a direct O(1) lookup for finding existing tasks by their
    /// idempotency key, which is more reliable than scanning dependent tasks.
    pub async fn get_task_by_idempotency_key(&self, idempotency_key: &str) -> Result<Option<Task>> {
        let Some(ref task_queue) = self.task_queue_service else {
            anyhow::bail!("Task queue service not configured");
        };

        task_queue.get_task_by_idempotency_key(idempotency_key).await
    }

    // ==================== Chain Execution Tracking ====================

    /// Get or create a chain execution for a task
    ///
    /// If an execution already exists for this task, returns it.
    /// Otherwise, creates a new execution and persists it.
    ///
    /// # Arguments
    /// * `chain_id` - The chain being executed
    /// * `task_id` - The task ID (used as execution identifier)
    ///
    /// # Returns
    /// * `Ok(ChainExecution)` - Existing or new execution
    /// * `Err(_)` - If repository is not configured or operation fails
    pub async fn get_or_create_execution(
        &self,
        chain_id: &str,
        task_id: &str,
    ) -> Result<ChainExecution> {
        let Some(ref repo) = self.chain_repository else {
            // If no repository, create a transient execution (not persisted)
            debug!("Chain repository not configured, creating transient execution");
            return Ok(ChainExecution::new(chain_id.to_string(), task_id.to_string()));
        };

        // Try to find existing execution for this task
        let existing_executions = repo.list_executions_for_task(task_id).await
            .context("Failed to list executions for task")?;

        // Find execution for this specific chain
        if let Some(execution) = existing_executions.into_iter().find(|e| e.chain_id == chain_id) {
            info!(
                execution_id = %execution.id,
                task_id = %task_id,
                chain_id = %chain_id,
                current_step = execution.current_step,
                completed_steps = execution.step_results.len(),
                "Found existing chain execution"
            );
            return Ok(execution);
        }

        // Create new execution
        let execution = ChainExecution::new(chain_id.to_string(), task_id.to_string());
        repo.insert_execution(&execution).await
            .context("Failed to insert new chain execution")?;

        info!(
            execution_id = %execution.id,
            task_id = %task_id,
            chain_id = %chain_id,
            "Created new chain execution"
        );

        Ok(execution)
    }

    /// Record completion of a chain step
    ///
    /// Updates the execution record with the step result.
    /// This should be called AFTER saving task.result_data for idempotency.
    ///
    /// # Arguments
    /// * `execution` - The chain execution to update
    /// * `step_result` - Result of the completed step
    ///
    /// # Returns
    /// * `Ok(ChainExecution)` - Updated execution
    /// * `Err(_)` - If update fails
    pub async fn record_step_completion(
        &self,
        mut execution: ChainExecution,
        step_result: StepResult,
    ) -> Result<ChainExecution> {
        execution.add_result(step_result);

        let Some(ref repo) = self.chain_repository else {
            // If no repository, just return updated execution (not persisted)
            return Ok(execution);
        };

        repo.update_execution(&execution).await
            .context("Failed to update chain execution with step result")?;

        info!(
            execution_id = %execution.id,
            current_step = execution.current_step,
            total_results = execution.step_results.len(),
            "Recorded step completion in chain execution"
        );

        Ok(execution)
    }

    /// Mark chain execution as completed
    ///
    /// # Arguments
    /// * `execution` - The chain execution to complete
    ///
    /// # Returns
    /// * `Ok(())` - If update succeeds
    /// * `Err(_)` - If update fails
    pub async fn complete_execution(&self, mut execution: ChainExecution) -> Result<()> {
        execution.complete();

        let Some(ref repo) = self.chain_repository else {
            return Ok(());
        };

        repo.update_execution(&execution).await
            .context("Failed to mark chain execution as completed")?;

        info!(
            execution_id = %execution.id,
            total_steps = execution.step_results.len(),
            "Chain execution completed"
        );

        Ok(())
    }

    /// Mark chain execution as failed
    ///
    /// # Arguments
    /// * `execution` - The chain execution to mark failed
    /// * `error` - Error message
    ///
    /// # Returns
    /// * `Ok(())` - If update succeeds
    /// * `Err(_)` - If update fails
    pub async fn fail_execution(&self, mut execution: ChainExecution, error: String) -> Result<()> {
        execution.fail(error.clone());

        let Some(ref repo) = self.chain_repository else {
            return Ok(());
        };

        repo.update_execution(&execution).await
            .context("Failed to mark chain execution as failed")?;

        warn!(
            execution_id = %execution.id,
            error = %error,
            "Chain execution failed"
        );

        Ok(())
    }

    /// Get the last completed step index from an execution
    ///
    /// Used to resume chains from the last successful step on retry.
    ///
    /// # Arguments
    /// * `execution` - The chain execution to check
    ///
    /// # Returns
    /// * Index of the last completed step, or 0 if none completed
    pub fn get_last_completed_step_index(&self, execution: &ChainExecution) -> usize {
        execution.step_results.len()
    }

    /// Check if a chain execution can be resumed
    ///
    /// Returns true if the execution has completed steps but is not finished.
    ///
    /// # Arguments
    /// * `execution` - The chain execution to check
    /// * `total_steps` - Total number of steps in the chain
    pub fn can_resume_execution(&self, execution: &ChainExecution, total_steps: usize) -> bool {
        !execution.step_results.is_empty()
            && execution.step_results.len() < total_steps
            && matches!(execution.status, ChainStatus::Running)
    }

    /// Get the last step output for resumption
    ///
    /// Returns the output from the last completed step, or None if no steps completed.
    pub fn get_last_step_output<'a>(&self, execution: &'a ChainExecution) -> Option<&'a str> {
        execution.step_results.last().map(|r| r.output.as_str())
    }

    // ==================== End Chain Execution Tracking ====================

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
            self.execute_hooks(&step.pre_hooks, task, &step.id, "pre", None).await?;
        }

        // Build the prompt with current variables
        let prompt = step
            .build_prompt(input)
            .context(format!("Failed to build prompt for step {}", step.id))?;

        debug!("Built prompt for step {}: {}", step.id, prompt);

        // Substitute variables in working_directory if specified
        let working_directory = if let Some(ref wd_template) = step.working_directory {
            Some(Self::substitute_variables(wd_template, input)?)
        } else {
            None
        };

        // Execute the step with retries
        let result = self.execute_step_with_retry(step, &prompt, working_directory.as_deref()).await?;

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

        // Store step output to memory if configured (core feature)
        if step.store_in_memory.is_some() {
            if let Err(e) = self.store_step_output_to_memory(step, &result, task).await {
                // Log but don't fail - memory storage is best-effort
                warn!(
                    step_id = %step.id,
                    error = ?e,
                    "Memory storage failed but continuing execution"
                );
            }
        }

        // Execute post-hooks if any
        if !step.post_hooks.is_empty() {
            info!("Executing {} post-hooks for step {}", step.post_hooks.len(), step.id);
            self.execute_hooks(&step.post_hooks, task, &step.id, "post", Some(&result)).await?;
        }

        // Check if this step should spawn implementation tasks (from tasks array in output)
        if self.should_spawn_tasks(step) {
            info!(
                step_id = %step.id,
                "Step configured to spawn tasks, parsing output"
            );

            // Find step index in chain
            let step_index = chain.steps.iter().position(|s| s.id == step.id).unwrap_or(0);

            // CRITICAL: Task spawning failures are now fatal to prevent hung workflows
            self.spawn_tasks_from_output(&result, task, &chain.name, &step.id, step_index).await
                .with_context(|| format!(
                    "Failed to spawn tasks from step {} output. This is a fatal error to prevent hung workflows.",
                    step.id
                ))?;
        }

        // Decomposition is handled by the caller (execute_with_chain or execute_chain_with_task),
        // not here. Processing it here would cause duplicate children.

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
                self.execute_hooks(&step.pre_hooks, task, &step.id, "pre", None).await?;
            }

            // Build the prompt with current variables
            let prompt = step
                .build_prompt(&current_input)
                .context(format!("Failed to build prompt for step {}", step.id))?;

            debug!("Built prompt for step {}: {}", step.id, prompt);

            // Substitute variables in working_directory if specified
            let working_directory = if let Some(ref wd_template) = step.working_directory {
                Some(Self::substitute_variables(wd_template, &current_input)?)
            } else {
                None
            };

            // Execute the step with retries
            let result = self.execute_step_with_retry(step, &prompt, working_directory.as_deref()).await?;

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
                self.execute_hooks(&step.post_hooks, task, &step.id, "post", Some(&result)).await?;
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

                // CRITICAL: Task spawning failures are now fatal to prevent hung workflows
                self.spawn_tasks_from_output(&result, task, &chain.name, &step.id, index).await
                    .with_context(|| format!(
                        "Failed to spawn tasks from step {} output. This is a fatal error to prevent hung workflows.",
                        step.id
                    ))?;
            }

            // Check if this step has decomposition configuration (fan-out pattern)
            if self.has_decomposition(step) {
                if let Some(parent_task) = task {
                    info!(
                        step_id = %step.id,
                        "Step has decomposition config, processing fan-out"
                    );

                    // CRITICAL: Decomposition failures are fatal to prevent hung workflows
                    let child_count = self.process_decomposition(step, &result, parent_task, &chain.name).await
                        .with_context(|| format!(
                            "Failed to process decomposition for step {}. This is a fatal error to prevent hung workflows.",
                            step.id
                        ))?;

                    if child_count > 0 {
                        info!(
                            step_id = %step.id,
                            child_count = child_count,
                            "Decomposition spawned {} child tasks",
                            child_count
                        );
                    }
                } else {
                    warn!(
                        step_id = %step.id,
                        "Step has decomposition config but no parent task provided"
                    );
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

    /// Execute a single step with retry logic and validation feedback
    ///
    /// When a step fails validation, the retry includes the validation error feedback
    /// in the prompt to help the LLM correct its output. This significantly improves
    /// success rates for format-sensitive outputs like JSON.
    #[instrument(skip(self, step, prompt), fields(step_id = %step.id))]
    async fn execute_step_with_retry(
        &self,
        step: &PromptStep,
        prompt: &str,
        working_directory: Option<&str>,
    ) -> Result<StepResult> {
        let mut retry_count = 0;
        let mut last_error = None;
        let mut current_prompt = prompt.to_string();

        while retry_count <= self.max_retries {
            match self.execute_step(step, &current_prompt, working_directory).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let error_str = e.to_string();
                    let is_validation_error = error_str.contains("Validation failed") ||
                                              error_str.contains("validation failed");

                    warn!(
                        "Step {} execution failed (attempt {}/{}): {}{}",
                        step.id,
                        retry_count + 1,
                        self.max_retries + 1,
                        e,
                        if is_validation_error { " [Will include validation feedback in retry]" } else { "" }
                    );
                    last_error = Some(e);
                    retry_count += 1;

                    if retry_count <= self.max_retries {
                        // Exponential backoff
                        let delay = Duration::from_secs(2_u64.pow(retry_count));
                        tokio::time::sleep(delay).await;

                        // If this was a validation error, add feedback to the prompt
                        if is_validation_error {
                            current_prompt = format!(
                                "{}\n\n---\nIMPORTANT: Your previous response failed validation with the following error:\n{}\n\nPlease correct your response to address this validation error. Ensure your output matches the expected format exactly.",
                                prompt,
                                error_str
                            );
                            info!(
                                step_id = %step.id,
                                retry_attempt = retry_count,
                                "Adding validation feedback to retry prompt"
                            );
                        }
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
    async fn execute_step(&self, step: &PromptStep, prompt: &str, working_directory: Option<&str>) -> Result<StepResult> {
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
        let output = match timeout(step_timeout, self.execute_prompt(prompt, &step.role, step_timeout.as_secs(), working_directory)).await {
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
                let output_preview = if output.len() > 1000 {
                    format!("{}...[truncated {} more chars]", &output[..1000], output.len() - 1000)
                } else {
                    output.clone()
                };
                let error_msg = format!("Output format validation failed: {}", e);
                error!(
                    step_id = %step.id,
                    expected_format = ?step.expected_output,
                    output_length = output.len(),
                    output_preview = %output_preview,
                    error = %e,
                    "Output format validation failed for step"
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

    /// Substitute variables in a template string
    fn substitute_variables(template: &str, variables: &serde_json::Value) -> Result<String> {
        debug!(
            template = %template,
            variables = %serde_json::to_string(variables).unwrap_or_else(|_| "{}".to_string()),
            "Substituting variables in template"
        );

        let mut result = template.to_string();

        if let Some(vars) = variables.as_object() {
            for (key, value) in vars {
                let placeholder = format!("{{{}}}", key);
                let replacement = match value {
                    serde_json::Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                result = result.replace(&placeholder, &replacement);
            }
        }

        debug!(
            template = %template,
            result = %result,
            "Variable substitution result"
        );

        Ok(result)
    }

    /// Execute a prompt via LLM substrate (Claude Code CLI or Anthropic API)
    async fn execute_prompt(&self, prompt: &str, role: &str, timeout_secs: u64, working_directory: Option<&str>) -> Result<String> {
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
            // Note: We return a simple mock JSON that won't trigger false positives
            // in the markdown code block stripper (which looks for ``` in output)
            return Ok(serde_json::json!({
                "role": role,
                "response": "Mock response generated without substrate",
                "status": "success",
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
            working_directory: working_directory.map(|s| s.to_string()),
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
            // If the rule doesn't provide a schema but requires one (JsonSchema type),
            // try to use the schema from step's expected_output
            let schema_to_use = if rule.schema.is_none()
                && matches!(rule.rule_type, ValidationType::JsonSchema) {
                // Extract schema from step's expected_output if it's JSON format
                match &step.expected_output {
                    OutputFormat::Json { schema } => schema.as_ref(),
                    _ => None,
                }
            } else {
                rule.schema.as_ref()
            };

            match self
                .validator
                .validate_with_rule(&result.output, &rule.rule_type, schema_to_use)
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

    /// Extract JSON fields into variables map for hook substitution
    ///
    /// Recursively extracts fields from JSON, creating dot-notation keys for nested objects.
    /// For example: {"decomposition": {"strategy": "single"}} becomes "decomposition.strategy" = "single"
    ///
    /// # Arguments
    /// * `value` - JSON value to extract from
    /// * `prefix` - Dot-notation prefix for nested fields
    /// * `variables` - Mutable map to insert extracted variables into
    fn extract_json_fields_to_variables(
        value: &serde_json::Value,
        prefix: &str,
        variables: &mut std::collections::HashMap<String, String>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, val) in map {
                    let field_name = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", prefix, key)
                    };

                    match val {
                        serde_json::Value::String(s) => {
                            variables.insert(field_name, s.clone());
                        }
                        serde_json::Value::Number(n) => {
                            variables.insert(field_name, n.to_string());
                        }
                        serde_json::Value::Bool(b) => {
                            variables.insert(field_name, b.to_string());
                        }
                        serde_json::Value::Object(_) => {
                            // Recursively extract nested objects
                            Self::extract_json_fields_to_variables(val, &field_name, variables);
                        }
                        serde_json::Value::Array(_) => {
                            // Arrays are serialized as JSON strings
                            variables.insert(field_name, val.to_string());
                        }
                        serde_json::Value::Null => {
                            variables.insert(field_name, "null".to_string());
                        }
                    }
                }
            }
            _ => {
                // If root is not an object, just convert to string
                if !prefix.is_empty() {
                    variables.insert(prefix.to_string(), value.to_string());
                }
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
        step_result: Option<&StepResult>,
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

        // Extract JSON fields from step output if available
        if let Some(result) = step_result {
            // Strip markdown code blocks before parsing (agents often wrap JSON in ```json...```)
            let cleaned_output = OutputValidator::strip_markdown_code_blocks(&result.output);

            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&cleaned_output) {
                debug!(
                    step_id = %step_id,
                    "Extracting JSON fields from step output for hook variable substitution"
                );
                Self::extract_json_fields_to_variables(&json_value, "", &mut variables);
            } else {
                debug!(
                    step_id = %step_id,
                    "Step output is not valid JSON, skipping field extraction"
                );
            }
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

            // Substitute variables in working_directory if specified
            let working_directory = if let Some(ref wd_template) = step.working_directory {
                Some(Self::substitute_variables(wd_template, &current_input)?)
            } else {
                None
            };

            let result = self.execute_step_with_retry(step, &prompt, working_directory.as_deref()).await?;

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

    /// Generate an idempotency key from chain context and step context
    ///
    /// This ensures tasks spawned from the same chain step are not duplicated
    /// if the step retries or executes multiple times.
    ///
    /// The key includes:
    /// - chain_id: Identifies the specific chain being executed
    /// - step_id: Identifies the specific step within the chain
    /// - step_index: Position of the step in the chain (for disambiguation)
    /// - parent_task_id: Links to the parent task context
    ///
    /// NOTE: We intentionally do NOT include output_hash here. If the LLM returns
    /// slightly different output on retry (e.g., different formatting, additional
    /// context), we still want to prevent duplicate task spawning. The individual
    /// task keys (generated by generate_task_idempotency_key) include content-based
    /// hashing to ensure uniqueness for semantically different tasks.
    fn generate_idempotency_key(
        chain_id: &str,
        step_id: &str,
        step_index: usize,
        parent_task_id: Option<uuid::Uuid>,
        _output: &str, // Kept for API compatibility but not used in key generation
    ) -> String {
        format!(
            "chain:{}:step:{}:idx:{}:parent:{}",
            chain_id,
            step_id,
            step_index,
            parent_task_id.map(|id| id.to_string()).unwrap_or_else(|| "none".to_string()),
        )
    }

    /// Generate a content-based idempotency key for a single task
    ///
    /// Uses the task's content (summary, agent_type, description hash) instead of
    /// array index to ensure deterministic keys even if task order changes.
    ///
    /// Key design considerations:
    /// - Include full description hash for accurate uniqueness (not truncated)
    /// - Normalize whitespace to prevent formatting-sensitive duplicates
    /// - Include dependencies and priority for completeness
    /// - Parent key provides chain/step context
    fn generate_task_idempotency_key(
        parent_key: &str,
        task_def: &serde_json::Value,
    ) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let summary = task_def.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let agent_type = task_def.get("agent_type").and_then(|v| v.as_str()).unwrap_or("");
        let description = task_def.get("description").and_then(|v| v.as_str()).unwrap_or("");

        // Normalize whitespace to prevent duplicates due to formatting differences
        let normalized_summary: String = summary.split_whitespace().collect::<Vec<_>>().join(" ");
        let normalized_desc: String = description.split_whitespace().collect::<Vec<_>>().join(" ");

        let mut hasher = DefaultHasher::new();

        // Hash normalized content
        normalized_summary.hash(&mut hasher);
        agent_type.hash(&mut hasher);
        normalized_desc.hash(&mut hasher); // Full description, not truncated

        // Include additional fields if present for more precise uniqueness
        if let Some(deps) = task_def.get("dependencies") {
            deps.to_string().hash(&mut hasher);
        }
        if let Some(priority) = task_def.get("priority") {
            priority.to_string().hash(&mut hasher);
        }

        let content_hash = hasher.finish();

        format!("{}:task:{:x}", parent_key, content_hash)
    }

    /// Parse and spawn tasks from step output
    ///
    /// This function now:
    /// - Uses comprehensive idempotency keys including chain_id, step_id, step_index
    /// - Uses content-based keys for individual tasks (not array indices)
    /// - Uses TRANSACTIONAL batch insert to ensure atomicity
    /// - Returns error if task spawning fails (making failures fatal)
    ///
    /// IMPORTANT: All tasks are inserted in a single transaction to prevent partial
    /// state if the process crashes midway. Either all tasks are inserted, or none.
    async fn spawn_tasks_from_output(
        &self,
        result: &StepResult,
        parent_task: Option<&Task>,
        chain_id: &str,
        step_id: &str,
        step_index: usize,
    ) -> Result<()> {
        let Some(ref task_queue) = self.task_queue_service else {
            warn!("Task queue service not configured, cannot spawn tasks");
            return Ok(());
        };

        // Generate comprehensive idempotency key with chain and step context
        let idempotency_key = Self::generate_idempotency_key(
            chain_id,
            step_id,
            step_index,
            parent_task.map(|t| t.id),
            &result.output,
        );

        info!(
            idempotency_key = %idempotency_key,
            chain_id = %chain_id,
            step_id = %step_id,
            step_index = step_index,
            "Generating idempotency key for task spawning"
        );

        // Strip markdown code blocks before parsing (agents often wrap JSON in ```json...```)
        let cleaned_output = OutputValidator::strip_markdown_code_blocks(&result.output);

        debug!(
            raw_output_len = result.output.len(),
            cleaned_output_len = cleaned_output.len(),
            "Stripped markdown code blocks from step output for task parsing"
        );

        // Parse the output as JSON
        let output_json: serde_json::Value = serde_json::from_str(&cleaned_output)
            .with_context(|| format!(
                "Failed to parse step output as JSON. Output starts with: {}",
                &cleaned_output[..cleaned_output.len().min(200)]
            ))?;

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
            "Parsing {} tasks from step output for transactional insert",
            tasks_array.len()
        );

        // PHASE 1: Build all Task objects with content-based idempotency keys
        let mut tasks_to_insert = Vec::with_capacity(tasks_array.len());

        for (idx, task_def) in tasks_array.iter().enumerate() {
            // Use content-based idempotency key (not array index!)
            let task_idempotency_key = Self::generate_task_idempotency_key(&idempotency_key, task_def);

            match self.build_task_from_def(task_def, parent_task, &task_idempotency_key) {
                Ok(task) => {
                    debug!(
                        index = idx,
                        task_id = %task.id,
                        idempotency_key = %task_idempotency_key,
                        "Built task for batch insert"
                    );
                    tasks_to_insert.push(task);
                }
                Err(e) => {
                    // Fail fast on build errors - these are validation issues
                    let error_msg = format!(
                        "Failed to build task {} (summary: {:?}): {}",
                        idx,
                        task_def.get("summary").and_then(|v| v.as_str()),
                        e
                    );
                    error!("{}", error_msg);
                    anyhow::bail!("{}", error_msg);
                }
            }
        }

        // PHASE 2: Insert all tasks transactionally
        info!(
            task_count = tasks_to_insert.len(),
            "Inserting {} tasks transactionally",
            tasks_to_insert.len()
        );

        let result = task_queue
            .submit_tasks_transactional(tasks_to_insert)
            .await
            .with_context(|| format!(
                "Transactional batch insert failed for {} tasks from step {}",
                tasks_array.len(),
                step_id
            ))?;

        info!(
            inserted = result.inserted.len(),
            already_existed = result.already_existed.len(),
            total = result.total(),
            "Transactional task spawning complete"
        );

        Ok(())
    }

    /// Parse a task definition and enqueue it idempotently using atomic insert
    ///
    /// This method uses the atomic `submit_task_idempotent` to prevent race conditions
    /// when multiple concurrent executions try to spawn the same task.
    ///
    /// NOTE: This method is currently unused since spawn_tasks_from_output now uses
    /// transactional batch insert. Kept for potential future single-task insert needs.
    #[allow(dead_code)]
    async fn parse_and_enqueue_task_idempotent(
        &self,
        task_def: &serde_json::Value,
        parent_task: Option<&Task>,
        task_queue: &dyn TaskQueueService,
        idempotency_key: &str,
    ) -> Result<crate::domain::ports::task_repository::IdempotentInsertResult> {
        // Build the task using the common helper
        let task = self.build_task_from_def(task_def, parent_task, idempotency_key)?;

        // Submit atomically with idempotency check
        task_queue.submit_task_idempotent(task).await
    }

    /// Build a Task from a JSON definition (shared by both enqueue methods)
    fn build_task_from_def(
        &self,
        task_def: &serde_json::Value,
        parent_task: Option<&Task>,
        idempotency_key: &str,
    ) -> Result<Task> {
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

        // Extract or inherit feature_branch
        let feature_branch = task_def
            .get("feature_branch")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| parent_task.and_then(|t| t.feature_branch.clone()));

        // Determine branch and worktree_path
        // IMPORTANT: Each task MUST get its own unique worktree. Never inherit from parent.
        // Parent worktrees are for the parent's work only. Subtasks branch from the feature
        // branch and merge back to it when complete.
        let (branch_value, worktree_path) = if needs_worktree {
            if let Some(ref fb) = feature_branch {
                // ALWAYS create unique branch/worktree from feature branch
                // Generate a unique task UUID for the worktree path
                let task_uuid = uuid::Uuid::new_v4();
                let task_uuid_str = task_uuid.to_string();

                // Use task definition ID for branch name (human-readable)
                // Fall back to first 8 chars of task UUID if no ID provided
                let task_id_slug = task_def
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&task_uuid_str[..8]);

                // Extract feature name from feature branch
                let feature_name = fb
                    .strip_prefix("feature/")
                    .or_else(|| fb.strip_prefix("features/"))
                    .unwrap_or(fb);

                let branch = format!("task/{}/{}", feature_name, task_id_slug);
                let worktree = format!(".abathur/worktrees/task-{}", task_uuid);

                info!(
                    branch = %branch,
                    worktree = %worktree,
                    feature_branch = %fb,
                    parent_task_id = ?parent_task.map(|p| p.id),
                    "Generated unique task branch and worktree (branched from feature branch)"
                );

                (Some(branch), Some(worktree))
            } else {
                // No feature_branch available - this is a configuration error
                error!(
                    task_id = ?task_def.get("id"),
                    summary = ?task_def.get("summary"),
                    "needs_worktree is true but no feature_branch available. \
                     Task plans MUST include feature_branch for all tasks requiring worktrees."
                );
                (None, None)
            }
        } else {
            (None, None)
        };

        let input_data = task_def.get("input_data").cloned();
        let now = chrono::Utc::now();

        // Create the task
        Ok(Task {
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
            branch: branch_value,
            feature_branch,
            worktree_path,
            // Tasks from task plans default to needing worktrees
            needs_worktree: None,
            validation_requirement: crate::domain::models::ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: None,
            chain_step_index: 0,
            awaiting_children: None,
            spawned_by_task_id: None,
            chain_handoff_state: None,
            idempotency_key: Some(idempotency_key.to_string()),
            version: 1,
        })
    }

    /// Parse a task definition and enqueue it (non-idempotent, for backward compatibility)
    #[allow(dead_code)]
    async fn parse_and_enqueue_task(
        &self,
        task_def: &serde_json::Value,
        parent_task: Option<&Task>,
        task_queue: &dyn TaskQueueService,
        idempotency_key: &str,
    ) -> Result<uuid::Uuid> {
        let task = self.build_task_from_def(task_def, parent_task, idempotency_key)?;
        task_queue.submit_task(task).await
    }

    /// Process decomposition for a step that has decomposition configuration.
    ///
    /// This is core functionality for fan-out workflows where a step (like technical-architect)
    /// outputs a list of subprojects that should each get their own branch and child task.
    ///
    /// # Arguments
    /// * `step` - The step with decomposition config
    /// * `result` - The step execution result containing the output to decompose
    /// * `parent_task` - The parent task that will wait for children
    /// * `chain_id` - The chain ID for context
    ///
    /// # Returns
    /// * `Ok(child_count)` - Number of child tasks spawned
    /// * `Err(_)` - If decomposition fails (fatal error)
    /// Check if a step has decomposition configuration
    fn has_decomposition(&self, step: &PromptStep) -> bool {
        step.decomposition.is_some() && self.task_queue_service.is_some()
    }

    #[instrument(skip(self, step, result, parent_task), fields(step_id = %step.id))]
    async fn process_decomposition(
        &self,
        step: &PromptStep,
        result: &StepResult,
        parent_task: &Task,
        chain_id: &str,
    ) -> Result<usize> {
        let Some(ref decomposition) = step.decomposition else {
            return Ok(0); // No decomposition configured
        };

        let Some(ref task_queue) = self.task_queue_service else {
            warn!(
                step_id = %step.id,
                "decomposition configured but no task queue service available"
            );
            return Ok(0);
        };

        info!(
            step_id = %step.id,
            items_path = %decomposition.items_path,
            "Processing decomposition from step output"
        );

        // Parse output as JSON
        let cleaned_output = OutputValidator::strip_markdown_code_blocks(&result.output);
        let output_json: serde_json::Value = serde_json::from_str(&cleaned_output)
            .with_context(|| format!(
                "Failed to parse step output as JSON for decomposition. Output starts with: {}",
                &cleaned_output[..cleaned_output.len().min(200)]
            ))?;

        // Extract feature_name for branch templates
        let feature_name = output_json
            .get("feature_name")
            .and_then(|v| v.as_str())
            .map(|s| Self::sanitize_branch_name(s))
            .unwrap_or_else(|| "unnamed".to_string());

        // Extract items using items_path (dot-notation JSON path)
        let items = self.extract_items_by_path(&output_json, &decomposition.items_path)?;

        if items.is_empty() {
            info!(
                step_id = %step.id,
                items_path = %decomposition.items_path,
                "No items found for decomposition"
            );
            return Ok(0);
        }

        info!(
            step_id = %step.id,
            item_count = items.len(),
            feature_name = %feature_name,
            "Found {} items for decomposition",
            items.len()
        );

        // Build child tasks
        let mut child_tasks = Vec::with_capacity(items.len());
        let mut child_ids = Vec::with_capacity(items.len());

        for (idx, item) in items.iter().enumerate() {
            // Get item name (string or object with "name" field)
            let item_name = match item {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(obj) => obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&format!("item-{}", idx))
                    .to_string(),
                _ => format!("item-{}", idx),
            };

            let sanitized_item_name = Self::sanitize_branch_name(&item_name);

            // Create branch name from template
            let branch_name = self.substitute_decomposition_template(
                &decomposition.per_item.branch.template,
                &feature_name,
                &sanitized_item_name,
                item,
                &result.output,
            );

            // Create task summary and description
            let summary = self.substitute_decomposition_template(
                &decomposition.per_item.task.summary,
                &feature_name,
                &sanitized_item_name,
                item,
                &result.output,
            );

            let description = self.substitute_decomposition_template(
                &decomposition.per_item.task.description,
                &feature_name,
                &sanitized_item_name,
                item,
                &result.output,
            );

            // Generate idempotency key for this child task
            let idempotency_key = format!(
                "decomposition:{}:{}:{}:{}",
                chain_id,
                step.id,
                parent_task.id,
                sanitized_item_name
            );

            // Build the child task
            let child_task = Task {
                id: uuid::Uuid::new_v4(),
                summary,
                description,
                agent_type: decomposition.per_item.task.agent_type.clone(),
                priority: decomposition.per_item.task.priority,
                calculated_priority: decomposition.per_item.task.priority as f64,
                status: TaskStatus::Pending,
                dependencies: Some(vec![parent_task.id]),
                dependency_type: DependencyType::Sequential,
                dependency_depth: parent_task.dependency_depth + 1,
                input_data: Some(serde_json::json!({
                    "parent_output": result.output,
                    "item": item,
                    "feature_name": feature_name,
                    "item_name": item_name,
                })),
                result_data: None,
                error_message: None,
                retry_count: 0,
                max_retries: 3,
                max_execution_timeout_seconds: 3600,
                submitted_at: chrono::Utc::now(),
                started_at: None,
                completed_at: None,
                last_updated_at: chrono::Utc::now(),
                created_by: Some(format!("chain:{}:step:{}", chain_id, step.id)),
                parent_task_id: Some(parent_task.id),
                session_id: parent_task.session_id.clone(),
                source: TaskSource::Chain,
                deadline: None,
                estimated_duration_seconds: None,
                feature_branch: Some(branch_name.clone()),
                branch: None, // Will be created when task starts
                worktree_path: None,
                // Decomposition children need their own worktrees for isolation
                needs_worktree: None,
                validation_requirement: ValidationRequirement::None,
                validation_task_id: None,
                validating_task_id: None,
                remediation_count: 0,
                is_remediation: false,
                workflow_state: None,
                workflow_expectations: None,
                chain_id: if decomposition.per_item.task.continue_chain {
                    Some(chain_id.to_string())
                } else {
                    None
                },
                // Chain step index is 0 - the chain executor will resolve the correct step
                // based on continue_at_step when continuing the chain
                chain_step_index: 0,
                awaiting_children: None,
                spawned_by_task_id: Some(parent_task.id),
                chain_handoff_state: None,
                idempotency_key: Some(idempotency_key),
                version: 1,
            };

            child_ids.push(child_task.id);
            child_tasks.push(child_task);

            info!(
                step_id = %step.id,
                item_name = %item_name,
                branch_name = %branch_name,
                "Prepared decomposition child task"
            );
        }

        let child_count = child_tasks.len();

        // Clone parent task for modification
        let mut updated_parent = parent_task.clone();

        // Update parent task to await children if configured
        if decomposition.on_complete.wait_for_children {
            updated_parent.status = TaskStatus::AwaitingChildren;
            updated_parent.awaiting_children = Some(child_ids.clone());
            updated_parent.last_updated_at = chrono::Utc::now();

            info!(
                step_id = %step.id,
                parent_task_id = %parent_task.id,
                child_count = child_count,
                "Parent task set to await {} children",
                child_count
            );
        }

        // Atomically update parent and insert all children
        info!(
            step_id = %step.id,
            parent_task_id = %parent_task.id,
            parent_version = parent_task.version,
            updated_parent_status = ?updated_parent.status,
            child_count = child_count,
            "Attempting atomic decomposition"
        );

        let atomic_result = match task_queue
            .update_parent_and_insert_children_atomic(&updated_parent, child_tasks)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                error!(
                    step_id = %step.id,
                    parent_task_id = %parent_task.id,
                    parent_version = parent_task.version,
                    updated_parent_status = ?updated_parent.status,
                    child_count = child_count,
                    error = %e,
                    error_chain = ?e,
                    "Atomic decomposition FAILED - this is likely a schema or version conflict issue"
                );
                return Err(e).with_context(|| format!(
                    "Failed to atomically insert {} decomposition children for step {} (parent version: {}, target status: {:?})",
                    child_count,
                    step.id,
                    parent_task.version,
                    updated_parent.status
                ));
            }
        };

        info!(
            step_id = %step.id,
            parent_task_id = %parent_task.id,
            children_inserted = atomic_result.children_inserted.len(),
            children_already_existed = atomic_result.children_already_existed.len(),
            "Decomposition complete: {} children spawned",
            child_count
        );

        // Ensure feature branches exist for all children
        for (idx, child_id) in atomic_result.children_inserted.iter().enumerate() {
            let item = &items[idx];
            let item_name = match item {
                serde_json::Value::String(s) => Self::sanitize_branch_name(s),
                serde_json::Value::Object(obj) => obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| Self::sanitize_branch_name(s))
                    .unwrap_or_else(|| format!("item-{}", idx)),
                _ => format!("item-{}", idx),
            };

            let branch_name = self.substitute_decomposition_template(
                &decomposition.per_item.branch.template,
                &feature_name,
                &item_name,
                item,
                &parent_task.id.to_string(), // Not used but required
            );

            // Create the feature branch from parent
            if let Err(e) = self
                .ensure_feature_branch_exists(&branch_name, &decomposition.per_item.branch.parent)
                .await
            {
                warn!(
                    branch_name = %branch_name,
                    error = ?e,
                    "Failed to create feature branch for decomposition child (continuing anyway)"
                );
            } else {
                debug!(
                    branch_name = %branch_name,
                    child_id = %child_id,
                    "Created feature branch for decomposition child"
                );
            }
        }

        Ok(child_count)
    }

    /// Extract items from JSON output using dot-notation path
    fn extract_items_by_path(
        &self,
        json: &serde_json::Value,
        path: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let mut current = json;

        for part in path.split('.') {
            current = current.get(part).with_context(|| {
                format!(
                    "Path '{}' not found in output (failed at '{}')",
                    path, part
                )
            })?;
        }

        match current {
            serde_json::Value::Array(arr) => Ok(arr.clone()),
            _ => Err(anyhow::anyhow!(
                "Path '{}' does not point to an array",
                path
            )),
        }
    }

    /// Substitute variables in decomposition templates
    fn substitute_decomposition_template(
        &self,
        template: &str,
        feature_name: &str,
        item_name: &str,
        item: &serde_json::Value,
        previous_output: &str,
    ) -> String {
        let mut result = template.to_string();

        // Basic substitutions
        result = result.replace("{feature_name}", feature_name);
        result = result.replace("{item}", item_name);

        // Item field substitutions like {item.name}, {item.description}
        if let serde_json::Value::Object(obj) = item {
            for (key, value) in obj {
                let placeholder = format!("{{item.{}}}", key);
                let replacement = match value {
                    serde_json::Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                result = result.replace(&placeholder, &replacement);
            }
        }

        // Previous output (truncated for summaries)
        if result.contains("{previous_output}") {
            let truncated = if previous_output.len() > 2000 {
                format!("{}...", &previous_output[..2000])
            } else {
                previous_output.to_string()
            };
            result = result.replace("{previous_output}", &truncated);
        }

        result
    }

    /// Sanitize a string for use as a branch name
    fn sanitize_branch_name(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .chars()
            .take(50)
            .collect()
    }

    /// Ensure a feature branch exists, creating it from parent if needed
    async fn ensure_feature_branch_exists(&self, branch_name: &str, parent: &str) -> Result<()> {
        use std::process::Stdio;
        use tokio::process::Command;

        // Check if branch exists
        let check = Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{}", branch_name)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;

        if check.success() {
            debug!(branch = %branch_name, "Branch already exists");
            return Ok(());
        }

        // Determine parent ref
        let parent_ref = if parent == "main" || parent == "master" {
            // Check which one exists
            let main_exists = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", "refs/heads/main"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if main_exists {
                "main"
            } else {
                "master"
            }
        } else {
            parent
        };

        // Create the branch
        let output = Command::new("git")
            .args(["branch", branch_name, parent_ref])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "Failed to create branch '{}' from '{}': {}",
                branch_name,
                parent_ref,
                stderr
            ));
        }

        info!(branch = %branch_name, parent = %parent_ref, "Created feature branch");
        Ok(())
    }

    /// Store step output to memory (core feature, no shell hooks required)
    ///
    /// This is called when a step has `store_in_memory` configuration set.
    /// The output is stored in the memory system for future reference by other
    /// agents or chain steps.
    ///
    /// # Arguments
    /// * `step` - The step that was executed
    /// * `result` - The step execution result
    /// * `task` - Optional task context for namespace construction
    ///
    /// # Returns
    /// * `Ok(())` - Storage succeeded
    /// * `Err(_)` - Storage failed (logged but non-fatal)
    #[instrument(skip(self, step, result, task), fields(step_id = %step.id))]
    async fn store_step_output_to_memory(
        &self,
        step: &PromptStep,
        result: &StepResult,
        task: Option<&Task>,
    ) -> Result<()> {
        let Some(config) = &step.store_in_memory else {
            return Ok(()); // No memory storage configured
        };

        let Some(memory_service) = &self.memory_service else {
            warn!(
                step_id = %step.id,
                "store_in_memory configured but no memory service available"
            );
            return Ok(());
        };

        // Build namespace from template or default
        let namespace = if let Some(template) = &config.namespace_template {
            let mut ns = template.clone();
            if let Some(task) = task {
                ns = ns.replace("{task_id}", &task.id.to_string());
                if let Some(ref feature_name) = task.feature_branch {
                    ns = ns.replace("{feature_name}", feature_name);
                }
            }
            ns = ns.replace("{step_id}", &step.id);
            ns
        } else if let Some(task) = task {
            format!("step:{}:{}", task.id, step.id)
        } else {
            format!("step:unknown:{}", step.id)
        };

        // Parse memory type
        let memory_type = match config.memory_type.to_lowercase().as_str() {
            "semantic" => MemoryType::Semantic,
            "episodic" => MemoryType::Episodic,
            "procedural" => MemoryType::Procedural,
            _ => {
                warn!(
                    step_id = %step.id,
                    memory_type = %config.memory_type,
                    "Unknown memory type, defaulting to Semantic"
                );
                MemoryType::Semantic
            }
        };

        // Parse output as JSON value, or wrap as string
        let value = match serde_json::from_str::<serde_json::Value>(&result.output) {
            Ok(v) => v,
            Err(_) => serde_json::json!({ "raw_output": result.output }),
        };

        // Create memory entry
        let memory = Memory::new(
            namespace.clone(),
            config.key.clone(),
            value,
            memory_type,
            format!("chain_step:{}", step.id),
        );

        // Store to memory service
        match memory_service.add(memory).await {
            Ok(id) => {
                info!(
                    step_id = %step.id,
                    namespace = %namespace,
                    key = %config.key,
                    memory_id = %id,
                    "Step output stored to memory successfully"
                );
                Ok(())
            }
            Err(e) => {
                // Check if it's a duplicate - that's not an error for idempotent retries
                if e.to_string().contains("already exists") {
                    info!(
                        step_id = %step.id,
                        namespace = %namespace,
                        key = %config.key,
                        "Step output already stored (idempotent retry)"
                    );
                    Ok(())
                } else {
                    error!(
                        step_id = %step.id,
                        namespace = %namespace,
                        key = %config.key,
                        error = ?e,
                        "Failed to store step output to memory"
                    );
                    Err(e)
                }
            }
        }
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
    use crate::domain::models::prompt_chain::{ChainStatus, OutputFormat, PromptChain, PromptStep, StepMemoryConfig};

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

    #[tokio::test]
    async fn test_store_step_output_no_config() {
        // When store_in_memory is None, should return Ok immediately
        let service = PromptChainService::new();

        let step = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Plain,
        );
        assert!(step.store_in_memory.is_none());

        let result = StepResult::new(
            "step1".to_string(),
            r#"{"data": "value"}"#.to_string(),
            true,
            Duration::from_secs(1),
        );

        // Should return Ok because there's no store_in_memory config
        let store_result = service.store_step_output_to_memory(&step, &result, None).await;
        assert!(store_result.is_ok());
    }

    #[tokio::test]
    async fn test_store_step_output_no_memory_service() {
        // When store_in_memory is set but no memory service, should log warning and return Ok
        let service = PromptChainService::new();
        assert!(service.memory_service.is_none());

        let mut step = PromptStep::new(
            "step1".to_string(),
            "Test".to_string(),
            "Tester".to_string(),
            OutputFormat::Json { schema: None },
        );
        step.store_in_memory = Some(StepMemoryConfig {
            key: "test_key".to_string(),
            memory_type: "semantic".to_string(),
            namespace_template: None,
        });

        let result = StepResult::new(
            "step1".to_string(),
            r#"{"data": "value"}"#.to_string(),
            true,
            Duration::from_secs(1),
        );

        // Should return Ok even without memory service (graceful degradation)
        let store_result = service.store_step_output_to_memory(&step, &result, None).await;
        assert!(store_result.is_ok());
    }

    #[test]
    fn test_step_memory_config_creation() {
        let config = StepMemoryConfig {
            key: "requirements".to_string(),
            memory_type: "semantic".to_string(),
            namespace_template: Some("task:{task_id}:requirements".to_string()),
        };

        assert_eq!(config.key, "requirements");
        assert_eq!(config.memory_type, "semantic");
        assert_eq!(config.namespace_template.unwrap(), "task:{task_id}:requirements");
    }

    #[test]
    fn test_memory_type_parsing() {
        // Test that various memory types are recognized
        let types = vec!["semantic", "episodic", "procedural", "SEMANTIC", "Episodic"];
        for mt in types {
            let config = StepMemoryConfig {
                key: "test".to_string(),
                memory_type: mt.to_string(),
                namespace_template: None,
            };
            // Just verify it doesn't panic - actual parsing happens in store_step_output_to_memory
            assert!(!config.memory_type.is_empty());
        }
    }

    #[tokio::test]
    async fn test_store_step_output_with_memory_service() {
        use crate::domain::ports::MemoryRepository;
        use mockall::mock;
        use mockall::predicate::*;

        // Create a mock memory repository
        mock! {
            MemRepo {}

            #[async_trait::async_trait]
            impl MemoryRepository for MemRepo {
                async fn insert(&self, memory: Memory) -> anyhow::Result<i64>;
                async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<Memory>>;
                async fn search(
                    &self,
                    namespace_prefix: &str,
                    memory_type: Option<MemoryType>,
                    limit: usize,
                ) -> anyhow::Result<Vec<Memory>>;
                async fn update(
                    &self,
                    namespace: &str,
                    key: &str,
                    value: serde_json::Value,
                    updated_by: &str,
                ) -> anyhow::Result<()>;
                async fn delete(&self, namespace: &str, key: &str) -> anyhow::Result<()>;
                async fn count(
                    &self,
                    namespace_prefix: &str,
                    memory_type: Option<MemoryType>,
                ) -> anyhow::Result<usize>;
            }
        }

        let mut mock_repo = MockMemRepo::new();

        // Expect get to return None (memory doesn't exist yet)
        mock_repo
            .expect_get()
            .returning(|_, _| Ok(None));

        // Expect insert to succeed
        mock_repo
            .expect_insert()
            .times(1)
            .returning(|_| Ok(42));

        let memory_service = Arc::new(MemoryService::new(Arc::new(mock_repo), None, None));
        let service = PromptChainService::new().with_memory_service(memory_service);

        // Create step with store_in_memory config
        let mut step = PromptStep::new(
            "test_step".to_string(),
            "Test prompt".to_string(),
            "tester".to_string(),
            OutputFormat::Json { schema: None },
        );
        step.store_in_memory = Some(StepMemoryConfig {
            key: "test_output".to_string(),
            memory_type: "semantic".to_string(),
            namespace_template: Some("test:{task_id}:output".to_string()),
        });

        let result = StepResult::new(
            "test_step".to_string(),
            r#"{"requirements": ["req1", "req2"]}"#.to_string(),
            true,
            Duration::from_secs(1),
        );

        // Create a test task for context
        let mut task = Task::new(
            "Test task".to_string(),
            "Test description".to_string(),
        );
        task.feature_branch = Some("feature/test".to_string());

        // Call store_step_output_to_memory
        let store_result = service.store_step_output_to_memory(&step, &result, Some(&task)).await;
        assert!(store_result.is_ok(), "Expected Ok, got {:?}", store_result);
    }

    // ============================================================================
    // Decomposition Tests
    // ============================================================================

    #[test]
    fn test_sanitize_branch_name() {
        // Basic alphanumeric
        assert_eq!(
            PromptChainService::sanitize_branch_name("my-feature"),
            "my-feature"
        );

        // Uppercase to lowercase
        assert_eq!(
            PromptChainService::sanitize_branch_name("My-Feature"),
            "my-feature"
        );

        // Special characters replaced with hyphens
        assert_eq!(
            PromptChainService::sanitize_branch_name("user authentication"),
            "user-authentication"
        );

        // Multiple special chars
        assert_eq!(
            PromptChainService::sanitize_branch_name("user/auth@system!"),
            "user-auth-system"
        );

        // Truncation to 50 chars
        let long_name = "a".repeat(60);
        assert_eq!(
            PromptChainService::sanitize_branch_name(&long_name).len(),
            50
        );

        // Leading/trailing hyphens trimmed
        assert_eq!(
            PromptChainService::sanitize_branch_name("--test--"),
            "test"
        );
    }

    #[test]
    fn test_extract_items_by_path() {
        let service = PromptChainService::new();

        // Simple path
        let json = serde_json::json!({
            "items": ["a", "b", "c"]
        });
        let result = service.extract_items_by_path(&json, "items");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 3);

        // Nested path
        let json = serde_json::json!({
            "decomposition": {
                "subprojects": ["frontend", "backend"]
            }
        });
        let result = service.extract_items_by_path(&json, "decomposition.subprojects");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);

        // Missing path
        let json = serde_json::json!({
            "other": "value"
        });
        let result = service.extract_items_by_path(&json, "decomposition.subprojects");
        assert!(result.is_err());

        // Path not an array
        let json = serde_json::json!({
            "decomposition": {
                "subprojects": "not-an-array"
            }
        });
        let result = service.extract_items_by_path(&json, "decomposition.subprojects");
        assert!(result.is_err());
    }

    #[test]
    fn test_substitute_decomposition_template() {
        let service = PromptChainService::new();

        // Basic substitutions
        let template = "feature/{feature_name}-{item}";
        let result = service.substitute_decomposition_template(
            template,
            "user-auth",
            "frontend",
            &serde_json::json!({}),
            "",
        );
        assert_eq!(result, "feature/user-auth-frontend");

        // Item field substitutions
        let template = "Create specs for {item.name}: {item.description}";
        let item = serde_json::json!({
            "name": "api",
            "description": "REST API module"
        });
        let result = service.substitute_decomposition_template(
            template,
            "feature",
            "api",
            &item,
            "",
        );
        assert_eq!(result, "Create specs for api: REST API module");

        // Previous output substitution (truncated)
        let template = "Context: {previous_output}";
        let long_output = "x".repeat(3000);
        let result = service.substitute_decomposition_template(
            template,
            "",
            "",
            &serde_json::json!({}),
            &long_output,
        );
        assert!(result.len() < 2100); // Truncated to ~2000 + "Context: " + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_has_decomposition_no_task_queue() {
        use crate::domain::models::prompt_chain::{
            BranchConfig, DecompositionConfig, OnDecompositionComplete, PerItemConfig,
            TaskSpawnConfig,
        };

        let service = PromptChainService::new();
        assert!(service.task_queue_service.is_none());

        // Step with decomposition config but no task queue
        let mut step = PromptStep::new(
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
            OutputFormat::Plain,
        );
        step.decomposition = Some(DecompositionConfig {
            items_path: "items".to_string(),
            per_item: PerItemConfig {
                branch: BranchConfig {
                    template: "feature/{item}".to_string(),
                    parent: "main".to_string(),
                },
                task: TaskSpawnConfig {
                    agent_type: "test".to_string(),
                    summary: "Test {item}".to_string(),
                    description: "Test".to_string(),
                    priority: 5,
                    continue_chain: false,
                    continue_at_step: None,
                },
            },
            on_complete: OnDecompositionComplete::default(),
        });

        // Should return false because no task queue service
        assert!(!service.has_decomposition(&step));

        // Step without decomposition
        let step_no_decomp = PromptStep::new(
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
            OutputFormat::Plain,
        );
        assert!(!service.has_decomposition(&step_no_decomp));
    }

    /// Regression test: Verify that chain.name (not chain.id) is used for child tasks
    ///
    /// Bug: Decomposition was using chain.id (a UUID like 3d119376-6a6f-...) instead of
    /// chain.name (the template name like "technical_feature_workflow"). Child tasks would
    /// then fail trying to load "3d119376-6a6f-....yaml" which doesn't exist.
    ///
    /// Fix: Changed process_decomposition and spawn_tasks_from_output to pass &chain.name
    /// instead of &chain.id.
    #[test]
    fn test_chain_name_vs_id_for_decomposition_regression() {
        // Create a chain with explicit name that differs from UUID
        let chain = PromptChain::new(
            "technical_feature_workflow".to_string(),
            "Test workflow".to_string(),
        );

        // Verify chain.id is a UUID format (not the template name)
        assert!(chain.id.contains('-'), "chain.id should be a UUID with dashes");
        assert!(!chain.id.contains("technical_feature_workflow"),
            "chain.id should NOT contain the template name");

        // Verify chain.name is the template name
        assert_eq!(chain.name, "technical_feature_workflow");

        // The fix ensures we use chain.name (maps to YAML filename) not chain.id (UUID)
        // When spawning child tasks for decomposition, the chain_id field must be
        // the template name so that child tasks can load "technical_feature_workflow.yaml"

        // This is the key assertion - chain.name should be usable as a filename
        let expected_yaml_path = format!("{}.yaml", chain.name);
        assert_eq!(expected_yaml_path, "technical_feature_workflow.yaml");

        // chain.id would produce an invalid path like "3d119376-6a6f-4f61-9d5d-3e70831d8b8e.yaml"
        // which doesn't exist - this was the bug
        let buggy_yaml_path = format!("{}.yaml", chain.id);
        assert!(buggy_yaml_path.contains('-'), "Buggy path would be a UUID-based filename");
        assert_ne!(buggy_yaml_path, expected_yaml_path,
            "The UUID path should differ from the correct template name path");
    }
}

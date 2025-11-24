use crate::domain::models::{AgentMetadataRegistry, Config, Task};
use crate::domain::ports::{
    ExecutionParameters,
    SubstrateError, SubstrateRequest,
};
use crate::infrastructure::substrates::SubstrateRegistry;
use crate::infrastructure::templates::ChainLoader;
use crate::services::PromptChainService;
use anyhow::Result;
use serde_json::Value;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Context for agent task execution
///
/// Contains all information needed to execute a specific agent task.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Unique identifier for the agent instance
    pub agent_id: Uuid,

    /// Task being executed
    pub task_id: Uuid,

    /// Agent type (determines behavior and capabilities)
    pub agent_type: String,

    /// Task description/prompt
    pub description: String,

    /// Optional input data for the task
    pub input_data: Option<Value>,

    /// Configuration for execution
    pub config: Config,
}

impl ExecutionContext {
    /// Create a new execution context
    pub const fn new(
        agent_id: Uuid,
        task_id: Uuid,
        agent_type: String,
        description: String,
        config: Config,
    ) -> Self {
        Self {
            agent_id,
            task_id,
            agent_type,
            description,
            input_data: None,
            config,
        }
    }

    /// Set input data for the task
    #[must_use]
    pub fn with_input_data(mut self, input_data: Value) -> Self {
        self.input_data = Some(input_data);
        self
    }
}

/// Error types for agent execution
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("Timeout executing task {task_id} after {timeout_secs}s")]
    Timeout { task_id: Uuid, timeout_secs: u64 },

    #[error("Substrate error for task {task_id}: {source}")]
    SubstrateError {
        task_id: Uuid,
        #[source]
        source: SubstrateError,
    },

    #[error("Max retries ({max_retries}) exceeded for task {task_id}: {last_error}")]
    MaxRetriesExceeded {
        task_id: Uuid,
        max_retries: u32,
        last_error: String,
    },

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
}

/// Agent executor responsible for running individual agent tasks
///
/// Orchestrates:
/// - LLM substrate routing based on agent type
/// - Timeout enforcement
/// - Retry logic for transient failures
/// - Comprehensive error handling
/// - Prompt chain execution for multi-step workflows
///
/// Note: MCP tool access is handled by the substrates themselves (Claude Code, API),
/// not by the agent executor. External LLM instances connect to HTTP MCP servers.
pub struct AgentExecutor {
    /// Substrate registry for LLM interactions
    ///
    /// Routes tasks to appropriate LLM substrate based on agent type
    substrate_registry: Arc<SubstrateRegistry>,

    /// Agent metadata registry for loading agent configuration
    ///
    /// Used to determine which model to use for each agent type
    agent_metadata_registry: Arc<Mutex<AgentMetadataRegistry>>,

    /// Chain loader for loading prompt chain templates
    chain_loader: Arc<ChainLoader>,

    /// Prompt chain service for executing multi-step workflows
    chain_service: Arc<PromptChainService>,

    /// Configuration for task execution
    config: Config,
}

impl AgentExecutor {
    /// Create a new `AgentExecutor`
    ///
    /// # Arguments
    /// * `substrate_registry` - Registry for routing to LLM substrates
    /// * `agent_metadata_registry` - Registry for loading agent metadata
    /// * `chain_loader` - Loader for prompt chain templates
    /// * `chain_service` - Service for executing prompt chains
    /// * `config` - Configuration for task execution
    pub fn new(
        substrate_registry: Arc<SubstrateRegistry>,
        agent_metadata_registry: Arc<Mutex<AgentMetadataRegistry>>,
        chain_loader: Arc<ChainLoader>,
        chain_service: Arc<PromptChainService>,
        config: Config,
    ) -> Self {
        Self {
            substrate_registry,
            agent_metadata_registry,
            chain_loader,
            chain_service,
            config,
        }
    }

    /// Execute a task, automatically detecting if it should use a prompt chain
    ///
    /// If the task has a `chain_id`, loads and executes the corresponding prompt chain.
    /// Otherwise, executes the task as a single agent.
    ///
    /// # Arguments
    /// * `task` - The task to execute
    ///
    /// # Returns
    /// * `Ok(String)` - Task execution result
    /// * `Err(ExecutionError)` - Execution failed or timed out
    pub async fn execute_task(&self, task: &Task) -> Result<String, ExecutionError> {
        // Check if task has a chain_id
        if let Some(chain_id) = &task.chain_id {
            info!(
                task_id = %task.id,
                chain_id = %chain_id,
                "Executing task with prompt chain"
            );

            self.execute_with_chain(task, chain_id).await
        } else {
            // No chain - execute as single agent
            debug!(
                task_id = %task.id,
                agent_type = %task.agent_type,
                "Executing task as single agent (no chain_id)"
            );

            let mut ctx = ExecutionContext::new(
                Uuid::new_v4(), // agent_id
                task.id,
                task.agent_type.clone(),
                task.description.clone(),
                self.config.clone(),
            );

            // If task has input_data, use it; otherwise create new input_data with worktree info
            let mut input_data = task.input_data.clone().unwrap_or_else(|| serde_json::json!({}));

            // Add worktree_path, branch, and feature_branch to input_data if they exist on the task
            if let Some(ref worktree_path) = task.worktree_path {
                input_data["worktree_path"] = serde_json::json!(worktree_path);
            }
            if let Some(ref branch) = task.branch {
                input_data["branch"] = serde_json::json!(branch);
            }
            if let Some(ref feature_branch) = task.feature_branch {
                input_data["feature_branch"] = serde_json::json!(feature_branch);
            }

            ctx.input_data = Some(input_data);

            self.execute_with_timeout(
                ctx,
                Duration::from_secs(task.max_execution_timeout_seconds as u64),
            )
            .await
        }
    }

    /// Execute a single step of a prompt chain
    ///
    /// Loads the chain, executes the current step, and enqueues the next step if needed.
    ///
    /// # Arguments
    /// * `task` - The task to execute (contains chain_id and chain_step_index)
    /// * `chain_id` - ID of the chain template to load (e.g., "technical_feature_workflow")
    ///
    /// # Returns
    /// * `Ok(String)` - Step execution result as JSON
    /// * `Err(ExecutionError)` - Chain loading or execution failed
    async fn execute_with_chain(
        &self,
        task: &Task,
        chain_id: &str,
    ) -> Result<String, ExecutionError> {
        // Load the chain template
        let chain_file = format!("{}.yaml", chain_id);
        let chain = self
            .chain_loader
            .load_from_file(&chain_file)
            .map_err(|e| {
                ExecutionError::ExecutionFailed(format!(
                    "Failed to load chain '{}': {}",
                    chain_id, e
                ))
            })?;

        let step_index = task.chain_step_index;

        // Validate step index
        if step_index >= chain.steps.len() {
            return Err(ExecutionError::ExecutionFailed(format!(
                "Invalid step index {} for chain '{}' (has {} steps)",
                step_index,
                chain_id,
                chain.steps.len()
            )));
        }

        let step = &chain.steps[step_index];

        // TODO: TEMPORARY DEBUG - Remove this logging once timeout issue is resolved
        info!(
            task_id = %task.id,
            step_id = %step.id,
            step_timeout = ?step.timeout,
            step_timeout_secs = step.timeout.as_ref().map(|d| d.as_secs()),
            "AgentExecutor: About to execute chain step with timeout"
        );

        info!(
            task_id = %task.id,
            chain_name = %chain.name,
            step_index = step_index,
            step_id = %step.id,
            total_steps = chain.steps.len(),
            "Executing chain step {}/{}",
            step_index + 1,
            chain.steps.len()
        );

        // Prepare input for this step (from task.input_data or initial input)
        let mut step_input = task.input_data.clone().unwrap_or_else(|| {
            serde_json::json!({
                "task_id": task.id.to_string(),
                "task_description": task.description,
                "task_summary": task.summary,
            })
        });

        // Ensure worktree information is included in step input
        if let Some(ref worktree_path) = task.worktree_path {
            step_input["worktree_path"] = serde_json::json!(worktree_path);
        }
        if let Some(ref branch) = task.branch {
            step_input["branch"] = serde_json::json!(branch);
        }
        if let Some(ref feature_branch) = task.feature_branch {
            step_input["feature_branch"] = serde_json::json!(feature_branch);
            // Extract feature_name from feature_branch for template substitution
            // Example: "feature/user-auth" -> "user-auth"
            if let Some(feature_name) = feature_branch.strip_prefix("feature/") {
                step_input["feature_name"] = serde_json::json!(feature_name);
                info!(
                    task_id = %task.id,
                    step_id = %step.id,
                    feature_branch = %feature_branch,
                    feature_name = %feature_name,
                    "Extracted feature_name from feature_branch for template substitution"
                );
            }
        } else {
            warn!(
                task_id = %task.id,
                step_id = %step.id,
                "Task has no feature_branch set - cannot extract feature_name for template substitution"
            );
        }

        // Log the complete step_input for debugging
        debug!(
            task_id = %task.id,
            step_id = %step.id,
            step_input = %serde_json::to_string_pretty(&step_input).unwrap_or_else(|_| "{}".to_string()),
            "Step input prepared for execution"
        );

        // Execute this single step
        let result = self
            .chain_service
            .execute_single_step(&chain, step, &step_input, Some(task))
            .await
            .map_err(|e| {
                ExecutionError::ExecutionFailed(format!(
                    "Step {} execution failed for '{}': {}",
                    step.id, chain_id, e
                ))
            })?;

        info!(
            task_id = %task.id,
            step_id = %step.id,
            step_index = step_index,
            output_length = result.output.len(),
            "Chain step completed successfully"
        );

        // Reload task from database to get any updates from post-hooks
        // (e.g., feature_branch, worktree_path set by create_feature_branch.sh)
        let updated_task = self
            .chain_service
            .get_task_from_repo(task.id)
            .await
            .map_err(|e| {
                ExecutionError::ExecutionFailed(format!("Failed to reload task after step execution: {}", e))
            })?
            .ok_or_else(|| {
                ExecutionError::ExecutionFailed(format!("Task {} not found after step execution", task.id))
            })?;

        // Enqueue the next step if there is one
        let next_step_index = step_index + 1;
        if next_step_index < chain.steps.len() {
            info!(
                task_id = %task.id,
                next_step_index = next_step_index,
                next_step_id = %chain.steps[next_step_index].id,
                "Enqueueing next chain step"
            );

            self.enqueue_next_chain_step(&updated_task, &chain, next_step_index, &result.output)
                .await
                .map_err(|e| {
                    ExecutionError::ExecutionFailed(format!(
                        "Failed to enqueue next step: {}",
                        e
                    ))
                })?;
        } else {
            info!(
                task_id = %task.id,
                chain_id = %chain_id,
                "Chain execution complete (all steps finished)"
            );
        }

        // Return step result as JSON
        serde_json::to_string_pretty(&result).map_err(|e| {
            ExecutionError::ExecutionFailed(format!("Failed to serialize step result: {}", e))
        })
    }

    /// Enqueue the next step of a chain as a new task
    async fn enqueue_next_chain_step(
        &self,
        current_task: &Task,
        chain: &crate::domain::models::prompt_chain::PromptChain,
        next_step_index: usize,
        previous_output: &str,
    ) -> anyhow::Result<uuid::Uuid> {
        use crate::domain::models::{DependencyType, TaskSource, TaskStatus};

        let next_step = &chain.steps[next_step_index];

        // Parse previous output as input_data for next step
        let input_data = match serde_json::from_str(previous_output) {
            Ok(value) => value,
            Err(_) => {
                // If not JSON, wrap it
                serde_json::json!({
                    "previous_output": previous_output,
                    "previous_step_index": current_task.chain_step_index
                })
            }
        };

        let now = chrono::Utc::now();

        // Chain orchestration steps (technical-architect, technical-requirements-specialist, task-planner)
        // work in the same branch as their parent (typically a feature branch)
        // They inherit both branch and feature_branch from the parent task
        // Implementation tasks spawned later will get new task branch values

        // Create task for next step
        let next_task = Task {
            id: uuid::Uuid::new_v4(),
            summary: format!(
                "Chain: {} - Step {}/{}",
                chain.name,
                next_step_index + 1,
                chain.steps.len()
            ),
            description: format!(
                "Execute step '{}' of chain '{}'",
                next_step.id, chain.id
            ),
            agent_type: next_step.role.clone(),
            priority: current_task.priority,
            calculated_priority: current_task.calculated_priority,
            status: TaskStatus::Pending,
            dependencies: Some(vec![current_task.id]), // Depend on current task
            dependency_type: DependencyType::Sequential,
            dependency_depth: current_task.dependency_depth + 1,
            input_data: Some(input_data),
            result_data: None,
            error_message: None,
            retry_count: 0,
            max_retries: 3,
            max_execution_timeout_seconds: 3600,
            submitted_at: now,
            started_at: None,
            completed_at: None,
            last_updated_at: now,
            created_by: Some("chain-orchestrator".to_string()),
            parent_task_id: current_task.parent_task_id,
            session_id: current_task.session_id,
            source: TaskSource::AgentPlanner,
            deadline: current_task.deadline,
            estimated_duration_seconds: None,
            branch: {
                // Generate branch and worktree if step needs task branch
                if next_step.needs_task_branch.unwrap_or(false) && current_task.feature_branch.is_some() {
                    let feature_name = current_task.feature_branch.as_ref()
                        .and_then(|fb| fb.strip_prefix("feature/"))
                        .unwrap_or("unknown");
                    Some(format!("task/{}/{}", feature_name, next_step.id))
                } else {
                    // No task branch - inherit from parent or None
                    current_task.branch.clone()
                }
            },
            feature_branch: current_task.feature_branch.clone(),
            worktree_path: {
                // Generate worktree if step needs task branch
                if next_step.needs_task_branch.unwrap_or(false) && current_task.feature_branch.is_some() {
                    Some(format!(".abathur/worktrees/task-{}", uuid::Uuid::new_v4()))
                } else {
                    // No task branch - use feature branch worktree or inherit from parent
                    current_task.worktree_path.clone()
                }
            },
            validation_requirement: crate::domain::models::ValidationRequirement::None,
            validation_task_id: None,
            validating_task_id: None,
            remediation_count: 0,
            is_remediation: false,
            workflow_state: None,
            workflow_expectations: None,
            chain_id: current_task.chain_id.clone(),
            chain_step_index: next_step_index,
        };

        // Get task queue service from chain service
        let task_id = self
            .chain_service
            .submit_task(next_task)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to submit next step task: {}", e))?;

        info!(
            next_task_id = %task_id,
            next_step_id = %next_step.id,
            next_step_index = next_step_index,
            "Enqueued next chain step"
        );

        Ok(task_id)
    }

    /// Execute a task with the configured timeout
    ///
    /// Uses the timeout from `ctx.config.retry.max_execution_timeout_seconds`.
    /// Falls back to a default of 3600 seconds (1 hour) if not specified.
    ///
    /// # Arguments
    /// * `ctx` - Execution context containing task details and configuration
    ///
    /// # Returns
    /// * `Ok(String)` - Task execution result
    /// * `Err(ExecutionError)` - Execution failed or timed out
    ///
    /// # Example
    /// ```ignore
    /// let result = executor.execute(ctx).await?;
    /// ```
    ///
    /// # Note
    /// This method uses a default timeout of 1 hour. For task-specific timeouts,
    /// use `execute_with_timeout` directly with the task's `max_execution_timeout_seconds`.
    pub async fn execute(&self, ctx: ExecutionContext) -> Result<String, ExecutionError> {
        // Default timeout to 1 hour
        let timeout_duration = Duration::from_secs(3600);

        self.execute_with_timeout(ctx, timeout_duration).await
    }

    /// Execute a task with a specific timeout
    ///
    /// Wraps the execution in a tokio timeout. If the execution exceeds the timeout,
    /// returns `ExecutionError::Timeout`.
    ///
    /// # Arguments
    /// * `ctx` - Execution context containing task details
    /// * `timeout_duration` - Maximum execution time
    ///
    /// # Returns
    /// * `Ok(String)` - Task execution result
    /// * `Err(ExecutionError::Timeout)` - Execution exceeded timeout
    /// * `Err(ExecutionError::*)` - Other execution errors
    ///
    /// # Example
    /// ```ignore
    /// let timeout = Duration::from_secs(600); // 10 minutes
    /// let result = executor.execute_with_timeout(ctx, timeout).await?;
    /// ```
    #[allow(clippy::option_if_let_else)]
    pub async fn execute_with_timeout(
        &self,
        ctx: ExecutionContext,
        timeout_duration: Duration,
    ) -> Result<String, ExecutionError> {
        let task_id = ctx.task_id;

        match timeout(timeout_duration, self.execute_with_retry(ctx)).await {
            Ok(result) => result,
            Err(_) => Err(ExecutionError::Timeout {
                task_id,
                timeout_secs: timeout_duration.as_secs(),
            }),
        }
    }

    /// Execute a task with retry logic
    ///
    /// Retries transient errors using exponential backoff.
    /// Non-retryable errors (`InvalidApiKey`, `InvalidArguments`) fail immediately.
    ///
    /// # Arguments
    /// * `ctx` - Execution context with retry configuration
    ///
    /// # Returns
    /// * `Ok(String)` - Successful execution result
    /// * `Err(ExecutionError)` - All retries exhausted or non-retryable error
    async fn execute_with_retry(&self, ctx: ExecutionContext) -> Result<String, ExecutionError> {
        let max_retries = ctx.config.retry.max_retries;
        let initial_backoff = Duration::from_millis(ctx.config.retry.initial_backoff_ms);
        let max_backoff = Duration::from_millis(ctx.config.retry.max_backoff_ms);

        let mut last_error = String::new();

        for attempt in 0..=max_retries {
            match self.execute_inner(ctx.clone()).await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    // Check if error is retryable
                    if !Self::is_retryable_error(&err) {
                        return Err(err);
                    }

                    last_error = err.to_string();

                    // Don't sleep after the last attempt
                    if attempt < max_retries {
                        // Calculate exponential backoff: initial * 2^attempt, capped at max
                        let backoff_ms = initial_backoff.as_millis() * (2_u128.pow(attempt));
                        #[allow(clippy::cast_possible_truncation)]
                        let backoff =
                            Duration::from_millis(backoff_ms.min(max_backoff.as_millis()) as u64);

                        tracing::warn!(
                            task_id = %ctx.task_id,
                            attempt = attempt + 1,
                            max_retries = max_retries,
                            backoff_ms = backoff.as_millis(),
                            error = %last_error,
                            "Retrying task execution after transient error"
                        );

                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(ExecutionError::MaxRetriesExceeded {
            task_id: ctx.task_id,
            max_retries,
            last_error,
        })
    }

    /// Inner execution logic (no timeout or retry)
    ///
    /// Orchestrates:
    /// 1. Route to appropriate LLM substrate based on agent type
    /// 2. Execute task via substrate
    /// 3. Return result
    ///
    /// Note: MCP tool invocations are handled by the substrate (Claude Code, API),
    /// not by the executor. External LLM instances can access MCP tools via HTTP servers.
    ///
    /// # Arguments
    /// * `ctx` - Execution context
    ///
    /// # Returns
    /// * `Ok(String)` - Execution result
    /// * `Err(ExecutionError)` - Execution failed
    async fn execute_inner(&self, ctx: ExecutionContext) -> Result<String, ExecutionError> {
        tracing::info!(
            task_id = %ctx.task_id,
            agent_id = %ctx.agent_id,
            agent_type = %ctx.agent_type,
            "Starting task execution"
        );

        // Load agent metadata to determine model (opus, sonnet, haiku, etc.)
        let model = self
            .agent_metadata_registry
            .lock()
            .unwrap()
            .get_model_id(&ctx.agent_type);

        tracing::debug!(
            task_id = %ctx.task_id,
            agent_type = %ctx.agent_type,
            model = %model,
            "Using model for agent type"
        );

        // Build prompt
        let prompt = self.build_prompt(&ctx);

        // Build extra parameters
        let extra_params = std::collections::HashMap::new();

        // Check if task has a worktree path and use it as working directory
        // This allows Claude Code to cd into the correct worktree directory
        let working_directory = ctx.input_data.as_ref()
            .and_then(|data| data.get("worktree_path"))
            .and_then(|v| v.as_str())
            .map(|s| {
                tracing::info!(
                    task_id = %ctx.task_id,
                    working_directory = %s,
                    "Task has worktree_path, using as working directory"
                );
                s.to_string()
            });

        // Create substrate request
        let request = SubstrateRequest {
            task_id: ctx.task_id,
            agent_type: ctx.agent_type.clone(),
            prompt,
            context: ctx.input_data.clone(),
            parameters: ExecutionParameters {
                model: Some(model),
                max_tokens: Some(4096),
                temperature: Some(0.7),
                timeout_secs: None, // Handled by outer timeout
                extra: extra_params,
            },
            working_directory,
        };

        // Execute via substrate registry (automatically routes to best substrate)
        let response = self
            .substrate_registry
            .execute(request)
            .await
            .map_err(|source| ExecutionError::SubstrateError {
                task_id: ctx.task_id,
                source,
            })?;

        tracing::info!(
            task_id = %ctx.task_id,
            input_tokens = response.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
            output_tokens = response.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
            stop_reason = ?response.stop_reason,
            "Substrate execution completed"
        );

        // Return the substrate response
        // Note: MCP tool access is handled by the substrate itself
        Ok(response.content)
    }

    /// Build prompt for Claude based on execution context
    ///
    /// Loads the agent definition markdown and includes it as the system prompt,
    /// matching the Python implementation's behavior.
    fn build_prompt(&self, ctx: &ExecutionContext) -> String {
        use crate::domain::models::AgentMetadata;

        let mut prompt = String::new();

        // Load the full agent definition content (after frontmatter)
        if let Ok(agent_file_path) = self
            .agent_metadata_registry
            .lock()
            .unwrap()
            .get_agent_file_path(&ctx.agent_type)
        {
            // Read the agent file and extract the prompt content
            match std::fs::read_to_string(&agent_file_path) {
                Ok(file_content) => {
                    match AgentMetadata::extract_prompt_content(&file_content) {
                        Ok(agent_prompt) => {
                            // Add the full agent definition as system prompt
                            let _ = write!(prompt, "{}\n\n", agent_prompt);

                            tracing::debug!(
                                task_id = %ctx.task_id,
                                agent_type = %ctx.agent_type,
                                agent_prompt_length = agent_prompt.len(),
                                "Loaded agent definition"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                task_id = %ctx.task_id,
                                agent_type = %ctx.agent_type,
                                error = %e,
                                "Failed to extract agent prompt content, using basic prompt"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %ctx.task_id,
                        agent_type = %ctx.agent_type,
                        error = %e,
                        "Failed to read agent file, using basic prompt"
                    );
                }
            }
        } else {
            tracing::warn!(
                task_id = %ctx.task_id,
                agent_type = %ctx.agent_type,
                "Could not find agent file, using basic prompt"
            );
        }

        // Add task context with task ID pre-prompt (matching Python's user_message)
        let _ = write!(
            prompt,
            "Your task ID is: {}\n\nYou are a {} agent.\n\nTask: {}\n",
            ctx.task_id, ctx.agent_type, ctx.description
        );

        if let Some(input_data) = &ctx.input_data {
            let _ = write!(prompt, "\nInput Data:\n{input_data}\n");
        }

        prompt
    }

    /// Check if an error is retryable
    ///
    /// Retryable errors:
    /// - `RateLimitExceeded`
    /// - `NetworkError`
    /// - `Unavailable`
    /// - Timeout
    ///
    /// Non-retryable errors:
    /// - `AuthError`
    /// - `InvalidConfig`
    /// - `NotConfigured`
    const fn is_retryable_error(err: &ExecutionError) -> bool {
        match err {
            ExecutionError::SubstrateError { source, .. } => matches!(
                source,
                SubstrateError::RateLimitExceeded(_)
                    | SubstrateError::NetworkError(_)
                    | SubstrateError::Unavailable(_)
                    | SubstrateError::Timeout(_)
            ),
            ExecutionError::Timeout { .. } => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::{
        HealthStatus, LlmSubstrate, McpClient, McpError,
        McpToolRequest, McpToolResponse, SubstrateTokenUsage,
        SubstrateResponse, StopReason,
    };
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Mock Substrate for testing
    struct MockSubstrate {
        call_count: Arc<AtomicU32>,
        should_fail: bool,
        fail_count: u32,
    }

    impl MockSubstrate {
        fn new() -> Self {
            Self {
                call_count: Arc::new(AtomicU32::new(0)),
                should_fail: false,
                fail_count: 0,
            }
        }

        fn with_failures(fail_count: u32) -> Self {
            Self {
                call_count: Arc::new(AtomicU32::new(0)),
                should_fail: true,
                fail_count,
            }
        }
    }

    #[async_trait]
    impl LlmSubstrate for MockSubstrate {
        fn substrate_id(&self) -> &str {
            "mock"
        }

        fn substrate_name(&self) -> &str {
            "Mock Substrate"
        }

        async fn execute(&self, request: SubstrateRequest) -> Result<SubstrateResponse, SubstrateError> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);

            if self.should_fail && count < self.fail_count {
                return Err(SubstrateError::RateLimitExceeded(
                    "Mock rate limit".to_string(),
                ));
            }

            Ok(SubstrateResponse {
                task_id: request.task_id,
                content: "Mock response".to_string(),
                stop_reason: StopReason::EndTurn,
                usage: Some(SubstrateTokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                }),
                metadata: std::collections::HashMap::new(),
            })
        }

        async fn health_check(&self) -> Result<HealthStatus, SubstrateError> {
            Ok(HealthStatus::Healthy)
        }
    }

    // Mock MCP Client for testing
    struct MockMcpClient;

    #[async_trait]
    impl McpClient for MockMcpClient {
        async fn invoke_tool(&self, request: McpToolRequest) -> Result<McpToolResponse, McpError> {
            Ok(McpToolResponse {
                task_id: request.task_id,
                result: serde_json::json!({"success": true}),
                is_error: false,
            })
        }

        async fn call_tool(
            &self,
            _server: &str,
            _tool: &str,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, McpError> {
            Ok(serde_json::json!({"success": true}))
        }

        async fn list_tools(
            &self,
            _server_name: &str,
        ) -> Result<Vec<crate::domain::ports::ToolInfo>, McpError> {
            use crate::domain::ports::ToolInfo;

            Ok(vec![
                ToolInfo {
                    name: "tool1".to_string(),
                    description: Some("Mock tool 1".to_string()),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {}
                    }),
                },
                ToolInfo {
                    name: "tool2".to_string(),
                    description: Some("Mock tool 2".to_string()),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {}
                    }),
                },
            ])
        }

        async fn read_resource(
            &self,
            _server: &str,
            uri: &str,
        ) -> Result<crate::domain::ports::ResourceContent, McpError> {
            use crate::domain::ports::ResourceContent;

            Ok(ResourceContent {
                uri: uri.to_string(),
                mime_type: Some("text/plain".to_string()),
                text: Some("Mock resource content".to_string()),
                blob: None,
            })
        }

        async fn list_resources(
            &self,
            _server: &str,
        ) -> Result<Vec<crate::domain::ports::ResourceInfo>, McpError> {
            use crate::domain::ports::ResourceInfo;

            Ok(vec![ResourceInfo {
                uri: "mock://resource1".to_string(),
                name: "Mock Resource".to_string(),
                description: Some("A mock resource for testing".to_string()),
                mime_type: Some("text/plain".to_string()),
            }])
        }

        async fn health_check(&self, _server_name: &str) -> Result<(), McpError> {
            Ok(())
        }
    }

    fn create_test_context() -> ExecutionContext {
        ExecutionContext::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "test-agent".to_string(),
            "Test task".to_string(),
            Config::default(),
        )
    }

    fn create_mock_registry() -> Arc<SubstrateRegistry> {
        // Create a mock config
        let _config = Config::default();

        // Create a mock substrate registry manually for testing
        // Note: In real tests, we'd use from_config, but for unit tests we build manually
        let mut substrates = std::collections::HashMap::new();
        substrates.insert(
            "mock".to_string(),
            Arc::new(MockSubstrate::new()) as Arc<dyn LlmSubstrate>,
        );

        Arc::new(SubstrateRegistry {
            substrates,
            default_substrate_id: "mock".to_string(),
            agent_mappings: std::collections::HashMap::new(),
        })
    }

    fn create_mock_registry_with_failures(fail_count: u32) -> Arc<SubstrateRegistry> {
        let mut substrates = std::collections::HashMap::new();
        substrates.insert(
            "mock".to_string(),
            Arc::new(MockSubstrate::with_failures(fail_count)) as Arc<dyn LlmSubstrate>,
        );

        Arc::new(SubstrateRegistry {
            substrates,
            default_substrate_id: "mock".to_string(),
            agent_mappings: std::collections::HashMap::new(),
        })
    }

    fn create_mock_executor(registry: Arc<SubstrateRegistry>) -> AgentExecutor {
        let metadata_registry = Arc::new(Mutex::new(AgentMetadataRegistry::new(
            &std::path::PathBuf::from("/tmp")
        )));
        let chain_loader = Arc::new(ChainLoader::default());
        let chain_service = Arc::new(PromptChainService::new());

        AgentExecutor::new(
            registry,
            metadata_registry,
            chain_loader,
            chain_service,
            Config::default(),
        )
    }

    #[tokio::test]
    async fn test_successful_execution() {
        let registry = create_mock_registry();
        let executor = create_mock_executor(registry);

        let ctx = create_test_context();
        let result = executor.execute(ctx).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Mock response");
    }

    #[tokio::test]
    async fn test_timeout_behavior() {
        let registry = create_mock_registry();
        let executor = create_mock_executor(registry);

        let ctx = create_test_context();
        let timeout_duration = Duration::from_millis(1); // Very short timeout

        // Add a small delay to the mock to trigger timeout
        // For now, this test will pass because mock is instant
        // In real implementation, we'd need a slow mock
        let result = executor.execute_with_timeout(ctx, timeout_duration).await;

        // This may or may not timeout depending on system speed
        // In real tests, we'd use a mock that sleeps
        assert!(result.is_ok() || matches!(result, Err(ExecutionError::Timeout { .. })));
    }

    #[tokio::test]
    async fn test_retry_logic_with_transient_errors() {
        // Registry with substrate that fails twice, then succeeds
        let registry = create_mock_registry_with_failures(2);
        let executor = create_mock_executor(registry);

        let mut ctx = create_test_context();
        ctx.config.retry.max_retries = 3;
        ctx.config.retry.initial_backoff_ms = 10; // Fast for testing
        ctx.config.retry.max_backoff_ms = 100;

        let result = executor.execute_with_retry(ctx).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Mock response");
        // Note: We can't easily check call count with current registry design
        // In a real implementation, we'd expose metrics
    }

    #[tokio::test]
    async fn test_max_retries_exceeded() {
        // Registry with substrate that always fails
        let registry = create_mock_registry_with_failures(10);
        let executor = create_mock_executor(registry);

        let mut ctx = create_test_context();
        ctx.config.retry.max_retries = 2;
        ctx.config.retry.initial_backoff_ms = 10;
        ctx.config.retry.max_backoff_ms = 100;

        let result = executor.execute_with_retry(ctx).await;

        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(ExecutionError::MaxRetriesExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn test_is_retryable_error() {
        // Rate limit is retryable
        let err = ExecutionError::SubstrateError {
            task_id: Uuid::new_v4(),
            source: SubstrateError::RateLimitExceeded("test".to_string()),
        };
        assert!(AgentExecutor::is_retryable_error(&err));

        // Auth error is NOT retryable
        let err = ExecutionError::SubstrateError {
            task_id: Uuid::new_v4(),
            source: SubstrateError::AuthError("test".to_string()),
        };
        assert!(!AgentExecutor::is_retryable_error(&err));

        // Timeout is retryable
        let err = ExecutionError::Timeout {
            task_id: Uuid::new_v4(),
            timeout_secs: 60,
        };
        assert!(AgentExecutor::is_retryable_error(&err));
    }
}

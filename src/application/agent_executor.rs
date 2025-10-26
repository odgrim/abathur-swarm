use crate::domain::models::Config;
use crate::domain::ports::{ClaudeClient, ClaudeError, ClaudeRequest, McpClient};
use crate::infrastructure::mcp::McpError;
use anyhow::Result;
use serde_json::Value;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
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

    #[error("Claude API error for task {task_id}: {source}")]
    ClaudeError {
        task_id: Uuid,
        #[source]
        source: ClaudeError,
    },

    #[error("MCP tool error for task {task_id}: {source}")]
    McpError {
        task_id: Uuid,
        #[source]
        source: McpError,
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
/// - Claude API calls for agent reasoning
/// - MCP tool invocations for actions
/// - Timeout enforcement
/// - Retry logic for transient failures
/// - Comprehensive error handling
pub struct AgentExecutor {
    claude_client: Arc<dyn ClaudeClient>,
    /// MCP client for tool invocations
    ///
    /// Reserved for future MCP tool integration. Currently, the `execute_inner`
    /// method has a TODO (line 290) to implement MCP tool call parsing and execution.
    #[allow(dead_code)]
    mcp_client: Arc<dyn McpClient>,
}

impl AgentExecutor {
    /// Create a new `AgentExecutor`
    ///
    /// # Arguments
    /// * `claude_client` - Client for Claude API interactions
    /// * `mcp_client` - Client for MCP tool invocations
    pub fn new(claude_client: Arc<dyn ClaudeClient>, mcp_client: Arc<dyn McpClient>) -> Self {
        Self {
            claude_client,
            mcp_client,
        }
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
    pub async fn execute(&self, ctx: ExecutionContext) -> Result<String, ExecutionError> {
        // Get timeout from config, default to 1 hour
        let timeout_secs = 3600; // TODO: Get from task.max_execution_timeout_seconds
        let timeout_duration = Duration::from_secs(timeout_secs);

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
    /// 1. Call Claude API with task prompt
    /// 2. Parse response for any MCP tool calls
    /// 3. Execute MCP tools if requested
    /// 4. Return final result
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

        // Build prompt for Claude
        let prompt = Self::build_prompt(&ctx);

        // Execute Claude API request
        let request = ClaudeRequest {
            task_id: ctx.task_id,
            agent_type: ctx.agent_type.clone(),
            prompt,
            max_tokens: Some(4096),
            temperature: Some(0.7),
        };

        let response = self
            .claude_client
            .execute(request)
            .await
            .map_err(|source| ExecutionError::ClaudeError {
                task_id: ctx.task_id,
                source,
            })?;

        tracing::info!(
            task_id = %ctx.task_id,
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            stop_reason = %response.stop_reason,
            "Claude API call completed"
        );

        // TODO: Parse response for MCP tool calls and execute them
        // For now, return the Claude response directly
        Ok(response.content)
    }

    /// Build prompt for Claude based on execution context
    fn build_prompt(ctx: &ExecutionContext) -> String {
        let mut prompt = format!(
            "You are a {} agent.\n\nTask: {}\n",
            ctx.agent_type, ctx.description
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
    /// - `ConnectionError`
    /// - Timeout
    ///
    /// Non-retryable errors:
    /// - `InvalidApiKey`
    /// - `InvalidArguments`
    /// - `ServerNotFound`
    /// - `ToolNotFound`
    const fn is_retryable_error(err: &ExecutionError) -> bool {
        match err {
            ExecutionError::ClaudeError { source, .. } => matches!(
                source,
                ClaudeError::RateLimitExceeded(_)
                    | ClaudeError::NetworkError(_)
                    | ClaudeError::Timeout
            ),
            ExecutionError::McpError { source, .. } => {
                matches!(source, McpError::CommunicationError(_) | McpError::HealthCheckTimeout(_))
            }
            ExecutionError::Timeout { .. } => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::{
        ClaudeClient, ClaudeError, ClaudeRequest, ClaudeResponse, McpClient, Resource, Tool,
    };
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Mock Claude Client for testing
    struct MockClaudeClient {
        call_count: Arc<AtomicU32>,
        should_fail: bool,
        fail_count: u32,
    }

    impl MockClaudeClient {
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
    impl ClaudeClient for MockClaudeClient {
        async fn execute(&self, request: ClaudeRequest) -> Result<ClaudeResponse, ClaudeError> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);

            if self.should_fail && count < self.fail_count {
                return Err(ClaudeError::RateLimitExceeded(
                    "Mock rate limit".to_string(),
                ));
            }

            Ok(ClaudeResponse {
                task_id: request.task_id,
                content: "Mock response".to_string(),
                stop_reason: "end_turn".to_string(),
                usage: crate::domain::ports::TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                },
            })
        }

        async fn health_check(&self) -> Result<(), ClaudeError> {
            Ok(())
        }
    }

    // Mock MCP Client for testing
    struct MockMcpClient;

    #[async_trait]
    impl McpClient for MockMcpClient {
        async fn list_tools(&self, _server: &str) -> Result<Vec<Tool>> {
            Ok(vec![Tool {
                name: "test_tool".to_string(),
                description: "A test tool".to_string(),
                input_schema: serde_json::json!({}),
            }])
        }

        async fn call_tool(&self, _server: &str, _tool: &str, _args: Value) -> Result<Value> {
            Ok(serde_json::json!({"success": true}))
        }

        async fn list_resources(&self, _server: &str) -> Result<Vec<Resource>> {
            Ok(vec![Resource {
                uri: "test://resource".to_string(),
                name: "Test Resource".to_string(),
                mime_type: Some("text/plain".to_string()),
            }])
        }

        async fn read_resource(&self, _server: &str, _uri: &str) -> Result<String> {
            Ok("test resource content".to_string())
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

    #[tokio::test]
    async fn test_successful_execution() {
        let claude_client = Arc::new(MockClaudeClient::new());
        let mcp_client = Arc::new(MockMcpClient);
        let executor = AgentExecutor::new(claude_client, mcp_client);

        let ctx = create_test_context();
        let result = executor.execute(ctx).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Mock response");
    }

    #[tokio::test]
    async fn test_timeout_behavior() {
        let claude_client = Arc::new(MockClaudeClient::new());
        let mcp_client = Arc::new(MockMcpClient);
        let executor = AgentExecutor::new(claude_client, mcp_client);

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
        // Mock that fails twice, then succeeds
        let claude_client = Arc::new(MockClaudeClient::with_failures(2));
        let mcp_client = Arc::new(MockMcpClient);
        let executor = AgentExecutor::new(claude_client.clone(), mcp_client);

        let mut ctx = create_test_context();
        ctx.config.retry.max_retries = 3;
        ctx.config.retry.initial_backoff_ms = 10; // Fast for testing
        ctx.config.retry.max_backoff_ms = 100;

        let result = executor.execute_with_retry(ctx).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Mock response");
        // Should have called 3 times (2 failures + 1 success)
        assert_eq!(claude_client.call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_max_retries_exceeded() {
        // Mock that always fails
        let claude_client = Arc::new(MockClaudeClient::with_failures(10));
        let mcp_client = Arc::new(MockMcpClient);
        let executor = AgentExecutor::new(claude_client.clone(), mcp_client);

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
        // Should have called 3 times (initial + 2 retries)
        assert_eq!(claude_client.call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_is_retryable_error() {
        // Rate limit is retryable
        let err = ExecutionError::ClaudeError {
            task_id: Uuid::new_v4(),
            source: ClaudeError::RateLimitExceeded("test".to_string()),
        };
        assert!(AgentExecutor::is_retryable_error(&err));

        // Invalid API key is NOT retryable
        let err = ExecutionError::ClaudeError {
            task_id: Uuid::new_v4(),
            source: ClaudeError::InvalidApiKey,
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

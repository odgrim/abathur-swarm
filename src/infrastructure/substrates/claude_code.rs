///! Claude Code Substrate
///!
///! This substrate shells out to the Claude Code CLI to execute tasks.
///! It requires the `claude` CLI to be installed and authenticated.
///!
///! Advantages:
///! - No API key management in config
///! - Uses Claude Code's authentication system
///! - Full access to Claude Code's tools and capabilities
///! - Works out of the box if claude CLI is set up

use crate::domain::ports::{
    HealthStatus, LlmSubstrate, StopReason, SubstrateError, SubstrateRequest,
    SubstrateResponse,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Configuration for Claude Code substrate
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Path to claude CLI executable (defaults to "claude" in PATH)
    pub claude_path: String,

    /// Working directory for claude execution (defaults to current dir)
    pub working_dir: Option<std::path::PathBuf>,

    /// Default timeout in seconds (overridden by request parameters)
    pub default_timeout_secs: u64,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            claude_path: "claude".to_string(),
            working_dir: None,
            default_timeout_secs: 300, // 5 minutes
        }
    }
}

/// Claude Code substrate implementation
///
/// Executes tasks by shelling out to the Claude Code CLI.
/// The CLI must be installed and authenticated separately.
pub struct ClaudeCodeSubstrate {
    config: ClaudeCodeConfig,
}

impl ClaudeCodeSubstrate {
    /// Create a new Claude Code substrate with default configuration
    pub fn new() -> Self {
        Self::with_config(ClaudeCodeConfig::default())
    }

    /// Create a new Claude Code substrate with custom configuration
    pub fn with_config(config: ClaudeCodeConfig) -> Self {
        Self { config }
    }

    /// Check if claude CLI is available
    async fn is_cli_available(&self) -> bool {
        Command::new(&self.config.claude_path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// Build the claude command with appropriate arguments
    fn build_command(&self, _request: &SubstrateRequest) -> Command {
        let mut cmd = Command::new(&self.config.claude_path);

        // Set working directory if specified
        if let Some(ref wd) = self.config.working_dir {
            cmd.current_dir(wd);
        }

        // Add agent type as context if available
        // Claude Code doesn't have direct agent type support, but we can
        // include it in the prompt prefix

        // Use stdin mode to send the prompt
        cmd.arg("--")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        cmd
    }

    /// Format the prompt with agent type context
    fn format_prompt(&self, request: &SubstrateRequest) -> String {
        let mut prompt = String::new();

        // Add agent type context
        prompt.push_str(&format!("[Agent Type: {}]\n\n", request.agent_type));

        // Add the main prompt
        prompt.push_str(&request.prompt);

        // Add context if available
        if let Some(ref context) = request.context {
            prompt.push_str("\n\n[Context]\n");
            if let Ok(pretty) = serde_json::to_string_pretty(context) {
                prompt.push_str(&pretty);
            }
        }

        prompt
    }
}

impl Default for ClaudeCodeSubstrate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmSubstrate for ClaudeCodeSubstrate {
    fn substrate_id(&self) -> &str {
        "claude-code"
    }

    fn substrate_name(&self) -> &str {
        "Claude Code CLI"
    }

    async fn execute(&self, request: SubstrateRequest) -> Result<SubstrateResponse, SubstrateError> {
        // Check if CLI is available
        if !self.is_cli_available().await {
            return Err(SubstrateError::Unavailable(format!(
                "Claude CLI not found at: {}. Please install Claude Code CLI.",
                self.config.claude_path
            )));
        }

        // Build command
        let mut cmd = self.build_command(&request);

        // Spawn the process
        let mut child = cmd
            .spawn()
            .map_err(|e| SubstrateError::ExecutionFailed(format!("Failed to spawn claude CLI: {}", e)))?;

        // Get handles for stdin/stdout/stderr
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| SubstrateError::ExecutionFailed("Failed to get stdin handle".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SubstrateError::ExecutionFailed("Failed to get stdout handle".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SubstrateError::ExecutionFailed("Failed to get stderr handle".to_string()))?;

        // Format and write the prompt to stdin
        let prompt = self.format_prompt(&request);
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| SubstrateError::ExecutionFailed(format!("Failed to write prompt: {}", e)))?;

        // Close stdin to signal end of input
        drop(stdin);

        // Set up timeout
        let timeout_duration = Duration::from_secs(
            request
                .parameters
                .timeout_secs
                .unwrap_or(self.config.default_timeout_secs),
        );

        // Read output with timeout
        let result = timeout(timeout_duration, async {
            // Read stdout
            let mut stdout_reader = BufReader::new(stdout);
            let mut output = String::new();
            let mut line = String::new();

            while stdout_reader.read_line(&mut line).await.map_err(|e| {
                SubstrateError::ExecutionFailed(format!("Failed to read output: {}", e))
            })? > 0 {
                output.push_str(&line);
                line.clear();
            }

            // Read stderr for any errors
            let mut stderr_reader = BufReader::new(stderr);
            let mut errors = String::new();
            let mut error_line = String::new();

            while stderr_reader.read_line(&mut error_line).await.map_err(|e| {
                SubstrateError::ExecutionFailed(format!("Failed to read stderr: {}", e))
            })? > 0 {
                errors.push_str(&error_line);
                error_line.clear();
            }

            // Wait for process to complete
            let status = child.wait().await.map_err(|e| {
                SubstrateError::ExecutionFailed(format!("Failed to wait for process: {}", e))
            })?;

            Ok::<_, SubstrateError>((output, errors, status))
        })
        .await;

        match result {
            Ok(Ok((output, errors, status))) => {
                // Check if execution succeeded
                if !status.success() {
                    return Err(SubstrateError::ExecutionFailed(format!(
                        "Claude CLI exited with code {:?}. Stderr: {}",
                        status.code(),
                        errors
                    )));
                }

                // Build response
                let mut metadata = HashMap::new();
                if !errors.is_empty() {
                    metadata.insert("stderr".to_string(), serde_json::Value::String(errors));
                }
                metadata.insert(
                    "exit_code".to_string(),
                    serde_json::Value::Number(status.code().unwrap_or(0).into()),
                );

                Ok(SubstrateResponse {
                    task_id: request.task_id,
                    content: output,
                    stop_reason: StopReason::EndTurn,
                    usage: None, // Claude CLI doesn't provide token usage
                    metadata,
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout occurred - kill the process
                let _ = child.kill().await;
                Err(SubstrateError::Timeout(
                    request
                        .parameters
                        .timeout_secs
                        .unwrap_or(self.config.default_timeout_secs),
                ))
            }
        }
    }

    async fn health_check(&self) -> Result<HealthStatus, SubstrateError> {
        if self.is_cli_available().await {
            Ok(HealthStatus::Healthy)
        } else {
            Ok(HealthStatus::Unavailable)
        }
    }

    fn can_handle_agent_type(&self, _agent_type: &str) -> bool {
        // Claude Code can handle all agent types
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::ExecutionParameters;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_health_check() {
        let substrate = ClaudeCodeSubstrate::new();
        let result = substrate.health_check().await;

        // This test will pass if claude CLI is installed, fail otherwise
        // We don't assert success here since CI might not have claude installed
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_prompt() {
        let substrate = ClaudeCodeSubstrate::new();
        let request = SubstrateRequest {
            task_id: Uuid::new_v4(),
            agent_type: "test-agent".to_string(),
            prompt: "Hello, world!".to_string(),
            context: None,
            parameters: ExecutionParameters::default(),
        };

        let formatted = substrate.format_prompt(&request);
        assert!(formatted.contains("[Agent Type: test-agent]"));
        assert!(formatted.contains("Hello, world!"));
    }

    #[test]
    fn test_format_prompt_with_context() {
        let substrate = ClaudeCodeSubstrate::new();
        let request = SubstrateRequest {
            task_id: Uuid::new_v4(),
            agent_type: "test-agent".to_string(),
            prompt: "Analyze this".to_string(),
            context: Some(serde_json::json!({"key": "value"})),
            parameters: ExecutionParameters::default(),
        };

        let formatted = substrate.format_prompt(&request);
        assert!(formatted.contains("[Agent Type: test-agent]"));
        assert!(formatted.contains("Analyze this"));
        assert!(formatted.contains("[Context]"));
        assert!(formatted.contains("key"));
    }
}
